// What a gutter change marker shows: the index text it replaced, and
// Rollback / Show Diff.
import type { GitHunk } from "../ipc/bindings";
import { Popup } from "../popup/popup";

export class HunkPopup extends Popup {
  constructor(
    hunk: GitHunk,
    at: { x: number; y: number },
    actions: { rollback: () => void; showDiff: () => void },
  ) {
    super("hunk-popup", "git-hunk", "Change");
    this.element.tabIndex = -1;
    this.element.style.position = "fixed";
    this.element.style.left = `${String(at.x)}px`;
    this.element.style.top = `${String(at.y)}px`;
    const bar = document.createElement("div");
    bar.className = "hunk-actions";
    const button = (label: string, testid: string, run: () => void) => {
      const b = document.createElement("button");
      b.type = "button";
      b.className = "toolbar-button";
      b.dataset.testid = testid;
      b.textContent = label;
      b.addEventListener("click", () => {
        this.close();
        run();
      });
      return b;
    };
    bar.append(
      button("Rollback", "git-hunk-rollback", actions.rollback),
      button("Show Diff", "git-hunk-diff", actions.showDiff),
    );
    const old = document.createElement("pre");
    old.className = "hunk-old";
    old.dataset.testid = "git-hunk-old";
    old.textContent =
      hunk.kind === "added" ? "Added lines (not in the index)." : hunk.oldText || " ";
    this.element.append(bar, old);
  }

  override open(): void {
    super.open();
    this.element.focus();
  }
}
