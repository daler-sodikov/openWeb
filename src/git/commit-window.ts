// Commit tool window: changed and unversioned files with checkboxes
// (checked = staged), the message, Amend and Commit. All Git work is in
// Rust; this shows the status it pushes.
import { commands, type GitFile, type GitStatus } from "../ipc/bindings";
import { errorMessage, unwrap } from "../ipc/result";
import { VirtualList } from "../ui/virtual-list";

const ROW = 22;

export type CommitRow =
  { kind: "group"; label: string; count: number } | { kind: "file"; file: GitFile };

const LETTERS: Record<GitFile["kind"], string> = {
  conflicted: "C",
  modified: "M",
  added: "A",
  deleted: "D",
  renamed: "R",
  untracked: "U",
  ignored: "I",
};

/** Rows of the commit window: changes first, then unversioned files. */
export function commitRows(status: GitStatus): CommitRow[] {
  const tracked = status.files.filter((f) => f.kind !== "untracked" && f.kind !== "ignored");
  const untracked = status.files.filter((f) => f.kind === "untracked");
  const rows: CommitRow[] = [];
  const group = (label: string, files: GitFile[]) => {
    if (files.length === 0) return;
    rows.push({ kind: "group", label, count: files.length });
    for (const file of [...files].sort((a, b) => a.path.localeCompare(b.path))) {
      rows.push({ kind: "file", file });
    }
  };
  group("Changes", tracked);
  group("Unversioned Files", untracked);
  return rows;
}

export interface CommitHost {
  showDiff(path: string): void;
  openFile(path: string): void;
  notify(message: string): void;
}

export class CommitWindow {
  readonly element = document.createElement("section");
  private readonly list: VirtualList<CommitRow>;
  private readonly empty = document.createElement("div");
  private readonly message = document.createElement("textarea");
  private readonly amend = document.createElement("input");
  private readonly commitButton = document.createElement("button");
  private readonly error = document.createElement("div");
  private status: GitStatus | null = null;
  private busy = false;

  constructor(private readonly host: CommitHost) {
    this.element.className = "tool-window commit";
    this.element.dataset.testid = "commit";
    this.element.setAttribute("aria-label", "Commit");
    this.element.hidden = true;
    this.element.tabIndex = -1;
    const header = document.createElement("div");
    header.className = "tool-window-header";
    const title = document.createElement("span");
    title.className = "tool-window-title";
    title.textContent = "Commit";
    const close = document.createElement("button");
    close.type = "button";
    close.className = "tool-window-close";
    close.textContent = "×";
    close.setAttribute("aria-label", "Hide Commit");
    close.addEventListener("click", () => {
      this.toggle(false);
    });
    header.append(title, close);

    this.list = new VirtualList<CommitRow>(
      ROW,
      (row) => this.renderRow(row),
      () => undefined,
      "commit-files",
    );
    this.list.element.addEventListener("keydown", (event) => {
      this.onListKey(event);
    });
    this.list.element.tabIndex = 0;
    this.empty.className = "tool-window-empty";
    this.empty.textContent = "No changes.";

    const form = document.createElement("div");
    form.className = "commit-form";
    this.message.className = "commit-message";
    this.message.dataset.testid = "commit-message";
    this.message.placeholder = "Commit message";
    this.message.setAttribute("aria-label", "Commit message");
    this.message.addEventListener("keydown", (event) => {
      if (event.key === "Enter" && (event.ctrlKey || event.metaKey)) {
        event.preventDefault();
        void this.commit();
      }
    });
    const amendLabel = document.createElement("label");
    amendLabel.className = "commit-amend";
    this.amend.type = "checkbox";
    this.amend.dataset.testid = "commit-amend";
    this.amend.addEventListener("change", () => {
      void this.onAmend();
    });
    amendLabel.append(this.amend, " Amend");
    this.commitButton.type = "button";
    this.commitButton.className = "toolbar-button";
    this.commitButton.dataset.testid = "commit-button";
    this.commitButton.textContent = "Commit";
    this.commitButton.addEventListener("click", () => {
      void this.commit();
    });
    const actions = document.createElement("div");
    actions.className = "commit-actions";
    actions.append(amendLabel, this.commitButton);
    this.error.className = "commit-error";
    this.error.dataset.testid = "commit-error";
    this.error.setAttribute("role", "alert");
    this.error.hidden = true;
    form.append(this.message, actions, this.error);

    const body = document.createElement("div");
    body.className = "commit-body";
    const files = document.createElement("div");
    files.className = "commit-files";
    files.append(this.list.element, this.empty);
    body.append(files, form);
    this.element.append(header, body);
  }

