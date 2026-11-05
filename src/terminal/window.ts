// Terminal tool window: tabs (new, close, rename by double click) over
// terminal views. A tab whose shell exited stays open with the exit code.
import {
  commands,
  type TerminalExited,
  type TerminalFrame,
  type TerminalInfo,
} from "../ipc/bindings";
import { errorMessage, unwrap } from "../ipc/result";
import { type TerminalHost, TerminalView } from "./view";

interface Tab {
  info: TerminalInfo;
  view: TerminalView;
  button: HTMLElement;
  label: HTMLElement;
  /** Last title the program set. */
  programTitle: string;
}

export class TerminalWindow {
  readonly element = document.createElement("section");
  private readonly tabBar = document.createElement("div");
  private readonly views = document.createElement("div");
  private readonly unavailable = document.createElement("div");
  private readonly tabs: Tab[] = [];
  private active: Tab | null = null;
  /** Frames that arrived before `open_terminal` returned. */
  private readonly early = new Map<number, TerminalFrame[]>();
  private opening: Promise<void> | null = null;
  /** Called when the window hides itself (last tab closed). */
  onHide: (() => void) | null = null;

  private readonly host: TerminalHost;

  constructor(host: Omit<TerminalHost, "newTab">) {
    this.host = {
      ...host,
      newTab: () => {
        void this.newTab();
      },
    };
    this.element.className = "tool-window terminal";
    this.element.dataset.testid = "terminal";
    this.element.setAttribute("aria-label", "Terminal");
    this.element.hidden = true;
    const header = document.createElement("div");
    header.className = "tool-window-header";
    const title = document.createElement("span");
    title.className = "tool-window-title";
    title.textContent = "Terminal";
    this.tabBar.className = "terminal-tabs";
    this.tabBar.setAttribute("role", "tablist");
    const add = document.createElement("button");
    add.type = "button";
    add.className = "terminal-new";
    add.dataset.testid = "terminal-new";
    add.textContent = "+";
    add.title = "New Terminal Tab";
    add.setAttribute("aria-label", "New terminal tab");
    // Keep the focus in the terminal (keys typed right after a click must
    // not land on the button).
    add.addEventListener("mousedown", (event) => {
      event.preventDefault();
    });
    add.addEventListener("click", () => {
      void this.newTab();
    });
    const close = document.createElement("button");
    close.type = "button";
    close.className = "tool-window-close";
    close.textContent = "×";
    close.setAttribute("aria-label", "Hide Terminal");
    close.addEventListener("click", () => {
      this.hide();
    });
    const start = document.createElement("div");
    start.className = "terminal-header-start";
    start.append(title, this.tabBar, add);
    header.append(start, close);
    this.views.className = "terminal-views";
    this.unavailable.className = "tool-window-empty";
    this.unavailable.dataset.testid = "terminal-unavailable";
    this.unavailable.hidden = true;
    this.element.append(header, this.unavailable, this.views);
  }

  get visible(): boolean {
    return !this.element.hidden;
  }

  hasFocus(): boolean {
    return this.element.contains(document.activeElement);
  }

  /** Shows the window, starting a terminal if none is open, and focuses it. */
  async show(): Promise<void> {
    this.element.hidden = false;
    if (this.tabs.length === 0) await this.newTab();
    this.active?.view.focus();
  }

  hide(): void {
    this.element.hidden = true;
    this.onHide?.();
  }

  /** Forgets every tab (the backend closed them with the old project). */
  reset(): void {
    for (const tab of this.tabs) tab.view.dispose();
    this.tabs.length = 0;
    this.active = null;
    this.early.clear();
    this.tabBar.replaceChildren();
    this.unavailable.hidden = true;
    this.element.hidden = true;
  }

  async newTab(): Promise<void> {
    // One at a time, so a double Alt+F12 does not start two shells.
    if (this.opening) return this.opening;
    this.opening = this.open().finally(() => {
      this.opening = null;
    });
    return this.opening;
  }

  private async open(): Promise<void> {
    // The view exists and has the focus before any await: typing starts
    // now and the view holds the input until the shell runs.
    const view = new TerminalView(this.host);
    view.element.hidden = false;
    this.views.append(view.element);
    for (const tab of this.tabs) tab.view.element.hidden = true;
    view.focus();
    const size = view.measure() ?? { cols: 80, rows: 24 };
    const list = await commands.terminals();
    if (list.status === "ok" && list.data.unavailable) {
      view.dispose();
      this.unavailable.textContent = list.data.unavailable;
      this.unavailable.hidden = false;
      return;
    }
    let info: TerminalInfo;
    try {
      info = unwrap(await commands.openTerminal(size));
    } catch (error: unknown) {
      view.dispose();
      if (this.active) this.active.view.element.hidden = false;
      // Shown in the tool window itself (e.g. a wrong terminal.shellPath).
      this.unavailable.textContent = `Cannot start a terminal: ${errorMessage(error)}`;
      this.unavailable.hidden = false;
      return;
    }
    this.unavailable.hidden = true;
    view.attach(info.id, size);
    const tab = this.addTab(info, view);
    for (const frame of this.early.get(info.id) ?? []) view.applyFrame(frame);
    this.early.delete(info.id);
    this.select(tab);
  }

