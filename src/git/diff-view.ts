// Diff of a file against HEAD: side-by-side or unified rows from Rust,
// virtualized, with word-level highlights.
import {
  commands,
  type GitDiff,
  type GitDiffLine,
  type GitDiffMode,
  type GitSplitRow,
} from "../ipc/bindings";
import { errorMessage, unwrap } from "../ipc/result";
import { Popup } from "../popup/popup";
import { VirtualList } from "../ui/virtual-list";

const ROW = 20;
let lastMode: GitDiffMode = "sideBySide";

/** A line's text with the changed word ranges (UTF-16 pairs) marked. */
export function lineContent(line: GitDiffLine): HTMLElement {
  const text = document.createElement("span");
  text.className = "diff-text";
  let at = 0;
  for (let i = 0; i + 1 < line.words.length; i += 2) {
    const from = line.words[i] ?? 0;
    const to = line.words[i + 1] ?? from;
    if (from > at) text.append(line.text.slice(at, from));
    const mark = document.createElement("mark");
    mark.className = "diff-word";
    mark.textContent = line.text.slice(from, to);
    text.append(mark);
    at = to;
  }
  if (at < line.text.length) text.append(line.text.slice(at));
  return text;
}

function cell(line: GitDiffLine | null | undefined, side: "old" | "new"): HTMLElement {
  const el = document.createElement("div");
  el.className = `diff-cell ${line ? `diff-${line.kind}` : "diff-empty"}`;
  if (!line) return el;
  const number = document.createElement("span");
  number.className = "diff-number";
  number.textContent = String((side === "old" ? line.oldLine : line.newLine) ?? "");
  el.append(number, lineContent(line));
  return el;
}

export class DiffView extends Popup {
  private readonly title = document.createElement("div");
  private readonly labels = document.createElement("div");
  private readonly body = document.createElement("div");
  private readonly message = document.createElement("div");
  private readonly modeButton = document.createElement("button");
  private mode = lastMode;

  constructor(private readonly path: string) {
    super("diff-view", "diff-view", `Diff of ${path}`);
    this.element.tabIndex = -1;
    const header = document.createElement("div");
    header.className = "popup-title diff-header";
    this.title.textContent = path;
    this.modeButton.type = "button";
    this.modeButton.className = "toolbar-button";
    this.modeButton.dataset.testid = "diff-mode";
    this.modeButton.addEventListener("click", () => {
      this.mode = this.mode === "sideBySide" ? "unified" : "sideBySide";
      lastMode = this.mode;
      void this.load();
    });
    header.append(this.title, this.modeButton);
    this.labels.className = "diff-labels";
    this.body.className = "diff-body";
    this.message.className = "popup-footer";
    this.message.dataset.testid = "diff-message";
    this.element.append(header, this.labels, this.body, this.message);
  }

  override open(): void {
    super.open();
    this.element.focus();
    void this.load();
  }

  private async load(): Promise<void> {
    this.modeButton.textContent = this.mode === "sideBySide" ? "Unified" : "Side by Side";
    this.element.dataset.mode = this.mode;
    try {
      this.render(unwrap(await commands.gitDiff(this.path, this.mode)));
    } catch (error: unknown) {
      this.message.textContent = errorMessage(error);
    }
  }

  private render(diff: GitDiff): void {
    const old = document.createElement("span");
    old.textContent = diff.oldLabel;
    old.dataset.testid = "diff-old-label";
    const now = document.createElement("span");
    now.textContent = diff.newLabel;
    now.dataset.testid = "diff-new-label";
    this.labels.replaceChildren(old, now);
    this.message.textContent =
      diff.hunks === 0
        ? "No differences."
        : `${String(diff.hunks)} ${diff.hunks === 1 ? "difference" : "differences"}`;
    if (this.mode === "sideBySide") {
      const list = new VirtualList<GitSplitRow>(
        ROW,
        (row) => {
          const el = document.createElement("div");
          el.className = "diff-row diff-split";
          el.dataset.testid = "diff-row";
          el.append(cell(row.left, "old"), cell(row.right, "new"));
          return el;
        },
        () => undefined,
        "diff-rows",
      );
      this.body.replaceChildren(list.element);
      list.setItems(diff.split);
    } else {
      const list = new VirtualList<GitDiffLine>(
        ROW,
        (line) => {
          const el = document.createElement("div");
          el.className = `diff-row diff-unified diff-${line.kind}`;
          el.dataset.testid = "diff-row";
          el.dataset.kind = line.kind;
          const numbers = document.createElement("span");
          numbers.className = "diff-number";
          numbers.textContent = `${String(line.oldLine ?? "")} ${String(line.newLine ?? "")}`;
          const sign = document.createElement("span");
          sign.className = "diff-sign";
          sign.textContent = line.kind === "added" ? "+" : line.kind === "deleted" ? "-" : " ";
          el.append(numbers, sign, lineContent(line));
          return el;
        },
        () => undefined,
        "diff-rows",
      );
      this.body.replaceChildren(list.element);
      list.setItems(diff.unified);
    }
  }
}
