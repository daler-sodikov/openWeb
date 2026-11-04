// Find in Files (Ctrl+Shift+F) and Replace in Files (Ctrl+Shift+R).
import { commands, type FindMatch, type ReplaceResult, type TextSearch } from "../ipc/bindings";
import { BackendError, errorMessage, unwrap } from "../ipc/result";
import { VirtualList } from "../ui/virtual-list";
import { bindListKeys, input, Popup } from "./popup";

const DEBOUNCE_MS = 150;

export interface FindHost {
  openLocation(path: string, line?: number, column?: number): void;
  /** Sends pending edits of every open tab to Rust. */
  flushAll(): Promise<void>;
  /** Applies replacement edits to open tabs; returns failures. */
  applyReplace(result: ReplaceResult): Promise<string[]>;
}

type Toggle = "regex" | "caseSensitive" | "wholeWord";

export class FindInFiles extends Popup {
  private readonly findField = input("popup-input", "find-input", "Text to find");
  private readonly replaceField = input("popup-input", "find-replace-input", "Replace with");
  private readonly status = document.createElement("div");
  private readonly list: VirtualList<FindMatch>;
  private readonly toggles: Record<Toggle, boolean> = {
    regex: false,
    caseSensitive: false,
    wholeWord: false,
  };
  private seq = 0;
  private timer: ReturnType<typeof setTimeout> | undefined;

  constructor(
    private readonly host: FindHost,
    private readonly replaceMode: boolean,
    initial: string,
  ) {
    super("find-in-files", "find-in-files", replaceMode ? "Replace in Files" : "Find in Files");
    const title = document.createElement("div");
    title.className = "popup-title";
    title.textContent = replaceMode ? "Replace in Files" : "Find in Files";

    const findRow = document.createElement("div");
    findRow.className = "find-row";
    findRow.append(
      this.findField,
      this.toggle("caseSensitive", "Cc", "Match case", "find-case"),
      this.toggle("wholeWord", "W", "Words", "find-word"),
      this.toggle("regex", ".*", "Regex", "find-regex"),
    );
    this.element.append(title, findRow);

    if (replaceMode) {
      const replaceRow = document.createElement("div");
      replaceRow.className = "find-row";
      const replaceAll = document.createElement("button");
      replaceAll.type = "button";
      replaceAll.className = "toolbar-button";
      replaceAll.textContent = "Replace All";
      replaceAll.dataset.testid = "find-replace-all";
      replaceAll.addEventListener("click", () => {
        void this.replaceAll();
      });
      replaceRow.append(this.replaceField, replaceAll);
      this.element.append(replaceRow);
      this.replaceField.addEventListener("input", () => {
        this.schedule();
      });
      bindListKeys(this.replaceField, {
        move: (d) => {
          this.list.move(d);
        },
        activateSelected: () => undefined,
      });
    }

    this.list = new VirtualList<FindMatch>(
      24,
      (m) => this.renderMatch(m),
      (m) => {
        this.close();
        this.host.openLocation(m.path, m.line, m.columnStart);
      },
      "find-results",
    );
    this.status.className = "popup-footer";
    this.status.dataset.testid = "find-status";
    this.element.append(this.list.element, this.status);
    this.findField.value = initial;
    bindListKeys(this.findField, this.list);
    this.findField.addEventListener("input", () => {
      this.schedule();
    });
  }

  override open(): void {
    super.open();
    this.findField.focus();
    this.findField.select();
    if (this.findField.value) this.schedule();
  }

  override close(): void {
    clearTimeout(this.timer);
    super.close();
  }

  private toggle(name: Toggle, label: string, title: string, testid: string): HTMLButtonElement {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "find-toggle";
    button.textContent = label;
    button.title = title;
    button.dataset.testid = testid;
    button.setAttribute("aria-pressed", "false");
    button.addEventListener("mousedown", (event) => {
      event.preventDefault();
      this.toggles[name] = !this.toggles[name];
      button.setAttribute("aria-pressed", String(this.toggles[name]));
      this.schedule();
    });
    return button;
  }

  private query(): TextSearch {
    return {
      pattern: this.findField.value,
      regex: this.toggles.regex,
      caseSensitive: this.toggles.caseSensitive,
      wholeWord: this.toggles.wholeWord,
      replacement: this.replaceMode ? this.replaceField.value : null,
    };
  }

  private schedule(): void {
    clearTimeout(this.timer);
    this.timer = setTimeout(() => {
      void this.search();
    }, DEBOUNCE_MS);
  }

  private async search(): Promise<void> {
    const seq = ++this.seq;
    if (!this.findField.value) {
      this.list.setItems([]);
      this.status.textContent = "";
      return;
    }
    try {
      await this.host.flushAll();
      const result = unwrap(await commands.findInFiles(this.query()));
      if (seq !== this.seq || this.isClosed) return;
      this.list.setItems(result.matches);
      const files = new Set(result.matches.map((m) => m.path)).size;
      this.status.textContent = result.indexing
        ? "Indexing…"
        : `${String(result.matches.length)}${result.truncated ? "+" : ""} matches in ${String(files)} files`;
    } catch (error: unknown) {
      if (seq !== this.seq) return;
      if (error instanceof BackendError && error.ipc.kind === "cancelled") return;
      this.list.setItems([]);
      this.status.textContent = errorMessage(error);
    }
  }

  private async replaceAll(): Promise<void> {
    if (!this.findField.value) return;
    try {
      await this.host.flushAll();
      const result = unwrap(await commands.replaceInFiles(this.query(), this.replaceField.value));
      const failures = await this.host.applyReplace(result);
      const files = result.filesWritten + result.openDocuments.length;
      this.status.textContent =
        `Replaced ${String(result.replacements)} occurrences in ${String(files)} files` +
        (failures.length > 0 ? `; not applied: ${failures.join(", ")}` : "");
      this.list.setItems([]);
    } catch (error: unknown) {
      this.status.textContent = errorMessage(error);
    }
  }

  private renderMatch(m: FindMatch): HTMLElement {
    const row = document.createElement("div");
    row.className = "find-match";
    row.dataset.testid = "find-match";
    row.dataset.path = m.path;
    const location = document.createElement("span");
    location.className = "find-location";
    location.textContent = `${m.path}:${String(m.line + 1)}`;
    const text = document.createElement("span");
    text.className = "find-text";
    // Columns are UTF-16 offsets, the same unit as JS string indices.
    const before = m.lineText.slice(0, m.columnStart).trimStart();
    const matched = m.lineText.slice(m.columnStart, m.columnEnd);
    const after = m.lineText.slice(m.columnEnd);
    const replacement = m.replacement ?? null;
    const hit = document.createElement(replacement === null ? "mark" : "del");
    hit.textContent = matched;
    text.append(before, hit);
    if (replacement !== null) {
      const ins = document.createElement("ins");
      ins.textContent = replacement;
      text.append(ins);
    }
    text.append(after);
    row.append(text, location);
    return row;
  }
}
