// Main window layout: toolbar, project tree, editor groups (split editors),
// status bar.
import { open } from "@tauri-apps/plugin-dialog";

import { DocModel, type DocHost } from "../editor/doc-model";
import { EDITOR_ACTIONS, EditorPane, type PaneHost } from "../editor/editor-pane";
import { DoubleTapDetector } from "../editor/gesture";
import {
  commands,
  events,
  type AppInfo,
  type FileProblems,
  type FsChangeBatch,
  type IndexState,
  type KeyAction,
  type KeyBinding,
  type NavKind,
  type NavTarget,
  type ReplaceResult,
  type SearchTab,
  type ServerStatus,
} from "../ipc/bindings";
import { errorMessage, unwrap } from "../ipc/result";
import { FindInFiles } from "../popup/find-in-files";
import { NavChooser } from "../popup/nav-chooser";
import type { Popup } from "../popup/popup";
import { SearchEverywhere } from "../popup/search-everywhere";
import { GotoLine, RecentFiles } from "../popup/simple-popups";
import { TreeView } from "../tree/tree-view";
import { chordMatches, formatChord } from "../ui/keys";
import { ProblemsPanel } from "./problems";

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

/** At most this many editor groups side by side. */
const MAX_GROUPS = 2;

/** A column of the editor area with its own tabs. */
interface EditorGroup {
  element: HTMLElement;
  tabBar: HTMLElement;
  editors: HTMLElement;
  panes: EditorPane[];
  active: EditorPane | null;
}

export class Workbench implements PaneHost, DocHost {
  private readonly groups: EditorGroup[] = [];
  private focusedGroup: EditorGroup;
  /** Open documents by path; panes in any group share them. */
  private readonly docs = new Map<string, DocModel>();
  private readonly opening = new Map<string, Promise<DocModel | null>>();
  private readonly tree: TreeView;
  private readonly groupsEl = el("div", "editor-groups", "editor-groups");
  private readonly welcome = el("div", "welcome", "welcome");
  private readonly projectName = el("span", "toolbar-project", "project-name");
  private readonly statusPath = el("span", "status-path", "status-path");
  private readonly statusNotice = el("span", "status-notice", "status-notice");
  private readonly statusPosition = el("span", "status-position", "status-position");
  private readonly statusLanguage = el("span", "status-language", "status-language");
  private readonly statusState = el("span", "status-state", "status-state");
  private readonly statusIndex = el("span", "status-index", "status-index");
  private readonly statusServer = el("button", "status-server", "status-server");
  private readonly statusProblems = el("button", "status-problems", "status-problems");
  private readonly problems: ProblemsPanel;
  private serverStatus: ServerStatus | null = null;
  readonly mac: boolean;
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

    this.focusedGroup = this.createGroup();
    this.renderWelcome();
    this.problems = new ProblemsPanel((path, line, column) => {
      this.openLocation(path, line, column);
    });
    this.problems.onChange = () => {
      this.renderProblemCounts();
    };
    const editorArea = el("div", "editor-area");
    editorArea.append(this.groupsEl, this.welcome);
    const main = el("section", "main");
    main.append(editorArea, this.problems.element);

    const body = el("div", "body");
    body.append(sidebar, main);

