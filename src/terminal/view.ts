// One terminal: draws frames from the backend, forwards keys, pastes,
// resizes, scrolling and mouse selection, and opens file links on
// Ctrl/Cmd+click. The screen itself (parsing, scrollback, selection) is in
// Rust; this only renders and routes events.
import {
  commands,
  type NavTarget,
  type TerminalCell,
  type TerminalFrame,
  type TerminalSelectionKind,
  type TerminalSize,
} from "../ipc/bindings";
import { errorMessage, type IpcResult, unwrap } from "../ipc/result";
import { type InputItem, InputQueue, routeKey } from "./input";
import { runStyle, ScreenModel } from "./screen";

export interface TerminalHost {
  readonly mac: boolean;
  notify(message: string): void;
  openLink(target: NavTarget): void;
}

const SELECTION_KINDS: Record<number, TerminalSelectionKind> = {
  1: "simple",
  2: "word",
  3: "line",
};

export class TerminalView {
  readonly element = document.createElement("div");
  private readonly screen = document.createElement("div");
  private readonly cursorEl = document.createElement("div");
  private readonly input = document.createElement("textarea");
  private readonly measureEl = document.createElement("span");
  private readonly model = new ScreenModel();
  private rowEls: HTMLElement[] = [];
  private dirty = new Set<number>();
  private pendingAck: number | null = null;
  private frameRequested = false;
  private cell = { width: 0, height: 0 };
  private size: TerminalSize | null = null;
  private resizeTimer: number | undefined;
  private readonly observer: ResizeObserver;
  private wheelRemainder = 0;
  private composing = false;
  private readonly queue = new InputQueue((item) => this.sendInput(item));
  /** Removes the window listeners. */
  private readonly abort = new AbortController();
  private drag: { anchor: TerminalCell; kind: TerminalSelectionKind; moved: boolean } | null = null;
  id = 0;
  /** Resolves once the backend has started the terminal; input waits for it. */
  private readonly attached: Promise<void>;
  private markAttached: () => void = () => undefined;

  constructor(private readonly host: TerminalHost) {
    this.element.className = "terminal-view";
    this.element.dataset.testid = "terminal-view";
    this.screen.className = "terminal-screen";
    this.cursorEl.className = "terminal-cursor";
    this.cursorEl.hidden = true;
    this.input.className = "terminal-input";
    this.input.dataset.testid = "terminal-input";
    this.input.setAttribute("aria-label", "Terminal input");
    this.input.autocapitalize = "off";
    this.input.spellcheck = false;
    this.measureEl.className = "terminal-measure";
    this.measureEl.textContent = "W".repeat(10);
    this.element.append(this.screen, this.cursorEl, this.input, this.measureEl);
    this.observer = new ResizeObserver(() => {
      this.scheduleFit();
    });
    this.observer.observe(this.element);
    this.attached = new Promise((resolve) => {
      this.markAttached = resolve;
    });
    this.bindEvents();
  }

  /** Grid size that fits the element, or null while it is hidden. */
  measure(): TerminalSize | null {
    const box = this.measureEl.getBoundingClientRect();
    if (box.width === 0 || this.element.clientWidth === 0) return null;
    this.cell = { width: box.width / 10, height: box.height };
    this.element.style.setProperty("--term-cell-w", `${String(this.cell.width)}px`);
    return {
      cols: Math.max(1, Math.floor(this.element.clientWidth / this.cell.width)),
      rows: Math.max(1, Math.floor(this.element.clientHeight / this.cell.height)),
    };
  }

  /** Called once the backend has started the terminal at `size`. */
  attach(id: number, size: TerminalSize): void {
    this.id = id;
    this.size = size;
    this.element.dataset.id = String(id);
    this.markAttached();
  }

  focus(): void {
    this.input.focus({ preventScroll: true });
  }

  hasFocus(): boolean {
    return this.element.contains(document.activeElement);
  }

  dispose(): void {
    this.observer.disconnect();
    this.abort.abort();
    window.clearTimeout(this.resizeTimer);
    this.element.remove();
  }

  get title(): string {
    return this.model.title;
  }

  /** Text of a visible row (tests and accessibility). */
  rowText(row: number): string {
    return this.model.rowText(row);
  }

  applyFrame(frame: TerminalFrame): void {
    for (const row of this.model.apply(frame)) this.dirty.add(row);
    this.pendingAck = frame.seq;
    if (!this.frameRequested) {
      this.frameRequested = true;
      requestAnimationFrame(() => {
        this.render();
      });
    }
  }

  private render(): void {
    this.frameRequested = false;
    const rows = this.model.rows.length;
    if (this.rowEls.length !== rows) {
      this.rowEls = Array.from({ length: rows }, (_, i) => {
        const row = document.createElement("div");
        row.className = "terminal-row";
        row.dataset.testid = "terminal-row";
        row.dataset.row = String(i);
        return row;
      });
      this.screen.replaceChildren(...this.rowEls);
      for (let i = 0; i < rows; i++) this.dirty.add(i);
    }
    for (const index of this.dirty) {
      const rowEl = this.rowEls[index];
      if (!rowEl) continue;
      rowEl.replaceChildren(
        ...(this.model.rows[index] ?? []).map((run) => {
          const span = document.createElement("span");
          span.textContent = run.text;
          const style = runStyle(run);
          if (style.color) span.style.color = style.color;
          if (style.background) span.style.backgroundColor = style.background;
          if (style.classes.length > 0) span.className = style.classes.join(" ");
          return span;
        }),
      );
    }
    this.dirty.clear();
    const cursor = this.model.cursor;
    this.cursorEl.hidden = cursor === null;
    if (cursor) {
      const left = `${String(cursor.col * this.cell.width)}px`;
      const top = `${String(cursor.row * this.cell.height)}px`;
      this.cursorEl.style.transform = `translate(${left}, ${top})`;
      // Keep the IME window next to the cursor.
      this.input.style.transform = `translate(${left}, ${top})`;
    }
    this.element.dataset.cols = String(this.model.cols);
    this.element.dataset.rows = String(rows);
    this.element.dataset.scrolled = String(this.model.displayOffset);
    if (this.pendingAck !== null) {
      const seq = this.pendingAck;
      this.pendingAck = null;
      void this.call(commands.terminalAck(this.id, seq));
    }
  }

