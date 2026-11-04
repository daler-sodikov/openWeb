// Rendering of code insight from Rust: diagnostic squiggles, the hover
// tooltip (diagnostic messages plus documentation) and the Quick
// Documentation popup. No analysis happens here: ranges, messages and the
// documentation HTML (sanitized in Rust) arrive over IPC.
import { RangeSetBuilder, StateEffect, StateField } from "@codemirror/state";
import {
  Decoration,
  type DecorationSet,
  EditorView,
  showTooltip,
  type Tooltip,
} from "@codemirror/view";

import type { DiagnosticSpan, Severity } from "../ipc/bindings";

/** Replaces the diagnostic decorations. */
export const setDiagnostics = StateEffect.define<DecorationSet>();

/** Diagnostic squiggles; mapped through edits until fresh ones arrive. */
export const diagnosticsField = StateField.define<DecorationSet>({
  create: () => Decoration.none,
  update(decorations, tr) {
    let next = decorations.map(tr.changes);
    for (const effect of tr.effects) if (effect.is(setDiagnostics)) next = effect.value;
    return next;
  },
  provide: (field) => EditorView.decorations.from(field),
});

const SEVERITY_ORDER: Record<Severity, number> = {
  error: 0,
  warning: 1,
  information: 2,
  hint: 3,
};

function markFor(item: DiagnosticSpan): Decoration {
  const classes = [`cm-diag`, `cm-diag-${item.severity}`];
  if (item.unnecessary) classes.push("cm-diag-unnecessary");
  if (item.deprecated) classes.push("cm-diag-deprecated");
  return Decoration.mark({
    class: classes.join(" "),
    attributes: { "data-severity": item.severity },
    diagnostic: item,
  });
}

/**
 * Decorations for diagnostics of a document of `length` UTF-16 units. An
 * empty range is widened to one character so it stays visible.
 */
export function buildDiagnostics(items: readonly DiagnosticSpan[], length: number): DecorationSet {
  const ranges = items
    .map((item) => {
      let { from, to } = item;
      if (from > length) return null;
      to = Math.min(to, length);
      if (from === to) {
        if (to < length) to += 1;
        else if (from > 0) from -= 1;
        else return null;
      }
      return { from, to, item };
    })
    .filter((r): r is { from: number; to: number; item: DiagnosticSpan } => r !== null)
    // RangeSetBuilder needs ranges sorted by start (then by end).
    .sort((a, b) => a.from - b.from || a.to - b.to);
  const builder = new RangeSetBuilder<Decoration>();
  for (const { from, to, item } of ranges) builder.add(from, to, markFor(item));
  return builder.finish();
}

/** Diagnostics whose range touches `pos`, most severe first. */
export function diagnosticsAt(decorations: DecorationSet, pos: number): DiagnosticSpan[] {
  const found: DiagnosticSpan[] = [];
  decorations.between(pos, pos, (_from, _to, deco) => {
    const item = (deco.spec as { diagnostic?: DiagnosticSpan }).diagnostic;
    if (item) found.push(item);
  });
  return found.sort((a, b) => SEVERITY_ORDER[a.severity] - SEVERITY_ORDER[b.severity]);
}

/** The tooltip content: diagnostic messages, then documentation. */
export function insightDom(
  diagnostics: readonly DiagnosticSpan[],
  html: string | null,
): HTMLElement {
  const dom = document.createElement("div");
  dom.className = "insight";
  dom.dataset.testid = "insight";
  for (const item of diagnostics) {
    const row = document.createElement("div");
    row.className = `insight-problem severity-${item.severity}`;
    row.dataset.testid = "insight-problem";
    const message = document.createElement("span");
    message.textContent = item.message;
    row.append(message);
    const origin = [item.source, item.code].filter((part) => part !== null).join(" ");
    if (origin) {
      const tag = document.createElement("span");
      tag.className = "insight-origin";
      tag.textContent = origin;
      row.append(tag);
    }
    dom.append(row);
  }
  if (html !== null) {
    const doc = document.createElement("div");
    doc.className = "insight-doc";
    doc.dataset.testid = "quick-doc";
    // Rendered from Markdown and sanitized in Rust (docs/ARCHITECTURE.md,
    // 4.7): escaped text, no raw HTML, no images, http(s)/mailto links only.
    doc.innerHTML = html;
    for (const link of doc.querySelectorAll("a")) {
      link.title = link.href;
      // Opening links in the system browser is not wired up yet; never
      // navigate the editor's webview away.
      link.addEventListener("click", (event) => {
        event.preventDefault();
      });
    }
    dom.append(doc);
  }
  return dom;
}

/** Shows or hides the Quick Documentation popup. */
export const setQuickDoc = StateEffect.define<Tooltip | null>();

/** Quick Documentation (Ctrl+Q); closed by edits, caret moves or Escape. */
export const quickDocField = StateField.define<Tooltip | null>({
  create: () => null,
  update(tooltip, tr) {
    for (const effect of tr.effects) if (effect.is(setQuickDoc)) return effect.value;
    if (tooltip && (tr.docChanged || tr.selection)) return null;
    return tooltip;
  },
  provide: (field) => showTooltip.from(field),
});