  get visible(): boolean {
    return !this.element.hidden;
  }

  toggle(show = !this.visible): void {
    this.element.hidden = !show;
    if (show) {
      this.render();
      this.message.focus();
    }
  }

  update(status: GitStatus): void {
    this.status = status;
    if (this.visible) this.render();
  }

  /** Files that will be committed (checked). */
  private checked(): string[] {
    return (this.status?.files ?? []).filter((f) => f.staged).map((f) => f.path);
  }

  private render(): void {
    const status = this.status;
    if (status?.unavailable) {
      this.empty.textContent = status.unavailable;
      this.empty.hidden = false;
      this.list.setItems([]);
      return;
    }
    const rows = status ? commitRows(status) : [];
    const keep = this.list.element.scrollTop;
    this.list.setItems(rows);
    this.list.element.scrollTop = keep;
    this.empty.textContent = "No changes.";
    this.empty.hidden = rows.length > 0;
    const count = this.checked().length;
    this.commitButton.textContent = count > 0 ? `Commit (${String(count)})` : "Commit";
  }

  private renderRow(row: CommitRow): HTMLElement {
    const el = document.createElement("div");
    if (row.kind === "group") {
      el.className = "commit-group";
      el.textContent = `${row.label} (${String(row.count)})`;
      return el;
    }
    const { file } = row;
    el.className = `commit-file git-${file.kind}`;
    el.dataset.testid = "commit-file";
    el.dataset.path = file.path;
    const box = document.createElement("input");
    box.type = "checkbox";
    box.dataset.testid = "commit-check";
    box.checked = file.staged;
    // Partly staged: staged and also changed after staging.
    box.indeterminate = file.staged && file.unstaged && file.kind !== "untracked";
    box.setAttribute("aria-label", `Include ${file.path}`);
    // The row selects on mousedown; let the box take its own click.
    box.addEventListener("mousedown", (event) => {
      event.stopPropagation();
    });
    box.addEventListener("change", () => {
      void this.setStaged([file.path], box.checked);
    });
    const letter = document.createElement("span");
    letter.className = "commit-letter";
    letter.textContent = LETTERS[file.kind];
    letter.title = file.kind;
    const name = document.createElement("span");
    name.className = "commit-path";
    name.textContent = file.origPath ? `${file.origPath} → ${file.path}` : file.path;
    el.append(box, letter, name);
    el.addEventListener("dblclick", () => {
      this.host.showDiff(file.path);
    });
    return el;
  }

  private onListKey(event: KeyboardEvent): void {
    const row = this.list.selectedItem();
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      this.list.move(event.key === "ArrowDown" ? 1 : -1);
      return;
    }
    if (row?.kind !== "file") return;
    if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "d") {
      event.preventDefault();
      this.host.showDiff(row.file.path);
    } else if (event.key === "Enter" || event.key === "F4") {
      event.preventDefault();
      this.host.openFile(row.file.path);
    } else if (event.key === " ") {
      event.preventDefault();
      void this.setStaged([row.file.path], !row.file.staged);
    }
  }

  private showError(message: string): void {
    this.error.textContent = message;
    this.error.hidden = message === "";
  }

  private async setStaged(paths: string[], staged: boolean): Promise<void> {
    try {
      unwrap(await (staged ? commands.gitStage(paths) : commands.gitUnstage(paths)));
      this.showError("");
    } catch (error: unknown) {
      this.showError(errorMessage(error));
    }
  }

  private async onAmend(): Promise<void> {
    if (!this.amend.checked || this.message.value.trim() !== "") return;
    try {
      this.message.value = unwrap(await commands.gitLastCommitMessage()) ?? "";
    } catch (error: unknown) {
      this.showError(errorMessage(error));
    }
  }

  private async commit(): Promise<void> {
    if (this.busy) return;
    this.busy = true;
    this.commitButton.disabled = true;
    try {
      const id = unwrap(
        await commands.gitCommit(this.message.value, this.amend.checked, this.checked()),
      );
      this.message.value = "";
      this.amend.checked = false;
      this.showError("");
      this.host.notify(`Committed ${id}.`);
    } catch (error: unknown) {
      this.showError(errorMessage(error));
    } finally {
      this.busy = false;
      this.commitButton.disabled = false;
    }
  }
}
