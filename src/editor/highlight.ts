import { RangeSetBuilder, StateEffect, StateField } from "@codemirror/state";
import { Decoration, type DecorationSet, EditorView } from "@codemirror/view";

import type { HighlightSpan, TokenClass } from "../ipc/bindings";

/** Replaces the highlight decorations (spans computed in Rust). */
export const setHighlights = StateEffect.define<DecorationSet>();

/** Highlight decorations; mapped through edits until fresh spans arrive. */
export const highlightField = StateField.define<DecorationSet>({
  create: () => Decoration.none,
  update(decorations, tr) {
    let next = decorations.map(tr.changes);
    for (const effect of tr.effects) {
      if (effect.is(setHighlights)) next = effect.value;
    }
    return next;
  },
  provide: (field) => EditorView.decorations.from(field),
});

const marks = new Map<TokenClass, Decoration>();
function markFor(cls: TokenClass): Decoration {
  let mark = marks.get(cls);
  if (!mark) {
    mark = Decoration.mark({ class: `tok-${cls}` });
    marks.set(cls, mark);
  }
  return mark;
}

/** Builds decorations from sorted, non-overlapping spans within `length`. */
export function buildDecorations(spans: readonly HighlightSpan[], length: number): DecorationSet {
  const builder = new RangeSetBuilder<Decoration>();
  for (const span of spans) {
    if (span.from < span.to && span.to <= length)
      builder.add(span.from, span.to, markFor(span.class));
  }
  return builder.finish();
}
