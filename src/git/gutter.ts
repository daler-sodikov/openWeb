// Git in the editor gutter: change markers against the index and, in a
// second column beside them, the staged changes (HEAD to index); both are
// hunks from Rust. Plus the optional blame column. Clicking a marker asks
// the host to show the hunk.
import { type Extension, type Range, RangeSet, StateEffect, StateField } from "@codemirror/state";
import { type EditorView, GutterMarker, gutter } from "@codemirror/view";

import type { GitBlameLine, GitHunk } from "../ipc/bindings";

class HunkMarker extends GutterMarker {
  constructor(readonly hunk: GitHunk) {
    super();
  }

  override eq(other: GutterMarker): boolean {
    return (
      other instanceof HunkMarker &&
      other.hunk.kind === this.hunk.kind &&
      other.hunk.staged === this.hunk.staged &&
      other.hunk.line === this.hunk.line &&
      other.hunk.lines === this.hunk.lines
    );
  }

  override toDOM(): Node {
    const el = document.createElement("div");
    const staged = this.hunk.staged;
    el.className = `git-marker git-${this.hunk.kind}${staged ? " git-staged" : ""}`;
    el.dataset.testid = staged ? "git-staged-marker" : "git-marker";
    el.dataset.kind = this.hunk.kind;
    el.dataset.line = String(this.hunk.line);
    const what =
      this.hunk.kind === "added"
        ? "Added lines"
        : this.hunk.kind === "deleted"
          ? "Deleted lines"
          : "Modified lines";
    el.title = staged ? `${what} (staged)` : what;
    return el;
  }
}

export const setGitHunks = StateEffect.define<readonly GitHunk[]>();

interface HunkState {
  hunks: readonly GitHunk[];
  /** Markers of the changes against the index. */
  markers: RangeSet<HunkMarker>;
  /** Markers of the staged changes (HEAD to index), in their own column. */
  staged: RangeSet<HunkMarker>;
}

function buildMarkers(
  view: { doc: EditorView["state"]["doc"] },
  allHunks: readonly GitHunk[],
  staged: boolean,
): RangeSet<HunkMarker> {
  const ranges: Range<HunkMarker>[] = [];
  const lines = view.doc.lines;
  for (const hunk of allHunks.filter((h) => h.staged === staged)) {
    const count = Math.max(hunk.lines, 1);
    // A deletion is drawn on the line after it (or the last line).
    const first = Math.min(hunk.line + 1, lines);
    const marker = new HunkMarker(hunk);
    for (let n = first; n < first + count && n <= lines; n++) {
      ranges.push(marker.range(view.doc.line(n).from));
    }
  }
  return RangeSet.of(ranges, true);
}

export const gitHunkField = StateField.define<HunkState>({
  create: () => ({
    hunks: [],
    markers: RangeSet.of<HunkMarker>([]),
    staged: RangeSet.of<HunkMarker>([]),
  }),
  update(state, tr) {
    for (const effect of tr.effects) {
      if (effect.is(setGitHunks)) {
        return {
          hunks: effect.value,
          markers: buildMarkers(tr.state, effect.value, false),
          staged: buildMarkers(tr.state, effect.value, true),
        };
      }
    }
    // Until fresh hunks arrive, markers move with the text.
    return tr.docChanged
      ? { ...state, markers: state.markers.map(tr.changes), staged: state.staged.map(tr.changes) }
      : state;
  },
});

/**
 * The change against the index covering zero-based `line`, if any. Staged
 * hunks are not returned: they cannot be rolled back.
 */
export function hunkAt(hunks: readonly GitHunk[], line: number): GitHunk | null {
  return (
    hunks.find(
      (h) =>
        !h.staged &&
        (h.kind === "deleted" ? h.line === line : line >= h.line && line < h.line + h.lines),
    ) ?? null
  );
}

/** A gutter column of the markers `pick` selects; a click shows the hunk. */
function hunkColumn(
  name: string,
  pick: (state: HunkState) => RangeSet<HunkMarker>,
  onClick: (hunk: GitHunk, event: MouseEvent) => void,
): Extension {
  return gutter({
    class: name,
    markers: (view) => pick(view.state.field(gitHunkField)),
    domEventHandlers: {
      mousedown: (view, block, event) => {
        let found: HunkMarker | null = null;
        pick(view.state.field(gitHunkField)).between(block.from, block.from, (_f, _t, m) => {
          found = m;
        });
        const marker = found as HunkMarker | null;
        if (!marker || !(event instanceof MouseEvent)) return false;
        onClick(marker.hunk, event);
        return true;
      },
    },
  });
}

export function gitGutter(onClick: (hunk: GitHunk, event: MouseEvent) => void): Extension {
  return [
    gitHunkField,
    hunkColumn("cm-git-gutter", (state) => state.markers, onClick),
    // Beside the first column: what is staged (HEAD to index).
    hunkColumn("cm-git-staged-gutter", (state) => state.staged, onClick),
  ];
}

// Blame

class BlameMarker extends GutterMarker {
  constructor(
    readonly line: GitBlameLine,
    readonly text: string,
  ) {
    super();
  }

  override eq(other: GutterMarker): boolean {
    return other instanceof BlameMarker && other.text === this.text;
  }

  override toDOM(): Node {
    const el = document.createElement("div");
    el.className = this.line.committed ? "git-blame" : "git-blame uncommitted";
    el.dataset.testid = "git-blame";
    el.textContent = this.text;
    el.title = this.line.committed
      ? `${this.line.commit} ${this.line.author} <${this.line.email}>\n${formatDate(this.line.time, true)}\n\n${this.line.summary}`
      : "Not committed yet";
    return el;
  }
}

/** Blame lines, or null when blame is off. */
export const setBlame = StateEffect.define<readonly GitBlameLine[] | null>();

export function formatDate(seconds: number, withTime = false): string {
  const date = new Date(seconds * 1000);
  return withTime
    ? date.toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" })
    : date.toLocaleDateString(undefined, { year: "numeric", month: "short", day: "numeric" });
}

/** Gutter text of a blame line: the author's short name and the date. */
export function blameLabel(line: GitBlameLine): string {
  if (!line.committed) return "";
  const author = line.author.split(" ")[0] ?? line.author;
  return `${author.slice(0, 12)} ${formatDate(line.time)}`;
}

const blameField = StateField.define<RangeSet<BlameMarker> | null>({
  create: () => null,
  update(markers, tr) {
    for (const effect of tr.effects) {
      if (!effect.is(setBlame)) continue;
      if (!effect.value) return null;
      const ranges: Range<BlameMarker>[] = [];
      const doc = tr.state.doc;
      for (const [i, line] of effect.value.entries()) {
        if (i + 1 > doc.lines) break;
        ranges.push(new BlameMarker(line, blameLabel(line)).range(doc.line(i + 1).from));
      }
      return RangeSet.of(ranges);
    }
    return markers && tr.docChanged ? markers.map(tr.changes) : markers;
  },
});

export const blameGutter: Extension = [
  blameField,
  gutter({
    class: "cm-blame-gutter",
    markers: (view) => view.state.field(blameField) ?? RangeSet.of<BlameMarker>([]),
  }),
];

export function blameShown(view: EditorView): boolean {
  return (view.state.field(blameField, false) ?? null) !== null;
}
