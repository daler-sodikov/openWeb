// Test run markers in the editor gutter: one per describe/it/test block
// (found by tree-sitter in Rust) with the block's last status. Clicking a
// marker asks the host to run that block.
import { type Extension, type Range, RangeSet, StateEffect, StateField } from "@codemirror/state";
import { GutterMarker, gutter } from "@codemirror/view";

import type { TestBlock } from "../ipc/bindings";
import { statusGlyph } from "./format";

class TestMarker extends GutterMarker {
  constructor(readonly block: TestBlock) {
    super();
  }

  override eq(other: GutterMarker): boolean {
    return (
      other instanceof TestMarker &&
      other.block.line === this.block.line &&
      (other.block.status ?? null) === (this.block.status ?? null) &&
      other.block.name.join("\u0000") === this.block.name.join("\u0000")
    );
  }

  override toDOM(): Node {
    const el = document.createElement("div");
    const status = this.block.status ?? null;
    el.className = `test-marker test-${status ?? "none"}`;
    el.dataset.testid = "test-marker";
    el.dataset.line = String(this.block.line);
    el.dataset.status = status ?? "none";
    el.textContent = statusGlyph(status);
    const name = this.block.name[this.block.name.length - 1] ?? "";
    el.title = `Run '${name}'${status && status !== "running" ? ` (last: ${status})` : ""}`;
    return el;
  }
}

export const setTestBlocks = StateEffect.define<readonly TestBlock[]>();

const testMarkerField = StateField.define<RangeSet<TestMarker>>({
  create: () => RangeSet.of<TestMarker>([]),
  update(markers, tr) {
    for (const effect of tr.effects) {
      if (!effect.is(setTestBlocks)) continue;
      const doc = tr.state.doc;
      const ranges: Range<TestMarker>[] = [];
      for (const block of effect.value) {
        if (block.line + 1 > doc.lines) continue;
        ranges.push(new TestMarker(block).range(doc.line(block.line + 1).from));
      }
      return RangeSet.of(ranges, true);
    }
    return tr.docChanged ? markers.map(tr.changes) : markers;
  },
});

export function testGutter(onClick: (block: TestBlock) => void): Extension {
  return [
    testMarkerField,
    gutter({
      class: "cm-test-gutter",
      markers: (view) => view.state.field(testMarkerField),
      domEventHandlers: {
        mousedown: (view, line) => {
          let found: TestMarker | null = null;
          view.state.field(testMarkerField).between(line.from, line.from, (_f, _t, m) => {
            found = m;
          });
          const marker = found as TestMarker | null;
          if (!marker) return false;
          // The block's line now (edits may have moved it).
          onClick({ ...marker.block, line: view.state.doc.lineAt(line.from).number - 1 });
          return true;
        },
      },
    }),
  ];
}