    const status = el("footer", "status", "status-bar");
    this.statusServer.type = "button";
    this.statusServer.hidden = true;
    this.statusServer.addEventListener("click", () => {
      this.onServerStatusClick();
    });
    this.statusProblems.type = "button";
    this.statusProblems.title = "Problems";
    this.statusProblems.addEventListener("click", () => {
      this.runAction("toggleProblems");
    });
    this.renderProblemCounts();
    status.append(
      this.statusPath,
      this.statusNotice,
      this.statusProblems,
      this.statusPosition,
      this.statusIndex,
      this.statusServer,
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
      await events.serverStatusChanged.listen((event) => {
        this.onServerStatus(event.payload);
      });
      await events.problemsChanged.listen((event) => {
        this.onProblemsChanged(event.payload);
      });
    } catch (error: unknown) {
      this.notify(`Backend events unavailable: ${errorMessage(error)}`);
    }
  }

  /** Runs an action from a shortcut or Search Everywhere. */
  runAction(action: KeyAction): void {
    const pane = this.active;
    if (EDITOR_ACTIONS.has(action)) {
      pane?.runAction(action);
      return;
    }
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
        if (pane) {
          const state = pane.view.state;
          const line = state.doc.lineAt(state.selection.main.head);
          const current = `${String(line.number)}:${String(state.selection.main.head - line.from + 1)}`;
          this.showPopup(
            new GotoLine(current, (l, c) => {
              pane.reveal(l, c);
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
      case "splitRight":
        this.splitRight();
        return;
      case "toggleProblems":
        if (this.requireProject()) this.problems.toggle();
        if (!this.problems.visible) this.active?.view.focus();
        return;
      case "restartTypeScriptServer":
        if (this.requireProject()) void this.restartServer();
        return;
      default:
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

  private get active(): EditorPane | null {
    return this.focusedGroup.active;
  }

  /** Opens a file and, if given, moves the caret to a zero-based line and column. */
  openLocation(path: string, line?: number, column?: number): void {
    void (async () => {
      await this.openFile(path);
      const pane = this.focusedGroup.panes.find((p) => p.path === path);
      if (pane && line !== undefined) pane.reveal(line, column ?? 0);
    })();
  }

  async flushAll(): Promise<void> {
    await Promise.all([...this.docs.values()].map((d) => d.flush()));
  }

  async applyReplace(result: ReplaceResult): Promise<string[]> {
    const failures: string[] = [];
    for (const replace of result.openDocuments) {
      const doc = [...this.docs.values()].find((d) => d.docId === replace.docId);
      if (!doc) {
        failures.push(`document ${String(replace.docId)}`);
        continue;
      }
      try {
        await doc.applyReplace(replace.version, replace.edits);
      } catch (error: unknown) {
        failures.push(`${doc.name} (${errorMessage(error)})`);
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
      const doc = this.docs.get(change.path);
      if (!doc) continue;
      if (change.kind === "removed") {
        this.notify(`${doc.name} was deleted on disk; saving recreates it.`);
      } else {
        void doc.onDiskChange();
      }
    }
  }

  private onProblemsChanged(file: FileProblems): void {
    this.problems.update(file);
    this.docs.get(file.path)?.scheduleDiagnostics(0);
  }

  private onServerStatus(status: ServerStatus): void {
    this.serverStatus = status;
    const shown = status.state !== "notStarted";
    this.statusServer.hidden = !shown;
    this.statusServer.textContent = status.message;
    this.statusServer.title = status.detail;
    this.statusServer.dataset.state = status.state;
  }

  private onServerStatusClick(): void {
    const status = this.serverStatus;
    if (!status) return;
    if (status.state === "error" || status.state === "notFound" || status.state === "stopped") {
      void this.restartServer();
    } else {
      this.notify(status.detail.split("\n")[0] ?? "");
    }
  }

  private async restartServer(): Promise<void> {
    try {
      unwrap(await commands.restartLanguageServer());
    } catch (error: unknown) {
      this.notify(`Restarting the TypeScript server failed: ${errorMessage(error)}`);
    }
  }

  private renderProblemCounts(): void {
    const { errors, warnings, other } = this.problems.counts();
    this.statusProblems.replaceChildren();
    const part = (cls: string, icon: string, count: number) => {
      const span = el("span", `status-count ${cls}`);
      span.textContent = `${icon} ${String(count)}`;
      return span;
    };
    this.statusProblems.append(part("errors", "✖", errors), part("warnings", "▲", warnings));
    if (other > 0) this.statusProblems.append(part("other", "ℹ", other));
    this.statusProblems.setAttribute(
      "aria-label",
      `Problems: ${String(errors)} errors, ${String(warnings)} warnings`,
    );
    this.statusProblems.dataset.errors = String(errors);
    this.statusProblems.dataset.warnings = String(warnings);
    this.statusProblems.hidden = !this.hasProject;
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
      this.closeEverything();
      this.projectName.textContent = project.name;
      this.projectName.title = project.root;
      document.title = `${this.info.name} — ${project.name}`;
      this.hasProject = true;
      this.tree.setRoot(unwrap(await commands.listDir("")));
      this.notify("");
      this.renderWelcome(project.name);
      this.problems.setAll(unwrap(await commands.problems()));
      const servers = unwrap(await commands.languageServers());
      const server = servers[0];
      if (server) this.onServerStatus(server);
    } catch (error: unknown) {
      this.notify(`Cannot open ${path}: ${errorMessage(error)}`);
    }
  }

  /** Opens a file in `group` (default: the focused one). */
  async openFile(path: string, group: EditorGroup = this.focusedGroup): Promise<void> {
    const existing = group.panes.find((p) => p.path === path);
    if (existing) {
      this.activate(group, existing);
      return;
    }
    const doc = await this.documentFor(path);
    if (!doc) return;
    // The group may have opened it meanwhile.
    const again = group.panes.find((p) => p.path === path);
    if (again) {
      this.activate(group, again);
      return;
    }
    const pane = new EditorPane(doc, this.keymap, this);
    group.panes.push(pane);
    group.editors.append(pane.container);
    this.activate(group, pane);
  }

  private documentFor(path: string): Promise<DocModel | null> {
    const open = this.docs.get(path);
    if (open) return Promise.resolve(open);
    const pending = this.opening.get(path);
    if (pending) return pending;
    const task = (async () => {
      try {
        const opened = unwrap(await commands.openDocument(path));
        const doc = new DocModel(opened, this);
        this.docs.set(path, doc);
        this.notify(opened.restored ? `Restored unsaved changes in ${doc.name}` : "");
        return doc;
      } catch (error: unknown) {
        this.notify(`Cannot open ${path}: ${errorMessage(error)}`);
        return null;
      } finally {
        this.opening.delete(path);
      }
    })();
    this.opening.set(path, task);
    return task;
  }

  /** Opens the current file in the other group (creating it if needed). */
  private splitRight(): void {
    const pane = this.active;
    if (!pane) return;
    const target =
      this.groups.find((g) => g !== this.focusedGroup) ??
      (this.groups.length < MAX_GROUPS ? this.createGroup() : this.focusedGroup);
    const { head } = pane.view.state.selection.main;
    void (async () => {
      await this.openFile(pane.path, target);
      target.active?.revealOffset(head);
    })();
  }

  private createGroup(): EditorGroup {
    const element = el("div", "editor-group", "editor-group");
    const tabBar = el("div", "tabs", "tabs");
    tabBar.setAttribute("role", "tablist");
    const editors = el("div", "editors");
    element.append(tabBar, editors);
    const group: EditorGroup = { element, tabBar, editors, panes: [], active: null };
    element.addEventListener("mousedown", () => {
      if (this.focusedGroup !== group) {
        this.focusedGroup = group;
        this.updateChrome();
      }
    });
    this.groups.push(group);
    this.groupsEl.append(element);
    return group;
  }

  private groupOf(pane: EditorPane): EditorGroup | undefined {
    return this.groups.find((g) => g.panes.includes(pane));
  }

  /** Closes every pane and document (another project was opened). */
  private closeEverything(): void {
    for (const group of this.groups) {
      for (const pane of group.panes) pane.destroy();
      group.element.remove();
    }
    this.groups.length = 0;
    for (const doc of this.docs.values()) doc.destroy();
    this.docs.clear();
    this.focusedGroup = this.createGroup();
    this.problems.setAll([]);
    this.serverStatus = null;
    this.statusServer.hidden = true;
    this.updateChrome();
  }

  // PaneHost and DocHost

  onFocus(pane: EditorPane): void {
    const group = this.groupOf(pane);
    if (!group) return;
    if (this.focusedGroup !== group || group.active !== pane) {
      this.focusedGroup = group;
      group.active = pane;
      this.updateChrome();
    }
  }

  onCursor(pane: EditorPane): void {
    if (pane === this.active) this.updatePosition();
  }

  onStatus(): void {
    this.updateChrome();
  }

  onError(doc: DocModel, message: string): void {
    this.notify(`${doc.name}: ${message}`);
  }

  navigate(pane: EditorPane, kind: NavKind, pos: number | null): void {
    const label =
      kind === "definition"
        ? "declaration"
        : kind === "typeDefinition"
          ? "type declaration"
          : "implementation";
    void (async () => {
      let targets: NavTarget[];
      try {
        targets = await pane.doc.navigate(pane, kind, pos);
      } catch (error: unknown) {
        this.notify(errorMessage(error));
        return;
      }
      const [first] = targets;
      if (!first) {
        this.notify(`No ${label} found.`);
        return;
      }
      if (targets.length === 1) {
        this.openTarget(first);
        return;
      }
      this.showPopup(
        new NavChooser(`Choose ${label}`, targets, (target) => {
          this.openTarget(target);
        }),
      );
    })();
  }

  private openTarget(target: NavTarget): void {
    if (!target.inProject) {
      this.notify(`${target.path} is outside the project; opening it is not supported yet.`);
      return;
    }
    this.notify("");
    this.openLocation(target.path, target.line, target.column);
  }

  closePane(pane: EditorPane): void {
    const group = this.groupOf(pane);
    if (!group) return;
    const index = group.panes.indexOf(pane);
    group.panes.splice(index, 1);
    const doc = pane.doc;
    pane.destroy();
    if (group.active === pane) {
      group.active = null;
      const next = group.panes[Math.min(index, group.panes.length - 1)];
      if (next) this.activate(group, next);
    }
    if (group.panes.length === 0 && this.groups.length > 1) {
      group.element.remove();
      this.groups.splice(this.groups.indexOf(group), 1);
      if (this.focusedGroup === group && this.groups[0]) this.focusedGroup = this.groups[0];
      this.focusedGroup.active?.view.focus();
    }
    this.updateChrome();
    if (doc.panes.size === 0) {
      this.docs.delete(doc.path);
      void (async () => {
        // Let queued edits reach Rust first (unsaved text stays recoverable).
        await doc.idle();
        doc.destroy();
        try {
          unwrap(await commands.closeDocument(doc.docId));
        } catch (error: unknown) {
          this.notify(errorMessage(error));
        }
      })();
    }
  }

  private activate(group: EditorGroup, pane: EditorPane): void {
    group.active = pane;
    this.focusedGroup = group;
    for (const p of group.panes) p.container.hidden = p !== pane;
    this.updateChrome();
    pane.view.requestMeasure();
    pane.view.focus();
  }

  private updateChrome(): void {
    for (const group of this.groups) {
      group.element.classList.toggle(
        "focused",
        group === this.focusedGroup && this.groups.length > 1,
      );
      const items: HTMLElement[] = group.panes.map((pane) => {
        const doc = pane.doc;
        const item = el("div", "tab", "tab");
        item.setAttribute("role", "tab");
        item.setAttribute("aria-selected", String(pane === group.active));
        item.dataset.path = pane.path;
        item.title = pane.path;
        if (pane === group.active) item.classList.add("active");
        if (doc.dirty) item.classList.add("dirty");
        const label = el("span", "tab-label", "tab-label");
        label.textContent = pane.name;
        const marker = el("span", "tab-dirty", "tab-dirty");
        marker.textContent = doc.dirty ? "●" : "";
        marker.setAttribute("aria-label", doc.dirty ? "modified" : "");
        const close = el("button", "tab-close", "tab-close");
        close.type = "button";
        close.textContent = "×";
        close.setAttribute("aria-label", `Close ${pane.name}`);
        close.addEventListener("click", (event) => {
          event.stopPropagation();
          this.closePane(pane);
        });
        item.addEventListener("mousedown", (event) => {
          if (event.button === 1) {
            event.preventDefault();
            this.closePane(pane);
          }
        });
        item.addEventListener("click", () => {
          this.activate(group, pane);
        });
        item.append(label, marker, close);
        return item;
      });
      if (group.panes.length > 0 && this.groups.length < MAX_GROUPS) {
        const split = el("button", "tab-split", "split-right");
        split.type = "button";
        split.textContent = "⫿";
        split.title = "Split Right";
        split.setAttribute("aria-label", "Split Right");
        split.addEventListener("click", () => {
          if (group.active) this.activate(group, group.active);
          this.splitRight();
        });
        items.push(split);
      }
      group.tabBar.replaceChildren(...items);
    }

    const pane = this.active;
    const anyPane = this.groups.some((g) => g.panes.length > 0);
    this.welcome.hidden = anyPane;
    this.groupsEl.hidden = !anyPane;
    this.statusPath.textContent = pane?.path ?? "";
    this.statusLanguage.textContent = pane ? (pane.doc.language ?? "Plain text") : "";
    this.statusState.textContent = pane ? (pane.doc.dirty ? "Modified" : "Saved") : "";
    this.updatePosition();
  }

  private updatePosition(): void {
    const pane = this.active;
    if (!pane) {
      this.statusPosition.textContent = "";
      return;
    }
    const state = pane.view.state;
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

  notify(message: string): void {
    this.statusNotice.textContent = message;
  }
}
