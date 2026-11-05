// Git in the editor gutter: change markers against the index (hunks from
// Rust) and the optional blame column. Clicking a marker asks the host to
// show the hunk.
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
      other.hunk.line === this.hunk.line &&
      other.hunk.lines === this.hunk.lines
    );
  }

  override toDOM(): Node {
    const el = document.createElement("div");
    el.className = `git-marker git-${this.hunk.kind}`;
    el.dataset.testid = "git-marker";
    el.dataset.kind = this.hunk.kind;
    el.dataset.line = String(this.hunk.line);
    el.title =
      this.hunk.kind === "added"
        ? "Added lines"
        : this.hunk.kind === "deleted"
          ? "Deleted lines"
          : "Modified lines";
    return el;
  }
}

export const setGitHunks = StateEffect.define<readonly GitHunk[]>();

interface HunkState {
  hunks: readonly GitHunk[];
  markers: RangeSet<HunkMarker>;
}

function buildMarkers(
  view: { doc: EditorView["state"]["doc"] },
  hunks: readonly GitHunk[],
): RangeSet<HunkMarker> {
  const ranges: Range<HunkMarker>[] = [];
  const lines = view.doc.lines;
  for (const hunk of hunks) {
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
  create: () => ({ hunks: [], markers: RangeSet.of<HunkMarker>([]) }),
  update(state, tr) {
    for (const effect of tr.effects) {
      if (effect.is(setGitHunks)) {
        return { hunks: effect.value, markers: buildMarkers(tr.state, effect.value) };
      }
    }
    // Until fresh hunks arrive, markers move with the text.
    return tr.docChanged ? { ...state, markers: state.markers.map(tr.changes) } : state;
  },
});

/** The hunk covering zero-based `line`, if any. */
export function hunkAt(hunks: readonly GitHunk[], line: number): GitHunk | null {
  return (
    hunks.find((h) =>
      h.kind === "deleted" ? h.line === line : line >= h.line && line < h.line + h.lines,
    ) ?? null
  );
}

export function gitGutter(onClick: (hunk: GitHunk, event: MouseEvent) => void): Extension {
  return [
    gitHunkField,
    gutter({
      class: "cm-git-gutter",
      markers: (view) => view.state.field(gitHunkField).markers,
      domEventHandlers: {
        mousedown: (view, block, event) => {
          let found: HunkMarker | null = null;
          view.state.field(gitHunkField).markers.between(block.from, block.from, (_f, _t, m) => {
            found = m;
          });
          const marker = found as HunkMarker | null;
          if (!marker || !(event instanceof MouseEvent)) return false;
          onClick(marker.hunk, event);
          return true;
        },
      },
    }),
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
