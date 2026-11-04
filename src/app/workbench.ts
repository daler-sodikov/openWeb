// Main window layout: toolbar, project tree, editor tabs, status bar.
import { open } from "@tauri-apps/plugin-dialog";

import { EDITOR_ACTIONS, EditorTab, type EditorTabHost } from "../editor/editor-tab";
import { DoubleTapDetector } from "../editor/gesture";
import {
  commands,
  events,
  type AppInfo,
  type FsChangeBatch,
  type IndexState,
  type KeyAction,
  type KeyBinding,
  type ReplaceResult,
  type SearchTab,
} from "../ipc/bindings";
import { errorMessage, unwrap } from "../ipc/result";
import { FindInFiles } from "../popup/find-in-files";
import type { Popup } from "../popup/popup";
import { SearchEverywhere } from "../popup/search-everywhere";
import { GotoLine, RecentFiles } from "../popup/simple-popups";
import { TreeView } from "../tree/tree-view";
import { chordMatches, formatChord } from "../ui/keys";

function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  className: string,
  testid?: string,
): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  node.className = className;
  if (testid) node.dataset.testid = testid;
  return node;
}

export class Workbench implements EditorTabHost {
  private readonly tabs: EditorTab[] = [];
  private active: EditorTab | null = null;
  private readonly tree: TreeView;
  private readonly tabBar = el("div", "tabs", "tabs");
  private readonly editors = el("div", "editors");
  private readonly welcome = el("div", "welcome", "welcome");
  private readonly projectName = el("span", "toolbar-project", "project-name");
  private readonly statusPath = el("span", "status-path", "status-path");
  private readonly statusNotice = el("span", "status-notice", "status-notice");
  private readonly statusPosition = el("span", "status-position", "status-position");
  private readonly statusLanguage = el("span", "status-language", "status-language");
  private readonly statusState = el("span", "status-state", "status-state");
  private readonly statusIndex = el("span", "status-index", "status-index");
  private readonly opening = new Map<string, Promise<void>>();
  private readonly mac: boolean;
  private readonly doubleShift = new DoubleTapDetector("Shift");
  private popup: Popup | null = null;
  private hasProject = false;

  constructor(
    root: HTMLElement,
    private readonly info: AppInfo,
    private readonly keymap: readonly KeyBinding[],
  ) {
    this.mac = info.platform === "macos";
    const toolbar = el("header", "toolbar");
    const brand = el("span", "toolbar-brand", "app-name");
    brand.textContent = info.name;
    const openButton = el("button", "toolbar-button", "open-folder");
    openButton.type = "button";
    openButton.textContent = "Open Folder…";
    openButton.addEventListener("click", () => {
      void this.pickFolder();
    });
    toolbar.append(brand, openButton, this.projectName);

    const sidebar = el("aside", "sidebar");
    const sidebarTitle = el("div", "panel-title");
    sidebarTitle.textContent = "Project";
    const treeElement = el("div", "tree", "project-tree");
    treeElement.setAttribute("aria-label", "Project files");
    sidebar.append(sidebarTitle, treeElement);
    this.tree = new TreeView(treeElement, {
      listDir: async (path) => unwrap(await commands.listDir(path)),
      openFile: (path) => {
        void this.openFile(path);
      },
      onError: (message) => {
        this.notify(message);
      },
    });

    this.tabBar.setAttribute("role", "tablist");
    this.renderWelcome();
    const main = el("section", "main");
    main.append(this.tabBar, this.editors, this.welcome);

    const body = el("div", "body");
    body.append(sidebar, main);

    const status = el("footer", "status", "status-bar");
    status.append(
      this.statusPath,
      this.statusNotice,
      this.statusPosition,
      this.statusIndex,
      this.statusLanguage,
      this.statusState,
    );

    const shell = el("div", "workbench");
    shell.append(toolbar, body, status);
    root.replaceChildren(shell);
    this.updateChrome();
    this.installGlobalKeys();
    void this.listenToBackend();
  }

