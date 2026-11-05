// Git log tool window: commits of the current branch, or of the current
// file (following renames), virtualized.
import { commands, type GitCommit } from "../ipc/bindings";
import { errorMessage, unwrap } from "../ipc/result";
import { VirtualList } from "../ui/virtual-list";
import { formatDate } from "./gutter";

export class LogWindow {
  readonly element = document.createElement("section");
  private readonly list: VirtualList<GitCommit>;
  private readonly scope = document.createElement("span");
  private readonly empty = document.createElement("div");
  private readonly fileButton = document.createElement("button");
  private path: string | null = null;

  constructor(private readonly currentFile: () => string | null) {
    this.element.className = "tool-window git-log";
    this.element.dataset.testid = "git-log";
    this.element.setAttribute("aria-label", "Git Log");
    this.element.hidden = true;
    this.element.tabIndex = -1;
    const header = document.createElement("div");
    header.className = "tool-window-header";
    const start = document.createElement("div");
    start.className = "terminal-header-start";
    const title = document.createElement("span");
    title.className = "tool-window-title";
    title.textContent = "Git Log";
    this.scope.dataset.testid = "git-log-scope";
    const all = document.createElement("button");
    all.type = "button";
    all.className = "terminal-new";
    all.dataset.testid = "git-log-all";
    all.textContent = "All";
    all.addEventListener("click", () => {
      void this.show(null);
    });
    this.fileButton.type = "button";
    this.fileButton.className = "terminal-new";
    this.fileButton.dataset.testid = "git-log-file";
    this.fileButton.textContent = "Current File";
    this.fileButton.addEventListener("click", () => {
      void this.show(this.currentFile());
    });
    start.append(title, all, this.fileButton, this.scope);
    const close = document.createElement("button");
    close.type = "button";
    close.className = "tool-window-close";
    close.textContent = "×";
    close.setAttribute("aria-label", "Hide Git Log");
    close.addEventListener("click", () => {
      this.element.hidden = true;
    });
    header.append(start, close);
    this.list = new VirtualList<GitCommit>(
      22,
      (commit) => {
        const row = document.createElement("div");
        row.className = "log-row";
        row.dataset.testid = "log-commit";
        const subject = document.createElement("span");
        subject.className = "log-subject";
        subject.textContent = commit.subject;
        const author = document.createElement("span");
        author.className = "log-author";
        author.textContent = commit.author;
        const date = document.createElement("span");
        date.className = "log-date";
        date.textContent = formatDate(commit.time, true);
        const id = document.createElement("span");
        id.className = "log-id";
        id.textContent = commit.shortId;
        row.title = `${commit.id}\n${commit.author} <${commit.email}>`;
        row.append(subject, author, date, id);
        return row;
      },
      () => undefined,
      "log-commits",
    );
    this.empty.className = "tool-window-empty";
    this.element.append(header, this.list.element, this.empty);
  }

  get visible(): boolean {
    return !this.element.hidden;
  }

  /** Shows the history of `path` (null: the whole branch). */
  async show(path: string | null = this.path): Promise<void> {
    this.path = path;
    this.element.hidden = false;
    this.scope.textContent = path ? `History of ${path}` : "Current branch";
    this.fileButton.disabled = this.currentFile() === null;
    try {
      const commits = unwrap(await commands.gitLog(path));
      this.list.setItems(commits);
      this.empty.textContent = "No commits.";
      this.empty.hidden = commits.length > 0;
    } catch (error: unknown) {
      this.list.setItems([]);
      this.empty.textContent = errorMessage(error);
      this.empty.hidden = false;
    }
    this.element.focus();
  }
}
