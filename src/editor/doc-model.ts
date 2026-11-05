// One open document: the replica of the Rust buffer (docs/ARCHITECTURE.md,
// 4.2) shared by every pane that shows it. A hub EditorState with the collab
// extension is the only thing that talks to Rust, through one serialized
// queue; panes are mirrors that forward their local edits to the hub and
// receive every hub change (split editors, M2b).
import { collab, getSyncedVersion, receiveUpdates, sendableUpdates } from "@codemirror/collab";
import { Annotation, EditorState, Transaction, type TransactionSpec } from "@codemirror/state";
import type { DecorationSet, EditorView } from "@codemirror/view";

import {
  commands,
  type CaretInfo,
  type DocId,
  type DocStatus,
  type DocumentEdit_Serialize as DocumentEdit,
  type EditorSelection as IpcSelection,
  type FoldSpan,
  type HoverResult,
  type NavKind,
  type NavTarget,
  type OpenedDocument_Serialize as OpenedDocument,
  type TextEdit,
} from "../ipc/bindings";
import { BackendError, errorMessage, type IpcResult, unwrap } from "../ipc/result";
import {
  backendUpdates,
  composePending,
  fromIpcSelection,
  syncedLength,
  toChangeTransaction,
  toIpcSelection,
} from "./convert";
import { buildDecorations } from "./highlight";
import { buildDiagnostics } from "./insight";
import { type KeyTrace, mark, markNextFrame } from "../perf/typing-trace";

/** Marks transactions that mirror a change made elsewhere (not to forward). */
export const mirrored = Annotation.define<boolean>();

/** What a document needs from the panes showing it. */
export interface DocPane {
  readonly view: EditorView;
  /** Replaces the pane's state after a resync. */
  reset(text: string): void;
  setHighlights(decorations: DecorationSet): void;
  setFolds(ranges: readonly FoldSpan[]): void;
  setDiagnostics(decorations: DecorationSet): void;
  setCaretInfo(info: CaretInfo | null): void;
  showExternalChange(show: boolean): void;
  applyDocumentEdit(edit: DocumentEdit): void;
  /** Keys typed while an Enter was in flight; replayed afterwards. */
  replayDeferredKeys(): void;
}

export interface DocHost {
  onStatus(doc: DocModel): void;
  onError(doc: DocModel, message: string): void;
}

const HIGHLIGHT_MARGIN = 4000;
const HIGHLIGHT_DEBOUNCE_MS = 40;
const FOLD_DEBOUNCE_MS = 300;
const CARET_DEBOUNCE_MS = 60;
const DIAGNOSTICS_RETRY_MS = 150;

type SelectionCall = (
  docId: DocId,
  version: number,
  sel: IpcSelection,
) => Promise<IpcResult<IpcSelection>>;

export class DocModel {
  readonly docId: DocId;
  readonly path: string;
  readonly language: string | null;
  readonly indentUnit: string;
  readonly tabWidth: number;
  readonly autoClose: readonly string[];
  /** Library code (outside the project or in node_modules): view only. */
  readonly readOnly: boolean;
  dirty: boolean;
  readonly panes = new Set<DocPane>();
  private hub: EditorState;
  private chain: Promise<void> = Promise.resolve();
  private pushQueued = false;
  private highlightTimer: ReturnType<typeof setTimeout> | undefined;
  private foldTimer: ReturnType<typeof setTimeout> | undefined;
  private caretTimer: ReturnType<typeof setTimeout> | undefined;
  private diagnosticsTimer: ReturnType<typeof setTimeout> | undefined;
  private destroyed = false;
  /** Traced keystrokes not yet confirmed by Rust / not yet highlighted. */
  private unconfirmed: KeyTrace[] = [];
  private unhighlighted: KeyTrace[] = [];

  constructor(
    doc: OpenedDocument,
    private readonly host: DocHost,
  ) {
    this.docId = doc.docId;
    this.path = doc.path;
    this.language = doc.language;
    this.indentUnit = doc.indentUnit;
    this.tabWidth = doc.tabWidth;
    this.autoClose = doc.autoClose;
    this.readOnly = doc.readOnly;
    this.dirty = doc.dirty;
    this.hub = this.createHub(doc.text, doc.version);
  }

