// Conversions between CodeMirror values and the IPC DTOs. Positions on both
// sides are UTF-16 offsets, so no arithmetic beyond mapping is needed.
import { ChangeSet, EditorSelection, Transaction } from "@codemirror/state";
import type { Update } from "@codemirror/collab";

import type {
  ChangeTransaction,
  EditKind,
  EditorSelection as IpcSelection,
  TextEdit,
} from "../ipc/bindings";

/** Client id used for edits that originate in the Rust backend. */
export const BACKEND_CLIENT_ID = "ferrule-core";

export function toIpcSelection(selection: EditorSelection): IpcSelection {
  return {
    ranges: selection.ranges.map((r) => ({ anchor: r.anchor, head: r.head })),
    main: selection.mainIndex,
  };
}

export function fromIpcSelection(selection: IpcSelection, docLength: number): EditorSelection {
  const clamp = (n: number) => Math.max(0, Math.min(n, docLength));
  const ranges = selection.ranges.map((r) => EditorSelection.range(clamp(r.anchor), clamp(r.head)));
  if (ranges.length === 0) return EditorSelection.single(0);
  return EditorSelection.create(ranges, Math.min(selection.main, ranges.length - 1));
}

/** Edits of a change set, relative to the document before it. */
export function changeSetEdits(changes: ChangeSet): TextEdit[] {
  const edits: TextEdit[] = [];
  changes.iterChanges((fromA, toA, _fromB, _toB, inserted) => {
    edits.push({ from: fromA, to: toA, insert: inserted.toString() });
  });
  return edits;
}

/** Undo-grouping kind for a CodeMirror user event. */
export function editKind(tr: Transaction | undefined): EditKind {
  if (!tr) return "other";
  if (tr.isUserEvent("input.type")) return "typing";
  if (tr.isUserEvent("delete.backward") || tr.isUserEvent("delete.forward")) return "deleting";
  return "other";
}

/** Converts an unconfirmed collab update into the backend transaction. */
export function toChangeTransaction(update: Update & { origin?: Transaction }): ChangeTransaction {
  const origin = update.origin;
  const after = origin ? origin.newSelection : EditorSelection.single(0);
  const before = origin ? origin.startState.selection : after;
  return {
    edits: changeSetEdits(update.changes),
    kind: editKind(origin),
    selectionBefore: toIpcSelection(before),
    selectionAfter: toIpcSelection(after),
  };
}

/**
 * Turns backend edit lists (one per version step, each relative to the text
 * before it) into collab updates, starting from a document of `length`.
 */
export function backendUpdates(steps: TextEdit[][], length: number): Update[] {
  const updates: Update[] = [];
  let len = length;
  for (const edits of steps) {
    const changes = ChangeSet.of(
      edits.map((e) => ({ from: e.from, to: e.to, insert: e.insert })),
      len,
    );
    updates.push({ changes, clientID: BACKEND_CLIENT_ID });
    len = changes.newLength;
  }
  return updates;
}

/** Composes the changes of unconfirmed updates (empty if none). */
export function composePending(updates: readonly Update[], syncedLength: number): ChangeSet {
  return updates.reduce((acc, u) => acc.compose(u.changes), ChangeSet.empty(syncedLength));
}

/** Document length at the last confirmed version. */
export function syncedLength(docLength: number, pending: readonly Update[]): number {
  return pending.reduce((len, u) => len - (u.changes.newLength - u.changes.length), docLength);
}