  private addTab(info: TerminalInfo, view: TerminalView): Tab {
    const button = document.createElement("div");
    button.className = "terminal-tab";
    button.dataset.testid = "terminal-tab";
    button.dataset.id = String(info.id);
    button.setAttribute("role", "tab");
    const label = document.createElement("span");
    label.className = "terminal-tab-title";
    label.dataset.testid = "terminal-tab-title";
    label.textContent = info.title;
    const close = document.createElement("button");
    close.type = "button";
    close.className = "terminal-tab-close";
    close.dataset.testid = "terminal-tab-close";
    close.textContent = "×";
    close.setAttribute("aria-label", `Close ${info.title}`);
    button.append(label, close);
    const tab: Tab = { info, view, button, label, programTitle: "" };
    button.addEventListener("mousedown", (event) => {
      if (event.target === close) return;
      event.preventDefault();
      this.select(tab);
    });
    button.addEventListener("dblclick", () => {
      this.startRename(tab);
    });
    close.addEventListener("click", () => {
      void this.closeTab(tab);
    });
    this.tabs.push(tab);
    this.tabBar.append(button);
    return tab;
  }

  private select(tab: Tab): void {
    this.active = tab;
    for (const t of this.tabs) {
      const selected = t === tab;
      t.view.element.hidden = !selected;
      t.button.setAttribute("aria-selected", String(selected));
    }
    tab.view.focus();
  }

  private startRename(tab: Tab): void {
    const input = document.createElement("input");
    input.className = "terminal-tab-rename";
    input.dataset.testid = "terminal-tab-rename";
    input.value = tab.label.textContent;
    tab.label.replaceChildren(input);
    input.select();
    input.focus();
    let done = false;
    const finish = async (commit: boolean) => {
      if (done) return;
      done = true;
      if (commit) {
        try {
          tab.info = unwrap(await commands.renameTerminal(tab.info.id, input.value));
        } catch (error: unknown) {
          this.host.notify(`Rename failed: ${errorMessage(error)}`);
        }
      }
      tab.label.textContent = tab.info.title;
      tab.view.focus();
    };
    input.addEventListener("keydown", (event) => {
      event.stopPropagation();
      if (event.key === "Enter") void finish(true);
      if (event.key === "Escape") void finish(false);
    });
    input.addEventListener("blur", () => {
      void finish(true);
    });
  }

  private async closeTab(tab: Tab): Promise<void> {
    const result = await commands.closeTerminal(tab.info.id);
    if (result.status === "error" && result.error.kind !== "notFound") {
      this.host.notify(`Closing the terminal failed: ${result.error.message}`);
    }
    this.remove(tab);
  }

  private remove(tab: Tab): void {
    const index = this.tabs.indexOf(tab);
    if (index < 0) return;
    this.tabs.splice(index, 1);
    tab.view.dispose();
    tab.button.remove();
    const next = this.tabs[Math.min(index, this.tabs.length - 1)];
    if (next) {
      this.select(next);
    } else {
      this.active = null;
      this.hide();
    }
  }

  onFrame(frame: TerminalFrame): void {
    const tab = this.tabs.find((t) => t.info.id === frame.id);
    if (!tab) {
      const list = this.early.get(frame.id) ?? [];
      list.push(frame);
      this.early.set(frame.id, list);
      return;
    }
    tab.view.applyFrame(frame);
    // The tab shows the program's title unless the user renamed it; the
    // backend decides, so ask it when the program's title changes.
    if (frame.title !== tab.programTitle) {
      tab.programTitle = frame.title;
      void commands.terminals().then((list) => {
        if (list.status !== "ok") return;
        const info = list.data.terminals.find((t) => t.id === tab.info.id);
        if (info && info.title !== tab.info.title) {
          tab.info = info;
          if (!tab.label.querySelector("input")) tab.label.textContent = info.title;
        }
      });
    }
  }

  onExited(exited: TerminalExited): void {
    const tab = this.tabs.find((t) => t.info.id === exited.id);
    if (!tab) return;
    const code = exited.code ?? null;
    tab.view.showExit(code);
    tab.button.dataset.exited = "true";
    tab.button.title = code === null ? "Exited" : `Exited with code ${String(code)}`;
  }
}