  get name(): string {
    return this.path.split("/").pop() ?? this.path;
  }

  /** Current text (all panes show the same). */
  get text(): string {
    return this.hub.doc.toString();
  }

  get version(): number {
    return getSyncedVersion(this.hub);
  }

  attach(pane: DocPane): void {
    this.panes.add(pane);
    this.scheduleHighlights();
    this.scheduleFolds(0);
    this.scheduleDiagnostics(0);
  }

  detach(pane: DocPane): void {
    this.panes.delete(pane);
  }

  destroy(): void {
    this.destroyed = true;
    clearTimeout(this.highlightTimer);
    clearTimeout(this.foldTimer);
    clearTimeout(this.caretTimer);
    clearTimeout(this.diagnosticsTimer);
  }

  /** Runs after all queued work for this document has finished. */
  idle(): Promise<void> {
    return this.chain;
  }

  private createHub(text: string, version: number): EditorState {
    return EditorState.create({
      doc: text,
      extensions: [collab({ startVersion: version, clientID: `doc-${String(this.docId)}` })],
    });
  }

  // Changes from panes and from Rust

  /** A pane made a local edit: record it in the hub and mirror it. */
  applyLocal(tr: Transaction, from: DocPane, trace: KeyTrace | null = null): void {
    if (trace) this.unconfirmed.push(trace);
    const userEvent = tr.annotation(Transaction.userEvent);
    // The hub carries the pane's selections so undo can restore them.
    this.hub = this.hub.update({ selection: tr.startState.selection }).state;
    this.hub = this.hub.update({
      changes: tr.changes,
      selection: tr.newSelection,
      ...(userEvent ? { annotations: Transaction.userEvent.of(userEvent) } : {}),
    }).state;
    for (const pane of this.panes) {
      if (pane !== from)
        pane.view.dispatch({ changes: tr.changes, annotations: mirrored.of(true) });
    }
    this.afterChange();
  }

  /** Applies a collab spec (confirmations, backend edits) and mirrors it. */
  private applyHub(spec: TransactionSpec): void {
    const tr = this.hub.update(spec);
    this.hub = tr.state;
    if (tr.docChanged) {
      for (const pane of this.panes) {
        pane.view.dispatch({ changes: tr.changes, annotations: mirrored.of(true) });
      }
      this.afterChange();
    }
  }

  private afterChange(): void {
    if (sendableUpdates(this.hub).length > 0) this.schedulePush();
    this.scheduleHighlights();
    this.scheduleFolds(FOLD_DEBOUNCE_MS);
    // In-process results (oxc unused imports) follow edits without the server.
    this.scheduleDiagnostics(FOLD_DEBOUNCE_MS);
  }

  // Commands (all go through the queue)

  save(): Promise<void> {
    return this.enqueue(async () => {
      await this.pushAll();
      const result = unwrap(await commands.saveDocument(this.docId));
      this.applyBackendEdits(result.updates);
      this.setStatus(result.status);
    });
  }

  undo(pane: DocPane): Promise<void> {
    return this.history(pane, "undo");
  }

  redo(pane: DocPane): Promise<void> {
    return this.history(pane, "redo");
  }

  addCaret(pane: DocPane, direction: "up" | "down"): Promise<void> {
    return this.selectionCommand(pane, (id, v, sel) => commands.addCaret(id, v, sel, direction));
  }

  selectNextOccurrence(pane: DocPane): Promise<void> {
    return this.selectionCommand(pane, commands.selectNextOccurrence);
  }

  selectAllOccurrences(pane: DocPane): Promise<void> {
    return this.selectionCommand(pane, commands.selectAllOccurrences);
  }

  /** Enter in `pane`: newline and indentation computed in Rust. */
  newLine(pane: DocPane): Promise<void> {
    return this.enqueue(async () => {
      try {
        await this.pushAll();
        const update = unwrap(
          await commands.newLine(
            this.docId,
            getSyncedVersion(this.hub),
            toIpcSelection(pane.view.state.selection),
          ),
        );
        this.applyBackendEdits(update.updates);
        this.applySelection(pane, update.selection);
        this.setStatus(update.status);
      } finally {
        pane.replayDeferredKeys();
      }
    });
  }

