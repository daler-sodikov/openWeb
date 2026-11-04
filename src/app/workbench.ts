// Main window layout: toolbar, project tree, editor tabs, status bar.
import { open } from "@tauri-apps/plugin-dialog";

import { EditorTab, type EditorTabHost } from "../editor/editor-tab";
import { commands, type AppInfo, type KeyBinding } from "../ipc/bindings";
import { errorMessage, unwrap } from "../ipc/result";
import { TreeView } from "../tree/tree-view";

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
  private readonly opening = new Map<string, Promise<void>>();

  constructor(
    root: HTMLElement,
    private readonly info: AppInfo,
    private readonly keymap: readonly KeyBinding[],
  ) {
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
      this.statusLanguage,
      this.statusState,
    );

    const shell = el("div", "workbench");
    shell.append(toolbar, body, status);
    root.replaceChildren(shell);
    this.updateChrome();
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
