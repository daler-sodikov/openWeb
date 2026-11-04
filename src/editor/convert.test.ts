import { describe, expect, it } from "vitest";
import { ChangeSet, EditorSelection, EditorState } from "@codemirror/state";

import {
  BACKEND_CLIENT_ID,
  backendUpdates,
  changeSetEdits,
  composePending,
  editKind,
  fromIpcSelection,
  syncedLength,
  toChangeTransaction,
  toIpcSelection,
} from "./convert";

describe("convert", () => {
  it("round-trips multi-range selections", () => {
    const sel = EditorSelection.create([EditorSelection.range(5, 2), EditorSelection.cursor(8)], 1);
    const ipc = toIpcSelection(sel);
    expect(ipc).toEqual({
      ranges: [
        { anchor: 5, head: 2 },
        { anchor: 8, head: 8 },
      ],
      main: 1,
    });
    expect(fromIpcSelection(ipc, 10).eq(sel)).toBe(true);
    // Clamped to the document; the clamped ranges overlap and merge.
    const clamped = fromIpcSelection(ipc, 4);
    expect(clamped.ranges.map((r) => [r.from, r.to])).toEqual([[2, 4]]);
  });

  it("extracts edits relative to the original document", () => {
    const changes = ChangeSet.of(
      [
        { from: 0, insert: "a" },
        { from: 3, to: 5, insert: "" },
      ],
      6,
    );
    expect(changeSetEdits(changes)).toEqual([
      { from: 0, to: 0, insert: "a" },
      { from: 3, to: 5, insert: "" },
    ]);
  });

  it("maps user events to edit kinds and captures selections", () => {
    const state = EditorState.create({ doc: "abc", selection: EditorSelection.cursor(1) });
    const tr = state.update({
      changes: { from: 1, insert: "x" },
      selection: EditorSelection.cursor(2),
      userEvent: "input.type",
    });
    expect(editKind(tr)).toBe("typing");
    expect(editKind(state.update({ userEvent: "delete.backward" }))).toBe("deleting");
    expect(editKind(state.update({ userEvent: "input.paste" }))).toBe("other");

    const out = toChangeTransaction({ changes: tr.changes, clientID: "me", origin: tr });
    expect(out.kind).toBe("typing");
    expect(out.edits).toEqual([{ from: 1, to: 1, insert: "x" }]);
    expect(out.selectionBefore.ranges[0]).toEqual({ anchor: 1, head: 1 });
    expect(out.selectionAfter.ranges[0]).toEqual({ anchor: 2, head: 2 });
  });

  it("builds sequential backend updates", () => {
    const updates = backendUpdates(
      [[{ from: 0, to: 2, insert: "" }], [{ from: 0, to: 0, insert: "xyz" }]],
      5,
    );
    expect(updates).toHaveLength(2);
    expect(updates.every((u) => u.clientID === BACKEND_CLIENT_ID)).toBe(true);
    const text = updates.reduce(
      (doc, u) => u.changes.apply(doc),
      EditorState.create({ doc: "hello" }).doc,
    );
    expect(text.toString()).toBe("xyzllo");
  });

  it("computes synced length and composes pending changes", () => {
    const a = ChangeSet.of([{ from: 0, insert: "ab" }], 3);
    const b = ChangeSet.of([{ from: 1, to: 3 }], 5);
    const pending = [
      { changes: a, clientID: "me" },
      { changes: b, clientID: "me" },
    ];
    expect(syncedLength(3, pending)).toBe(3);
    const composed = composePending(pending, 3);
    expect(composed.length).toBe(3);
    expect(composed.newLength).toBe(3);
    expect(composePending([], 7).empty).toBe(true);
  });
});