  /** Sends all pending local edits to Rust. */
  flush(): Promise<void> {
    return this.enqueue(() => this.pushAll());
  }

  /** Replaces the buffer with the file on disk (one undoable step). */
  reload(): Promise<void> {
    this.showExternalChange(false);
    return this.enqueue(async () => {
      await this.pushAll();
      const result = unwrap(await commands.reloadDocument(this.docId, getSyncedVersion(this.hub)));
      this.applyBackendEdits(result.updates);
      this.setStatus(result.status);
    });
  }

  /**
   * The file changed on disk: reload an unmodified buffer; for a modified
   * one, ask (unless only our own save changed the file).
   */
  onDiskChange(): Promise<void> {
    return this.enqueue(async () => {
      await this.pushAll();
      if (!this.dirty) {
        const result = unwrap(
          await commands.reloadDocument(this.docId, getSyncedVersion(this.hub)),
        );
        this.applyBackendEdits(result.updates);
        this.setStatus(result.status);
        return;
      }
      if (unwrap(await commands.diskChanged(this.docId))) this.showExternalChange(true);
    });
  }

  showExternalChange(show: boolean): void {
    for (const pane of this.panes) pane.showExternalChange(show);
  }

  /** Replace in Files edits, applied as a local change (pushed, undoable). */
  applyReplace(version: number, edits: TextEdit[]): Promise<void> {
    return this.enqueue(async () => {
      await this.pushAll();
      if (getSyncedVersion(this.hub) !== version) {
        throw new Error(`${this.name} changed while replacing; run Replace again`);
      }
      const pane = this.panes.values().next().value;
      if (!pane) throw new Error(`${this.name} is not shown`);
      pane.view.dispatch({
        changes: edits.map((e) => ({ from: e.from, to: e.to, insert: e.insert })),
        userEvent: "input.replace",
      });
      await this.pushAll();
    });
  }

  /** Applies a completion edit as one undoable view transaction. */
  applyDocumentEdit(edit: DocumentEdit): Promise<void> {
    return this.enqueue(async () => {
      await this.pushAll();
      if (this.version !== edit.version) {
        throw new Error(`${this.name} changed while accepting completion`);
      }
      const pane = this.panes.values().next().value;
      if (!pane) throw new Error(`${this.name} is not shown`);
      pane.applyDocumentEdit(edit);
      await this.pushAll();
    });
  }

  private history(pane: DocPane, kind: "undo" | "redo"): Promise<void> {
    return this.enqueue(async () => {
      await this.pushAll();
      const call = kind === "undo" ? commands.undo : commands.redo;
      const update = unwrap(await call(this.docId, getSyncedVersion(this.hub)));
      if (!update) return;
      this.applyBackendEdits(update.updates);
      this.applySelection(pane, update.selection);
      this.setStatus(update.status);
    });
  }

  private selectionCommand(pane: DocPane, call: SelectionCall): Promise<void> {
    return this.enqueue(async () => {
      await this.pushAll();
      const result = unwrap(
        await call(
          this.docId,
          getSyncedVersion(this.hub),
          toIpcSelection(pane.view.state.selection),
        ),
      );
      this.applySelection(pane, result);
    });
  }

  // Sync with Rust

  private enqueue(task: () => Promise<void>): Promise<void> {
    const run = this.chain.then(async () => {
      if (!this.destroyed) await task();
    });
    this.chain = run.catch((error: unknown) => this.recover(error));
    return this.chain;
  }

  private schedulePush(): void {
    if (this.pushQueued) return;
    this.pushQueued = true;
    void this.enqueue(async () => {
      this.pushQueued = false;
      await this.pushAll();
    });
  }

  /** Sends unconfirmed local transactions until none are left. */
  private async pushAll(): Promise<void> {
    for (;;) {
      const updates = sendableUpdates(this.hub);
      if (updates.length === 0) return;
      const status = unwrap(
        await commands.applyChanges(
          this.docId,
          getSyncedVersion(this.hub),
          updates.map(toChangeTransaction),
        ),
      );
      this.applyHub(
        receiveUpdates(
          this.hub,
          updates.map((u) => ({ changes: u.changes, clientID: u.clientID })),
        ),
      );
      for (const trace of this.unconfirmed) mark(trace, "rust_confirmed");
      this.unhighlighted.push(...this.unconfirmed.splice(0));
      this.setStatus(status);
    }
  }

