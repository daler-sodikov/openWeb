// One open document: a CodeMirror view acting as a replica of the Rust
// buffer (docs/ARCHITECTURE.md, 4.2). Every backend request for this
// document goes through one serialized queue, so Rust sees edits in order.
import { collab, getSyncedVersion, receiveUpdates, sendableUpdates } from "@codemirror/collab";
import { simplifySelection, standardKeymap } from "@codemirror/commands";
import { EditorSelection, EditorState, Prec, type Extension } from "@codemirror/state";
import {
  type Command,
  drawSelection,
  dropCursor,
  EditorView,
  highlightActiveLine,
  highlightActiveLineGutter,
  type KeyBinding as CmKeyBinding,
  keymap,
  lineNumbers,
} from "@codemirror/view";

import {
  commands,
  type DocId,
  type DocStatus,
  type EditorSelection as IpcSelection,
  type KeyAction,
  type KeyBinding,
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
import { DoubleModifierGesture } from "./gesture";
import { buildDecorations, highlightField, setHighlights } from "./highlight";

/** Highlight this many UTF-16 units beyond the visible range. */
const HIGHLIGHT_MARGIN = 4000;
const HIGHLIGHT_DEBOUNCE_MS = 40;
const INDENT = "  ";

export interface EditorTabHost {
  onStatus(tab: EditorTab, status: DocStatus): void;
  onCursor(tab: EditorTab): void;
  onError(tab: EditorTab, message: string): void;
  closeTab(tab: EditorTab): void;
}

type SelectionCall = (
  docId: DocId,
  version: number,
  sel: IpcSelection,
) => Promise<IpcResult<IpcSelection>>;

export class EditorTab {
  readonly docId: DocId;
  readonly path: string;
  readonly language: string | null;
  readonly container: HTMLElement;
  readonly view: EditorView;
  dirty: boolean;
  private chain: Promise<void> = Promise.resolve();
  private pushQueued = false;
  private highlightTimer: ReturnType<typeof setTimeout> | undefined;
  private readonly gesture = new DoubleModifierGesture();
  private destroyed = false;

  constructor(
    doc: OpenedDocument,
    private readonly bindings: readonly KeyBinding[],
    private readonly host: EditorTabHost,
  ) {
    this.docId = doc.docId;
    this.path = doc.path;
    this.language = doc.language;
    this.dirty = doc.dirty;
    this.container = document.createElement("div");
    this.container.className = "editor-host";
    this.container.dataset.path = doc.path;
    this.view = new EditorView({
      parent: this.container,
      state: this.createState(doc.text, doc.version),
    });
    this.scheduleHighlights();
  }

  get name(): string {
    return this.path.split("/").pop() ?? this.path;
  }

  destroy(): void {
    this.destroyed = true;
    clearTimeout(this.highlightTimer);
    this.view.destroy();
    this.container.remove();
  }

  /** Runs after all queued work for this document has finished. */
  idle(): Promise<void> {
    return this.chain;
  }

  save(): Promise<void> {
    return this.enqueue(async () => {
      await this.pushAll();
      this.setStatus(unwrap(await commands.saveDocument(this.docId)));
    });
  }

  undo(): Promise<void> {
    return this.history("undo");
  }

  redo(): Promise<void> {
    return this.history("redo");
  }

  addCaret(direction: "up" | "down"): Promise<void> {
    return this.selectionCommand((id, v, sel) => commands.addCaret(id, v, sel, direction));
  }

  selectNextOccurrence(): Promise<void> {
    return this.selectionCommand(commands.selectNextOccurrence);
  }

  selectAllOccurrences(): Promise<void> {
    return this.selectionCommand(commands.selectAllOccurrences);
  }

  private createState(text: string, version: number): EditorState {
    return EditorState.create({ doc: text, extensions: this.extensions(version) });
  }

  private extensions(version: number): Extension[] {
    return [
      lineNumbers(),
      highlightActiveLineGutter(),
      highlightActiveLine(),
      drawSelection(),
      dropCursor(),
      EditorState.allowMultipleSelections.of(true),
      Prec.highest(this.gestureHandlers()),
      keymap.of([
        ...this.actionKeys(),
        { key: "Tab", run: insertIndent },
        { key: "Escape", run: simplifySelection },
        ...standardKeymap,
      ]),
      collab({ startVersion: version, clientID: `view-${String(this.docId)}` }),
      highlightField,
      EditorView.contentAttributes.of({ "aria-label": `Editor for ${this.path}` }),
      EditorView.updateListener.of((update) => {
        if (update.docChanged && sendableUpdates(update.state).length > 0) this.schedulePush();
        if (update.docChanged || update.viewportChanged) this.scheduleHighlights();
        if (update.selectionSet || update.docChanged) this.host.onCursor(this);
      }),
    ];
  }

  private runAction(action: KeyAction): void {
    const actions: Record<KeyAction, () => Promise<void> | void> = {
      save: () => this.save(),
      undo: () => this.undo(),
      redo: () => this.redo(),
      addCaretAbove: () => this.addCaret("up"),
      addCaretBelow: () => this.addCaret("down"),
      selectNextOccurrence: () => this.selectNextOccurrence(),
      selectAllOccurrences: () => this.selectAllOccurrences(),
      closeTab: () => {
        this.host.closeTab(this);
      },
    };
    void actions[action]();
  }

  private actionKeys(): CmKeyBinding[] {
    return this.bindings.flatMap((binding): CmKeyBinding[] =>
      binding.stroke.type === "chord"
        ? [
            {
              key: binding.stroke.keys,
              preventDefault: true,
              run: () => {
                this.runAction(binding.action);
                return true;
              },
            },
          ]
        : [],
    );
  }

  private gestureHandlers(): Extension {
    const gestures = this.bindings.flatMap((b) =>
      b.stroke.type === "doubleModifier"
        ? [
            {
              modifier: b.stroke.modifier === "control" ? "Control" : "Alt",
              key: b.stroke.key,
              action: b.action,
            },
          ]
        : [],
    );
    return EditorView.domEventHandlers({
      // IntelliJ-style Alt+Shift+Click adds a caret. Handled here because
      // CodeMirror treats Shift+Click as "extend selection", which would win
      // over `clickAddsSelectionRange`.
      mousedown: (event, view) => {
        if (event.button !== 0 || !event.altKey || !event.shiftKey) return false;
        const pos = view.posAtCoords({ x: event.clientX, y: event.clientY });
        if (pos === null) return false;
        event.preventDefault();
        view.dispatch({
          selection: view.state.selection.addRange(EditorSelection.cursor(pos)),
          userEvent: "select.pointer",
        });
        view.focus();
        return true;
      },
      keydown: (event) => {
        const armed = this.gesture.keydown(event.key, event.timeStamp);
        const match = gestures.find((g) => g.modifier === armed && g.key === event.key);
        if (!match) return false;
        event.preventDefault();
        this.runAction(match.action);
        return true;
      },
      keyup: (event) => {
        this.gesture.keyup(event.key, event.timeStamp);
        return false;
      },
    });
  }

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
      const updates = sendableUpdates(this.view.state);
      if (updates.length === 0) return;
      const status = unwrap(
        await commands.applyChanges(
          this.docId,
          getSyncedVersion(this.view.state),
          updates.map(toChangeTransaction),
        ),
      );
      this.view.dispatch(
        receiveUpdates(
          this.view.state,
          updates.map((u) => ({ changes: u.changes, clientID: u.clientID })),
        ),
      );
      this.setStatus(status);
    }
  }

  private history(kind: "undo" | "redo"): Promise<void> {
    return this.enqueue(async () => {
      await this.pushAll();
      const call = kind === "undo" ? commands.undo : commands.redo;
      const update = unwrap(await call(this.docId, getSyncedVersion(this.view.state)));
      if (!update) return;
      this.applyBackendEdits(update.updates);
      this.applySelection(update.selection);
      this.setStatus(update.status);
    });
  }

  private selectionCommand(call: SelectionCall): Promise<void> {
    return this.enqueue(async () => {
      await this.pushAll();
      const state = this.view.state;
      const result = unwrap(
        await call(this.docId, getSyncedVersion(state), toIpcSelection(state.selection)),
      );
      this.applySelection(result);
    });
  }

  /** Applies backend edits, rebasing any local edits typed meanwhile. */
  private applyBackendEdits(steps: TextEdit[][]): void {
    const state = this.view.state;
    const length = syncedLength(state.doc.length, sendableUpdates(state));
    this.view.dispatch(receiveUpdates(state, backendUpdates(steps, length)));
  }

  /** Applies a backend selection (relative to the synced version). */
  private applySelection(selection: IpcSelection): void {
    const state = this.view.state;
    const pending = sendableUpdates(state);
    const length = syncedLength(state.doc.length, pending);
    let next = fromIpcSelection(selection, length);
    if (pending.length > 0) next = next.map(composePending(pending, length));
    this.view.dispatch({ selection: next, scrollIntoView: true, userEvent: "select" });
    this.view.focus();
  }

  private scheduleHighlights(): void {
    clearTimeout(this.highlightTimer);
    this.highlightTimer = setTimeout(() => {
      void this.enqueue(() => this.fetchHighlights());
    }, HIGHLIGHT_DEBOUNCE_MS);
  }

  private async fetchHighlights(): Promise<void> {
    if (this.language === null) return;
    await this.pushAll();
    const { from, to } = this.view.viewport;
    const result = unwrap(
      await commands.highlights(
        this.docId,
        Math.max(0, from - HIGHLIGHT_MARGIN),
        to + HIGHLIGHT_MARGIN,
      ),
    );
    const state = this.view.state;
    if (result.version !== getSyncedVersion(state)) {
      this.scheduleHighlights();
      return;
    }
    const pending = sendableUpdates(state);
    const length = syncedLength(state.doc.length, pending);
    let decorations = buildDecorations(result.spans, length);
    if (pending.length > 0) decorations = decorations.map(composePending(pending, length));
    this.view.dispatch({ effects: setHighlights.of(decorations) });
  }

  private setStatus(status: DocStatus): void {
    this.dirty = status.dirty;
    this.host.onStatus(this, status);
  }

  /**
   * After a failed request the replica may have diverged from Rust: report
   * the error and reload the authoritative text. Local edits that were not
   * confirmed are lost in that case (Rust's copy and the recovery file win).
   */
  private async recover(error: unknown): Promise<void> {
    if (this.destroyed) return;
    this.host.onError(this, errorMessage(error));
    if (!(error instanceof BackendError)) return;
    try {
      const doc = unwrap(await commands.openDocument(this.path));
      this.view.setState(this.createState(doc.text, doc.version));
      this.setStatus({ version: doc.version, dirty: doc.dirty });
      this.scheduleHighlights();
    } catch (reloadError: unknown) {
      this.host.onError(this, `Reloading ${this.path} failed: ${errorMessage(reloadError)}`);
    }
  }
}

const insertIndent: Command = (view) => {
  view.dispatch(
    view.state.update(view.state.replaceSelection(INDENT), {
      scrollIntoView: true,
      userEvent: "input.type",
    }),
  );
  return true;
};