  // Global shortcuts, actions and popups

  private installGlobalKeys(): void {
    // Capture phase: works whether the editor, the tree or a popup has focus.
    window.addEventListener(
      "keydown",
      (event) => {
        this.doubleShift.keydown(event.key);
        for (const binding of this.keymap) {
          if (EDITOR_ACTIONS.has(binding.action) || binding.stroke.type !== "chord") continue;
          if (chordMatches(event, binding.stroke.keys, this.mac)) {
            event.preventDefault();
            event.stopPropagation();
            this.runAction(binding.action);
            return;
          }
        }
      },
      true,
    );
    window.addEventListener(
      "keyup",
      (event) => {
        if (this.doubleShift.keyup(event.key, event.timeStamp)) this.runAction("searchEverywhere");
      },
      true,
    );
  }

  private async listenToBackend(): Promise<void> {
    try {
      await events.fsChanged.listen((event) => {
        void this.onFsChanged(event.payload);
      });
      await events.indexStatus.listen((event) => {
        this.onIndexStatus(event.payload);
      });
    } catch (error: unknown) {
      this.notify(`Backend events unavailable: ${errorMessage(error)}`);
    }
  }

  /** Runs an action from a shortcut or Search Everywhere. */
  runAction(action: KeyAction): void {
    const tab = this.active;
    switch (action) {
      case "searchEverywhere":
        this.openSearch("all");
        return;
      case "gotoFile":
        this.openSearch("files");
        return;
      case "gotoSymbol":
        this.openSearch("symbols");
        return;
      case "gotoAction":
        this.openSearch("actions");
        return;
      case "recentFiles":
        if (this.requireProject()) {
          this.showPopup(
            new RecentFiles((path) => {
              void this.openFile(path);
            }),
          );
        }
        return;
      case "gotoLine":
        if (tab) {
          const state = tab.view.state;
          const line = state.doc.lineAt(state.selection.main.head);
          const current = `${String(line.number)}:${String(state.selection.main.head - line.from + 1)}`;
          this.showPopup(
            new GotoLine(current, (l, c) => {
              tab.reveal(l, c);
            }),
          );
        }
        return;
      case "findInFiles":
      case "replaceInFiles":
        if (this.requireProject()) {
          this.showPopup(new FindInFiles(this, action === "replaceInFiles", this.selectedText()));
        }
        return;
      case "openFolder":
        void this.pickFolder();
        return;
      case "closeTab":
        if (tab) this.closeTab(tab);
        return;
      case "save":
        void tab?.save();
        return;
      case "undo":
        void tab?.undo();
        return;
      case "redo":
        void tab?.redo();
        return;
      case "addCaretAbove":
        void tab?.addCaret("up");
        return;
      case "addCaretBelow":
        void tab?.addCaret("down");
        return;
      case "selectNextOccurrence":
        void tab?.selectNextOccurrence();
        return;
      case "selectAllOccurrences":
        void tab?.selectAllOccurrences();
        return;
    }
  }

  /** Shortcut text for an action (shown in Search Everywhere). */
  shortcutFor(action: KeyAction): string {
    const binding = this.keymap.find((b) => b.action === action);
    if (!binding) return "";
    switch (binding.stroke.type) {
      case "chord":
        return formatChord(binding.stroke.keys, this.mac);
      case "doubleTap":
        return "Double Shift";
      case "doubleModifier":
        return `Double ${binding.stroke.modifier === "control" ? "Ctrl" : this.mac ? "⌥" : "Alt"} + ${binding.stroke.key.replace("Arrow", "")}`;
    }
  }

  private requireProject(): boolean {
    if (!this.hasProject) this.notify("Open a folder first.");
    return this.hasProject;
  }

  private openSearch(tab: SearchTab): void {
    if (this.popup instanceof SearchEverywhere && !this.popup.isClosed) {
      this.popup.setTab(tab);
      return;
    }
    this.showPopup(new SearchEverywhere(this, tab, this.selectedText()));
  }