  /** Applies backend edits, rebasing local edits typed meanwhile. */
  private applyBackendEdits(steps: TextEdit[][]): void {
    if (steps.length === 0) return;
    const length = syncedLength(this.hub.doc.length, sendableUpdates(this.hub));
    this.applyHub(receiveUpdates(this.hub, backendUpdates(steps, length)));
  }

  /** Applies a backend selection (relative to the synced version) to a pane. */
  private applySelection(pane: DocPane, selection: IpcSelection): void {
    const pending = sendableUpdates(this.hub);
    const length = syncedLength(this.hub.doc.length, pending);
    let next = fromIpcSelection(selection, length);
    if (pending.length > 0) next = next.map(composePending(pending, length));
    pane.view.dispatch({ selection: next, scrollIntoView: true, userEvent: "select" });
    pane.view.focus();
  }

  /** Whether Rust has exactly the text the panes show. */
  private isSynced(version: number): boolean {
    return getSyncedVersion(this.hub) === version && sendableUpdates(this.hub).length === 0;
  }

  // Highlights, folds, caret info

  scheduleHighlights(): void {
    clearTimeout(this.highlightTimer);
    this.highlightTimer = setTimeout(() => {
      void this.enqueue(() => this.fetchHighlights());
    }, HIGHLIGHT_DEBOUNCE_MS);
  }

  private async fetchHighlights(): Promise<void> {
    if (this.language === null || this.panes.size === 0) return;
    await this.pushAll();
    let from = Infinity;
    let to = 0;
    for (const pane of this.panes) {
      from = Math.min(from, pane.view.viewport.from);
      to = Math.max(to, pane.view.viewport.to);
    }
    const result = unwrap(
      await commands.highlights(
        this.docId,
        Math.max(0, from - HIGHLIGHT_MARGIN),
        to + HIGHLIGHT_MARGIN,
      ),
    );
    if (result.version !== getSyncedVersion(this.hub)) {
      this.scheduleHighlights();
      return;
    }
    const pending = sendableUpdates(this.hub);
    const length = syncedLength(this.hub.doc.length, pending);
    let decorations = buildDecorations(result.spans, length);
    if (pending.length > 0) decorations = decorations.map(composePending(pending, length));
    for (const pane of this.panes) pane.setHighlights(decorations);
    for (const trace of this.unhighlighted.splice(0)) markNextFrame(trace, "highlight_frame");
  }

  private scheduleFolds(delay: number): void {
    clearTimeout(this.foldTimer);
    this.foldTimer = setTimeout(() => {
      void this.enqueue(async () => {
        if (this.language === null || this.panes.size === 0) return;
        await this.pushAll();
        const folds = unwrap(await commands.folds(this.docId));
        if (!this.isSynced(folds.version)) {
          this.scheduleFolds(FOLD_DEBOUNCE_MS);
          return;
        }
        for (const pane of this.panes) pane.setFolds(folds.ranges);
      });
    }, delay);
  }

  /** Breadcrumbs and bracket pair at `pane`'s caret (debounced). */
  scheduleCaretInfo(pane: DocPane): void {
    clearTimeout(this.caretTimer);
    this.caretTimer = setTimeout(() => {
      void this.enqueue(async () => {
        if (this.language === null || !this.panes.has(pane)) {
          pane.setCaretInfo(null);
          return;
        }
        await this.pushAll();
        const head = pane.view.state.selection.main.head;
        const info = unwrap(await commands.caretInfo(this.docId, head));
        if (this.isSynced(info.version) && pane.view.state.selection.main.head === head) {
          pane.setCaretInfo(info);
        }
      });
    }, CARET_DEBOUNCE_MS);
  }

  /** Caret info at `pane`'s caret now (not debounced), e.g. for Go to Matching Brace. */
  async caretInfoNow(pane: DocPane): Promise<CaretInfo | null> {
    const box: { info: CaretInfo | null } = { info: null };
    await this.enqueue(async () => {
      if (this.language === null) return;
      await this.pushAll();
      box.info = unwrap(await commands.caretInfo(this.docId, pane.view.state.selection.main.head));
    });
    return box.info;
  }