  private scheduleFit(): void {
    window.clearTimeout(this.resizeTimer);
    this.resizeTimer = window.setTimeout(() => {
      const size = this.measure();
      if (!size || this.id === 0) return;
      if (size.cols === this.size?.cols && size.rows === this.size.rows) return;
      this.size = size;
      void this.call(commands.terminalResize(this.id, size));
    }, 50);
  }

  private async call<T>(result: Promise<IpcResult<T>>): Promise<T | null> {
    try {
      return unwrap(await result);
    } catch (error: unknown) {
      this.host.notify(`Terminal: ${errorMessage(error)}`);
      return null;
    }
  }

  private async sendInput(item: InputItem): Promise<void> {
    await this.attached;
    switch (item.kind) {
      case "text":
        await this.call(commands.terminalInput(this.id, item.text));
        return;
      case "paste":
        await this.call(commands.terminalPaste(this.id, item.text));
        return;
      case "key":
        await this.call(commands.terminalKey(this.id, item.key));
        return;
    }
  }

  private cellAt(event: MouseEvent): TerminalCell {
    const box = this.screen.getBoundingClientRect();
    const col = Math.floor((event.clientX - box.left) / (this.cell.width || 1));
    const row = Math.floor((event.clientY - box.top) / (this.cell.height || 1));
    const rows = this.model.rows.length;
    return {
      row: Math.min(Math.max(row, 0), Math.max(rows - 1, 0)),
      col: Math.min(Math.max(col, 0), Math.max(this.model.cols - 1, 0)),
    };
  }

  private async copy(): Promise<void> {
    const text = await this.call(commands.terminalSelectionText(this.id));
    if (!text) return;
    try {
      await navigator.clipboard.writeText(text);
    } catch (error: unknown) {
      this.host.notify(`Copy failed: ${errorMessage(error)}`);
    }
  }

  private async openLinkAt(cell: TerminalCell): Promise<void> {
    const target = await this.call(commands.terminalLinkAt(this.id, cell));
    if (target) this.host.openLink(target);
  }

  private bindEvents(): void {
    this.input.addEventListener("keydown", (event) => {
      const route = routeKey(event, this.host.mac);
      switch (route.kind) {
        case "ignore":
        case "text":
          return;
        case "paste":
          // The browser fires a paste event on the textarea.
          return;
        case "copy":
          event.preventDefault();
          void this.copy();
          return;
        case "key":
          event.preventDefault();
          this.queue.push({ kind: "key", key: route.key });
          return;
      }
    });
    this.input.addEventListener("input", () => {
      if (this.composing) return;
      const text = this.input.value;
      this.input.value = "";
      if (text) this.queue.push({ kind: "text", text });
    });
    this.input.addEventListener("compositionstart", () => {
      this.composing = true;
    });
    this.input.addEventListener("compositionend", (event) => {
      this.composing = false;
      this.input.value = "";
      if (event.data) this.queue.push({ kind: "text", text: event.data });
    });
    this.input.addEventListener("paste", (event) => {
      event.preventDefault();
      const text = event.clipboardData?.getData("text/plain") ?? "";
      if (text) this.queue.push({ kind: "paste", text });
    });
    this.element.addEventListener("mousedown", (event) => {
      if (event.button !== 0) return;
      event.preventDefault();
      this.focus();
      const cell = this.cellAt(event);
      if (event.ctrlKey || event.metaKey) {
        void this.openLinkAt(cell);
        return;
      }
      const kind = SELECTION_KINDS[Math.min(event.detail, 3)] ?? "simple";
      this.drag = { anchor: cell, kind, moved: kind !== "simple" };
      if (kind !== "simple") {
        void this.call(commands.terminalSelect(this.id, cell, cell, kind));
      }
    });
    window.addEventListener(
      "mousemove",
      (event) => {
        const drag = this.drag;
        if (!drag || (event.buttons & 1) === 0) return;
        const head = this.cellAt(event);
        if (!drag.moved && head.row === drag.anchor.row && head.col === drag.anchor.col) return;
        drag.moved = true;
        void this.call(commands.terminalSelect(this.id, drag.anchor, head, drag.kind));
      },
      { signal: this.abort.signal },
    );
    window.addEventListener(
      "mouseup",
      () => {
        const drag = this.drag;
        this.drag = null;
        if (drag && !drag.moved) void this.call(commands.terminalClearSelection(this.id));
      },
      { signal: this.abort.signal },
    );
    this.element.addEventListener(
      "wheel",
      (event) => {
        event.preventDefault();
        const lines =
          event.deltaMode === WheelEvent.DOM_DELTA_LINE
            ? event.deltaY
            : (event.deltaY + this.wheelRemainder) / (this.cell.height || 16);
        const whole = Math.trunc(lines);
        this.wheelRemainder =
          event.deltaMode === WheelEvent.DOM_DELTA_LINE
            ? 0
            : (lines - whole) * (this.cell.height || 16);
        // Wheel up (negative delta) goes back into the scrollback.
        if (whole !== 0) void this.call(commands.terminalScroll(this.id, -whole));
      },
      { passive: false },
    );
  }
}
