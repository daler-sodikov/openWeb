// Git in the workbench: the status from Rust (tree colors, commit window,
// status bar branch, gutter refresh) and the Git actions.
import type { DocModel } from "../editor/doc-model";
import type { EditorPane } from "../editor/editor-pane";
import { commands, type GitHunk, type GitStatus, type KeyAction } from "../ipc/bindings";
import { errorMessage } from "../ipc/result";
import type { Popup } from "../popup/popup";
import { BranchesPopup } from "./branches-popup";
import { CommitWindow } from "./commit-window";
import { DiffView } from "./diff-view";
import { hunkAt } from "./gutter";
import { HunkPopup } from "./hunk-popup";
import { LogWindow } from "./log-window";
import { GitStatusModel, type TreeGitClass } from "./status";

export const GIT_ACTIONS: ReadonlySet<KeyAction> = new Set<KeyAction>([
  "toggleCommit",
  "toggleGitLog",
  "gitBranches",
  "showDiff",
  "toggleBlame",
  "rollbackHunk",
]);

export interface GitHost {
  notify(message: string): void;
  openFile(path: string): void;
  activePane(): EditorPane | null;
  docs(): Iterable<DocModel>;
  redrawTree(): void;
  /** Saves every unsaved document (Git works on the files on disk). */
  saveAll(): Promise<void>;
  showPopup(popup: Popup): void;
  /** Shows one bottom tool window and hides the others. */
  showBottom(element: HTMLElement | null): void;
}

export class GitUi {
  readonly model = new GitStatusModel();
  readonly commit: CommitWindow;
  readonly log: LogWindow;
  readonly branch = document.createElement("button");

  constructor(private readonly host: GitHost) {
    this.commit = new CommitWindow({
      showDiff: (path) => {
        this.showDiff(path);
      },
      openFile: (path) => {
        host.openFile(path);
      },
      notify: (message) => {
        host.notify(message);
      },
    });
    this.log = new LogWindow(() => host.activePane()?.doc.path ?? null);
    this.branch.type = "button";
    this.branch.className = "status-branch";
    this.branch.dataset.testid = "status-branch";
    this.branch.hidden = true;
    this.branch.addEventListener("click", () => {
      this.run("gitBranches");
    });
    // Others may have changed the repository while the window was away.
    window.addEventListener("focus", () => {
      void commands.gitRefresh();
    });
  }

  /** Tree class for a row. */
  classFor(path: string, directory: boolean): TreeGitClass | null {
    return this.model.classFor(path, directory);
  }

  /** A project was opened: start from its current status. */
  async load(): Promise<void> {
    const result = await commands.gitStatus();
    if (result.status === "ok") this.onStatus(result.data);
  }

  /** Another project is opening: forget the old repository. */
  reset(): void {
    const none: GitStatus = {
      unavailable: "Looking for a Git repository…",
      branch: null,
      head: null,
      upstream: null,
      ahead: 0,
      behind: 0,
      files: [],
      changedDirs: [],
      ignoredDirs: [],
      generation: 0,
    };
    this.model.update(none);
    this.branch.hidden = true;
    this.commit.update(none);
    this.commit.toggle(false);
    this.log.element.hidden = true;
  }

  onStatus(status: GitStatus): void {
    const previous = this.model.status?.generation ?? -1;
    if (status.generation !== 0 && status.generation < previous) return;
    this.model.update(status);
    this.host.redrawTree();
    this.commit.update(status);
    this.branch.hidden = status.unavailable !== null;
    const name = status.branch ?? `detached ${status.head ?? ""}`;
    this.branch.textContent = `⎇ ${name}`;
    const sync =
      status.ahead || status.behind
        ? ` (${String(status.ahead)} ahead, ${String(status.behind)} behind ${status.upstream ?? ""})`
        : "";
    this.branch.title = `Git branch: ${name}${sync}. Click for branches.`;
    for (const doc of this.host.docs()) doc.scheduleGitChanges(0);
  }

  private available(): boolean {
    const reason = this.model.status?.unavailable;
    if (reason) this.host.notify(reason);
    return !reason;
  }

  run(action: KeyAction): void {
    const pane = this.host.activePane();
    switch (action) {
      case "toggleCommit":
        if (this.commit.visible) {
          this.commit.toggle(false);
          pane?.view.focus();
        } else {
          this.host.showBottom(this.commit.element);
          this.commit.toggle(true);
          // Unsaved edits belong in the list of changes, like in IntelliJ.
          void this.host.saveAll();
        }
        return;
      case "toggleGitLog":
        if (this.log.visible) {
          this.log.element.hidden = true;
          pane?.view.focus();
        } else if (this.available()) {
          this.host.showBottom(this.log.element);
          void this.log.show();
        }
        return;
      case "gitBranches":
        if (this.available()) {
          this.host.showPopup(
            new BranchesPopup((message) => {
              this.host.notify(message);
            }),
          );
        }
        return;
      case "showDiff":
        if (pane && this.available()) this.showDiff(pane.doc.path);
        return;
      case "toggleBlame":
        if (pane && this.available()) pane.doc.toggleBlame();
        return;
      case "rollbackHunk": {
        if (!pane) return;
        const line = pane.view.state.doc.lineAt(pane.view.state.selection.main.head).number - 1;
        const hunk = hunkAt(pane.doc.hunks, line);
        if (hunk) void this.rollback(pane.doc, hunk);
        else this.host.notify("No change at the caret.");
        return;
      }
      default:
        return;
    }
  }

  showDiff(path: string): void {
    this.host.showPopup(new DiffView(path));
  }

  showHunk(pane: EditorPane, hunk: GitHunk, event: MouseEvent): void {
    this.host.showPopup(
      new HunkPopup(
        hunk,
        { x: event.clientX + 8, y: event.clientY },
        {
          rollback: () => {
            void this.rollback(pane.doc, hunk);
          },
          showDiff: () => {
            this.showDiff(pane.doc.path);
          },
        },
      ),
    );
  }

  private async rollback(doc: DocModel, hunk: GitHunk): Promise<void> {
    try {
      await doc.rollback(hunk);
      doc.scheduleGitChanges(0);
    } catch (error: unknown) {
      this.host.notify(`Rollback failed: ${errorMessage(error)}`);
    }
  }
}