  private showPopup(popup: Popup): void {
    this.popup?.close();
    this.popup = popup;
    popup.onClose = () => {
      if (this.popup === popup) this.popup = null;
      if (!document.querySelector(".popup")) this.active?.view.focus();
    };
    popup.open();
  }

  /** Single-line selection of the active editor, to prefill searches. */
  private selectedText(): string {
    const state = this.active?.view.state;
    if (!state) return "";
    const { from, to } = state.selection.main;
    const text = state.sliceDoc(from, to);
    return text.includes("\n") ? "" : text;
  }

  /** Opens a file and, if given, moves the caret to a zero-based line and column. */
  openLocation(path: string, line?: number, column?: number): void {
    void (async () => {
      await this.openFile(path);
      const tab = this.tabs.find((t) => t.path === path);
      if (tab && line !== undefined) tab.reveal(line, column ?? 0);
    })();
  }

  async flushAll(): Promise<void> {
    await Promise.all(this.tabs.map((t) => t.flush()));
  }

  async applyReplace(result: ReplaceResult): Promise<string[]> {
    const failures: string[] = [];
    for (const doc of result.openDocuments) {
      const tab = this.tabs.find((t) => t.docId === doc.docId);
      if (!tab) {
        failures.push(`document ${String(doc.docId)}`);
        continue;
      }
      try {
        await tab.applyReplace(doc.version, doc.edits);
      } catch (error: unknown) {
        failures.push(`${tab.name} (${errorMessage(error)})`);
      }
    }
    this.updateChrome();
    return failures;
  }

  // Backend events

  private async onFsChanged(batch: FsChangeBatch): Promise<void> {
    const removed = batch.changes.filter((c) => c.kind === "removed").map((c) => c.path);
    await this.tree.refreshPaths(
      batch.changes.map((c) => c.path),
      removed,
    );
    for (const change of batch.changes) {
      const tab = this.tabs.find((t) => t.path === change.path);
      if (!tab) continue;
      if (change.kind === "removed") {
        this.notify(`${tab.name} was deleted on disk; saving recreates it.`);
      } else {
        void tab.onDiskChange();
      }
    }
  }

  private onIndexStatus(state: IndexState): void {
    this.statusIndex.textContent = state.indexing ? "Indexing…" : "";
    this.statusIndex.title = state.indexing
      ? ""
      : `${String(state.files)} files, ${String(state.symbols)} symbols indexed`;
  }

  async pickFolder(): Promise<void> {
    try {
      const folder = await open({ directory: true, multiple: false, title: "Open Folder" });
      if (typeof folder === "string") await this.openProject(folder);
    } catch (error: unknown) {
      this.notify(`Cannot open the folder dialog: ${errorMessage(error)}`);
    }
  }

  async openProject(path: string): Promise<void> {
    try {
      const project = unwrap(await commands.openProject(path));
      for (const tab of [...this.tabs]) this.removeTab(tab);
      this.projectName.textContent = project.name;
      this.projectName.title = project.root;
      document.title = `${this.info.name} — ${project.name}`;
      this.hasProject = true;
      this.tree.setRoot(unwrap(await commands.listDir("")));
      this.notify("");
      this.renderWelcome(project.name);
    } catch (error: unknown) {
      this.notify(`Cannot open ${path}: ${errorMessage(error)}`);
    }
  }

  async openFile(path: string): Promise<void> {
    const existing = this.tabs.find((t) => t.path === path);
    if (existing) {
      this.activate(existing);
      return;
    }
    const pending = this.opening.get(path);
    if (pending) return pending;
    const task = (async () => {
      try {
        const doc = unwrap(await commands.openDocument(path));
        const tab = new EditorTab(doc, this.keymap, this);
        this.tabs.push(tab);
        this.editors.append(tab.container);
        this.activate(tab);
        this.notify(doc.restored ? `Restored unsaved changes in ${tab.name}` : "");
      } catch (error: unknown) {
        this.notify(`Cannot open ${path}: ${errorMessage(error)}`);
      } finally {
        this.opening.delete(path);
      }
    })();
    this.opening.set(path, task);
    return task;
  }