  // Code insight from the language server

  /** Fetches the diagnostics (after the server published new ones). */
  scheduleDiagnostics(delay: number): void {
    clearTimeout(this.diagnosticsTimer);
    this.diagnosticsTimer = setTimeout(() => {
      void this.enqueue(async () => {
        if (this.panes.size === 0) return;
        await this.pushAll();
        const result = unwrap(await commands.docDiagnostics(this.docId));
        if (!this.isSynced(result.version)) {
          this.scheduleDiagnostics(DIAGNOSTICS_RETRY_MS);
          return;
        }
        const length = this.hub.doc.length;
        const decorations = buildDiagnostics(result.items, length);
        for (const pane of this.panes) pane.setDiagnostics(decorations);
      });
    }, delay);
  }

  /** Hover / Quick Documentation at `pos` of `pane` (null: nothing to show). */
  hover(pane: DocPane, pos: number): Promise<HoverResult | null> {
    return this.insightRequest(pane, pos, async (version, offset) =>
      commands.hover(this.docId, version, offset),
    );
  }

  /** Navigation targets for the symbol at `pos` (default: the caret). */
  async navigate(pane: DocPane, kind: NavKind, pos: number | null): Promise<NavTarget[]> {
    return (
      (await this.insightRequest(pane, pos, async (version, offset) =>
        commands.gotoDefinition(this.docId, version, offset, kind),
      )) ?? []
    );
  }

  /** Sends a language server request after flushing edits to Rust. */
  requestAt<T>(
    pane: DocPane,
    pos: number | null,
    call: (version: number, offset: number) => Promise<IpcResult<T>>,
  ): Promise<T | null> {
    return this.insightRequest(pane, pos, call);
  }

  /**
   * Pushes pending edits through the queue, then sends `call` outside it so
   * typing never waits for the language server. `Cancelled` (a newer
   * request replaced this one) resolves to null; other errors throw.
   */
  private async insightRequest<T>(
    pane: DocPane,
    pos: number | null,
    call: (version: number, offset: number) => Promise<IpcResult<T>>,
  ): Promise<T | null> {
    // A holder: TypeScript cannot see the queued callback assigning it.
    const box: { at: { version: number; offset: number } | null } = { at: null };
    await this.enqueue(async () => {
      await this.pushAll();
      box.at = {
        version: getSyncedVersion(this.hub),
        offset: pos ?? pane.view.state.selection.main.head,
      };
    });
    if (box.at === null) return null;
    const result = await call(box.at.version, box.at.offset);
    if (result.status === "error") {
      if (result.error.kind === "cancelled") return null;
      throw new BackendError(result.error);
    }
    return result.data;
  }

  private setStatus(status: DocStatus): void {
    this.dirty = status.dirty;
    this.host.onStatus(this);
  }

  /**
   * After a failed request the replica may have diverged from Rust: report
   * the error and reload the authoritative text into the hub and every pane.
   */
  private async recover(error: unknown): Promise<void> {
    if (this.destroyed) return;
    this.host.onError(this, errorMessage(error));
    if (!(error instanceof BackendError)) return;
    try {
      const doc = unwrap(await openDocumentAt(this.path));
      this.hub = this.createHub(doc.text, doc.version);
      for (const pane of this.panes) pane.reset(doc.text);
      this.setStatus({ version: doc.version, dirty: doc.dirty });
      this.scheduleHighlights();
      this.scheduleFolds(0);
      this.scheduleDiagnostics(0);
    } catch (reloadError: unknown) {
      this.host.onError(this, `Reloading ${this.path} failed: ${errorMessage(reloadError)}`);
    }
  }
}

/** Whether `path` is absolute (a file outside the project). */
export function isAbsolutePath(path: string): boolean {
  return path.startsWith("/") || /^[A-Za-z]:[\\/]/.test(path) || path.startsWith("\\\\");
}

/** Opens a project-relative path, or an absolute one read-only. */
export function openDocumentAt(path: string): ReturnType<typeof commands.openDocument> {
  return isAbsolutePath(path) ? commands.openExternal(path) : commands.openDocument(path);
}