  // EditorTabHost

  onStatus(): void {
    this.updateChrome();
  }

  onCursor(tab: EditorTab): void {
    if (tab === this.active) this.updatePosition();
  }

  onError(tab: EditorTab, message: string): void {
    this.notify(`${tab.name}: ${message}`);
  }

  closeTab(tab: EditorTab): void {
    void (async () => {
      // Let queued edits reach Rust before closing (they are kept in the recovery file).
      await tab.idle();
      try {
        unwrap(await commands.closeDocument(tab.docId));
      } catch (error: unknown) {
        this.notify(errorMessage(error));
      }
      this.removeTab(tab);
    })();
  }

  private removeTab(tab: EditorTab): void {
    const index = this.tabs.indexOf(tab);
    if (index < 0) return;
    this.tabs.splice(index, 1);
    tab.destroy();
    if (this.active === tab) {
      this.active = null;
      const next = this.tabs[Math.min(index, this.tabs.length - 1)];
      if (next) this.activate(next);
    }
    this.updateChrome();
  }

  private activate(tab: EditorTab): void {
    this.active = tab;
    for (const t of this.tabs) t.container.hidden = t !== tab;
    this.updateChrome();
    tab.view.requestMeasure();
    tab.view.focus();
  }

  private updateChrome(): void {
    const items = this.tabs.map((tab) => {
      const item = el("div", "tab", "tab");
      item.setAttribute("role", "tab");
      item.setAttribute("aria-selected", String(tab === this.active));
      item.dataset.path = tab.path;
      item.title = tab.path;
      if (tab === this.active) item.classList.add("active");
      if (tab.dirty) item.classList.add("dirty");
      const label = el("span", "tab-label", "tab-label");
      label.textContent = tab.name;
      const marker = el("span", "tab-dirty", "tab-dirty");
      marker.textContent = tab.dirty ? "●" : "";
      marker.setAttribute("aria-label", tab.dirty ? "modified" : "");
      const close = el("button", "tab-close", "tab-close");
      close.type = "button";
      close.textContent = "×";
      close.setAttribute("aria-label", `Close ${tab.name}`);
      close.addEventListener("click", (event) => {
        event.stopPropagation();
        this.closeTab(tab);
      });
      item.addEventListener("mousedown", (event) => {
        if (event.button === 1) {
          event.preventDefault();
          this.closeTab(tab);
        }
      });
      item.addEventListener("click", () => {
        this.activate(tab);
      });
      item.append(label, marker, close);
      return item;
    });
    this.tabBar.replaceChildren(...items);

    const tab = this.active;
    this.welcome.hidden = tab !== null;
    this.statusPath.textContent = tab?.path ?? "";
    this.statusLanguage.textContent = tab ? (tab.language ?? "Plain text") : "";
    this.statusState.textContent = tab ? (tab.dirty ? "Modified" : "Saved") : "";
    this.updatePosition();
  }

  private updatePosition(): void {
    const tab = this.active;
    if (!tab) {
      this.statusPosition.textContent = "";
      return;
    }
    const state = tab.view.state;
    const head = state.selection.main.head;
    const line = state.doc.lineAt(head);
    const carets = state.selection.ranges.length;
    this.statusPosition.textContent =
      `${String(line.number)}:${String(head - line.from + 1)}` +
      (carets > 1 ? ` (${String(carets)} carets)` : "");
  }

  private renderWelcome(projectName?: string): void {
    const title = el("p", "welcome-title");
    const hint = el("p", "welcome-hint");
    if (projectName) {
      title.textContent = projectName;
      hint.textContent = "Select a file in the project tree to open it.";
    } else {
      title.textContent = "No project open";
      hint.textContent = "Use Open Folder… or start the app with a folder path.";
    }
    this.welcome.replaceChildren(title, hint);
  }

  private notify(message: string): void {
    this.statusNotice.textContent = message;
  }
}
