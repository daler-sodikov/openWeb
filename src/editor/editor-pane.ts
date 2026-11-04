// One visible editor: a CodeMirror view mirroring a DocModel, inside an
// editor group. Several panes (in different groups) can show the same
// document. Folding ranges, bracket pairs and breadcrumbs come from Rust.
import { closeBrackets, closeBracketsKeymap } from "@codemirror/autocomplete";
import { simplifySelection, standardKeymap } from "@codemirror/commands";
import {
  codeFolding,
  foldAll,
  foldCode,
  foldGutter,
  foldService,
  unfoldAll,
  unfoldCode,
} from "@codemirror/language";
import {
  EditorSelection,
  EditorState,
  Prec,
  StateEffect,
  StateField,
  type Extension,
} from "@codemirror/state";
import {
  type Command,
  Decoration,
  type DecorationSet,
  drawSelection,
  dropCursor,
  EditorView,
  highlightActiveLine,
  highlightActiveLineGutter,
  hoverTooltip,
  type KeyBinding as CmKeyBinding,
  keymap,
  lineNumbers,
  runScopeHandlers,
} from "@codemirror/view";

import type { CaretInfo, FoldSpan, KeyAction, KeyBinding, NavKind } from "../ipc/bindings";
import { errorMessage } from "../ipc/result";
import { type DocModel, type DocPane, mirrored } from "./doc-model";
import { DoubleModifierGesture } from "./gesture";
import { highlightField, setHighlights } from "./highlight";
import {
  diagnosticsAt,
  diagnosticsField,
  insightDom,
  quickDocField,
  setDiagnostics,
  setQuickDoc,
} from "./insight";

/** Actions handled by the focused editor; all others are global. */
export const EDITOR_ACTIONS: ReadonlySet<KeyAction> = new Set<KeyAction>([
  "save",
  "undo",
  "redo",
  "addCaretAbove",
  "addCaretBelow",
  "selectNextOccurrence",
  "selectAllOccurrences",
  "closeTab",
  "foldCollapse",
  "foldExpand",
  "foldCollapseAll",
  "foldExpandAll",
  "gotoMatchingBrace",
  "quickDocumentation",
  "gotoDeclaration",
  "gotoTypeDeclaration",
  "gotoImplementation",
]);

/** Delay before the mouse hover asks for documentation. */
const HOVER_DELAY_MS = 500;

export interface PaneHost {
  /** macOS (Cmd instead of Ctrl for Ctrl+Click). */
  readonly mac: boolean;
  onFocus(pane: EditorPane): void;
  onCursor(pane: EditorPane): void;
  closePane(pane: EditorPane): void;
  /** Go to Declaration and friends at `pos` (null: the caret). */
  navigate(pane: EditorPane, kind: NavKind, pos: number | null): void;
  notify(message: string): void;
}

// Folding ranges from Rust, mapped through edits until fresh ones arrive.
const setFoldRanges = StateEffect.define<readonly FoldSpan[]>();
const foldRangesField = StateField.define<readonly FoldSpan[]>({
  create: () => [],
  update(ranges, tr) {
    for (const effect of tr.effects) if (effect.is(setFoldRanges)) return effect.value;
    if (!tr.docChanged) return ranges;
    return ranges
      .map((r) => ({ from: tr.changes.mapPos(r.from, 1), to: tr.changes.mapPos(r.to, -1) }))
      .filter((r) => r.from < r.to);
  },
});
const rustFolds = foldService.of((state, lineStart, lineEnd) => {
  const range = state.field(foldRangesField).find((r) => r.from >= lineStart && r.from <= lineEnd);
  return range ? { from: range.from, to: range.to } : null;
});

// Matching bracket pair from Rust.
const setBrackets = StateEffect.define<DecorationSet>();
const bracketField = StateField.define<DecorationSet>({
  create: () => Decoration.none,
  update(decorations, tr) {
    let next = decorations.map(tr.changes);
    for (const effect of tr.effects) if (effect.is(setBrackets)) next = effect.value;
    return next;
  },
  provide: (field) => EditorView.decorations.from(field),
});
const bracketMark = Decoration.mark({ class: "cm-bracket-match" });

export class EditorPane implements DocPane {
  readonly container: HTMLElement;
  readonly view: EditorView;
  private readonly crumbsBar: HTMLElement;
  private externalBar: HTMLElement | null = null;
  private readonly gesture = new DoubleModifierGesture();
  /** Keys pressed while an Enter is waiting for Rust; replayed afterwards. */
  private deferredKeys: KeyboardEvent[] | null = null;

  constructor(
    readonly doc: DocModel,
    private readonly bindings: readonly KeyBinding[],
    private readonly host: PaneHost,
  ) {
    this.container = document.createElement("div");
    this.container.className = "editor-host";
    this.container.dataset.path = doc.path;
    this.crumbsBar = document.createElement("div");
    this.crumbsBar.className = "breadcrumbs";
    this.crumbsBar.dataset.testid = "breadcrumbs";
    this.view = new EditorView({
      state: this.createState(doc.text),
      dispatchTransactions: (trs, view) => {
        view.update(trs);
        for (const tr of trs) {
          if (tr.docChanged && !tr.annotation(mirrored)) this.doc.applyLocal(tr, this);
        }
      },
    });
    this.container.append(this.view.dom, this.crumbsBar);
    this.view.contentDOM.addEventListener("focus", () => {
      this.host.onFocus(this);
    });
    doc.attach(this);
  }

  get path(): string {
    return this.doc.path;
  }

  get name(): string {
    return this.doc.name;
  }

  destroy(): void {
    this.doc.detach(this);
    this.view.destroy();
    this.container.remove();
  }

  // DocPane

  reset(text: string): void {
    this.view.setState(this.createState(text));
  }

  setHighlights(decorations: DecorationSet): void {
    this.view.dispatch({ effects: setHighlights.of(decorations) });
  }

  setFolds(ranges: readonly FoldSpan[]): void {
    this.view.dispatch({ effects: setFoldRanges.of(ranges) });
  }

  setDiagnostics(decorations: DecorationSet): void {
    this.view.dispatch({ effects: setDiagnostics.of(decorations) });
  }

  setCaretInfo(info: CaretInfo | null): void {
    const pair = info?.brackets;
    const decorations = pair
      ? Decoration.set([
          bracketMark.range(pair.openFrom, pair.openTo),
          bracketMark.range(pair.closeFrom, pair.closeTo),
        ])
      : Decoration.none;
    this.view.dispatch({ effects: setBrackets.of(decorations) });
    this.renderCrumbs(info?.breadcrumbs ?? []);
  }

  showExternalChange(show: boolean): void {
    if (!show) {
      this.externalBar?.remove();
      this.externalBar = null;
      return;
    }
    if (this.externalBar) return;
    const bar = document.createElement("div");
    bar.className = "editor-bar";
    bar.dataset.testid = "external-change";
    bar.setAttribute("role", "alert");
    const text = document.createElement("span");
    text.textContent = "The file changed on disk, and this editor has unsaved changes.";
    const reload = document.createElement("button");
    reload.type = "button";
    reload.textContent = "Reload";
    reload.dataset.testid = "reload-from-disk";
    reload.addEventListener("click", () => {
      void this.doc.reload();
    });
    const keep = document.createElement("button");
    keep.type = "button";
    keep.textContent = "Keep my changes";
    keep.dataset.testid = "keep-changes";
    keep.addEventListener("click", () => {
      this.doc.showExternalChange(false);
      this.view.focus();
    });
    bar.append(text, reload, keep);
    this.container.prepend(bar);
    this.externalBar = bar;
  }

  replayDeferredKeys(): void {
    const keys = this.deferredKeys ?? [];
    this.deferredKeys = null;
    for (const [index, event] of keys.entries()) {
      // A replayed Enter may have started a new round trip (TypeScript cannot
      // see that the calls below change the field, hence the helper).
      const again = this.pendingDeferral();
      if (again !== null) {
        again.push(...keys.slice(index));
        return;
      }
      if (event.key.length === 1 && !event.ctrlKey && !event.metaKey) {
        this.view.dispatch(this.view.state.replaceSelection(event.key), {
          userEvent: "input.type",
          scrollIntoView: true,
        });
      } else {
        runScopeHandlers(this.view, event, "editor");
      }
    }
  }

  private pendingDeferral(): KeyboardEvent[] | null {
    return this.deferredKeys;
  }

  // Navigation

  /** Moves the caret to a zero-based line and UTF-16 column and focuses. */
  reveal(line: number, column: number): void {
    const doc = this.view.state.doc;
    const target = doc.line(Math.min(Math.max(line + 1, 1), doc.lines));
    this.revealOffset(Math.min(target.from + Math.max(column, 0), target.to));
  }

  revealOffset(offset: number): void {
    const pos = Math.min(Math.max(offset, 0), this.view.state.doc.length);
    this.view.dispatch({
      selection: { anchor: pos },
      effects: EditorView.scrollIntoView(pos, { y: "center" }),
      userEvent: "select",
    });
    this.view.focus();
  }

  /** Quick Documentation for the symbol at the caret, below it. */
  async showQuickDoc(): Promise<void> {
    const view = this.view;
    const pos = view.state.selection.main.head;
    const problems = diagnosticsAt(view.state.field(diagnosticsField), pos);
    let html: string | null = null;
    try {
      html = (await this.doc.hover(this, pos))?.html ?? null;
    } catch (error: unknown) {
      this.host.notify(errorMessage(error));
    }
    if (view.state.selection.main.head !== pos) return;
    if (html === null && problems.length === 0) {
      this.host.notify("No documentation found.");
      return;
    }
    view.dispatch({
      effects: setQuickDoc.of({
        pos,
        above: false,
        create: () => {
          const dom = insightDom(problems, html);
          dom.classList.add("quick-doc");
          return { dom };
        },
      }),
    });
  }

  /** Jumps between a bracket and its match (from the syntax tree). */
  async gotoMatchingBrace(): Promise<void> {
    const info = await this.doc.caretInfoNow(this);
    const pair = info?.brackets;
    if (!pair) return;
    const head = this.view.state.selection.main.head;
    const atOpen = head >= pair.openFrom && head <= pair.openTo;
    this.revealOffset(atOpen ? pair.closeFrom : pair.openFrom);
  }

  runAction(action: KeyAction): void {
    const view = this.view;
    switch (action) {
      case "save":
        void this.doc.save();
        return;
      case "undo":
        void this.doc.undo(this);
        return;
      case "redo":
        void this.doc.redo(this);
        return;
      case "addCaretAbove":
        void this.doc.addCaret(this, "up");
        return;
      case "addCaretBelow":
        void this.doc.addCaret(this, "down");
        return;
      case "selectNextOccurrence":
        void this.doc.selectNextOccurrence(this);
        return;
      case "selectAllOccurrences":
        void this.doc.selectAllOccurrences(this);
        return;
      case "closeTab":
        this.host.closePane(this);
        return;
      case "foldCollapse":
        foldCode(view);
        return;
      case "foldExpand":
        unfoldCode(view);
        return;
      case "foldCollapseAll":
        foldAll(view);
        return;
      case "foldExpandAll":
        unfoldAll(view);
        return;
      case "gotoMatchingBrace":
        void this.gotoMatchingBrace();
        return;
      case "quickDocumentation":
        void this.showQuickDoc();
        return;
      case "gotoDeclaration":
        this.host.navigate(this, "definition", null);
        return;
      case "gotoTypeDeclaration":
        this.host.navigate(this, "typeDefinition", null);
        return;
      case "gotoImplementation":
        this.host.navigate(this, "implementation", null);
        return;
      default:
        return;
    }
  }

  // Setup

  private newLine(): boolean {
    this.deferredKeys ??= [];
    void this.doc.newLine(this);
    return true;
  }

  private createState(text: string): EditorState {
    const doc = this.doc;
    const openers = doc.autoClose.map((pair) => pair.charAt(0));
    return EditorState.create({
      doc: text,
      extensions: [
        lineNumbers(),
        // Rebuild the markers when fresh ranges arrive from Rust (not only on
        // document or viewport changes).
        foldGutter({
          foldingChanged: (update) =>
            update.transactions.some((tr) => tr.effects.some((e) => e.is(setFoldRanges))),
        }),
        codeFolding(),
        highlightActiveLineGutter(),
        highlightActiveLine(),
        drawSelection(),
        dropCursor(),
        EditorState.allowMultipleSelections.of(true),
        EditorState.tabSize.of(doc.tabWidth),
        EditorState.languageData.of(() => [{ closeBrackets: { brackets: openers } }]),
        closeBrackets(),
        Prec.highest(this.domHandlers()),
        keymap.of([
          ...this.actionKeys(),
          { key: "Enter", run: () => this.newLine() },
          { key: "Shift-Enter", run: () => this.newLine() },
          { key: "Tab", run: insertIndent(doc.indentUnit) },
          { key: "Escape", run: closeQuickDoc },
          { key: "Escape", run: simplifySelection },
          ...closeBracketsKeymap,
          ...standardKeymap,
        ]),
        highlightField,
        diagnosticsField,
        quickDocField,
        hoverTooltip(
          async (view, pos) => {
            const problems = diagnosticsAt(view.state.field(diagnosticsField), pos);
            let html: string | null = null;
            try {
              html = (await this.doc.hover(this, pos))?.html ?? null;
            } catch {
              // Hover is best effort; the server may be starting or gone.
            }
            if (html === null && problems.length === 0) return null;
            return { pos, above: true, create: () => ({ dom: insightDom(problems, html) }) };
          },
          { hoverTime: HOVER_DELAY_MS },
        ),
        foldRangesField,
        rustFolds,
        bracketField,
        EditorView.contentAttributes.of({ "aria-label": `Editor for ${doc.path}` }),
        EditorView.updateListener.of((update) => {
          if (update.viewportChanged) doc.scheduleHighlights();
          if (update.selectionSet || update.docChanged) {
            this.host.onCursor(this);
            doc.scheduleCaretInfo(this);
          }
        }),
      ],
    });
  }

  private actionKeys(): CmKeyBinding[] {
    return this.bindings.flatMap((binding): CmKeyBinding[] =>
      binding.stroke.type === "chord" && EDITOR_ACTIONS.has(binding.action)
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

  private domHandlers(): Extension {
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
      // CodeMirror treats Shift+Click as "extend selection".
      mousedown: (event, view) => {
        // Ctrl+Click (macOS: Cmd+Click) goes to the declaration.
        const navigateKey = this.host.mac ? event.metaKey : event.ctrlKey;
        if (event.button === 0 && navigateKey && !event.altKey && !event.shiftKey) {
          const pos = view.posAtCoords({ x: event.clientX, y: event.clientY });
          if (pos === null) return false;
          event.preventDefault();
          view.dispatch({ selection: { anchor: pos }, userEvent: "select.pointer" });
          this.host.navigate(this, "definition", pos);
          return true;
        }
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
        // While an Enter is in flight, hold every key back (see newLine).
        if (this.deferredKeys !== null && !event.isComposing) {
          if (!["Shift", "Control", "Alt", "Meta"].includes(event.key)) {
            this.deferredKeys.push(event);
          }
          event.preventDefault();
          return true;
        }
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

  private renderCrumbs(crumbs: CaretInfo["breadcrumbs"]): void {
    const items: Node[] = [];
    crumbs.forEach((crumb, index) => {
      if (index > 0) {
        const sep = document.createElement("span");
        sep.className = "crumb-sep";
        sep.textContent = "›";
        items.push(sep);
      }
      const button = document.createElement("button");
      button.type = "button";
      button.className = "crumb";
      button.dataset.testid = "crumb";
      button.title = crumb.kind;
      button.textContent = crumb.label;
      button.addEventListener("click", () => {
        this.revealOffset(crumb.offset);
      });
      items.push(button);
    });
    this.crumbsBar.replaceChildren(...items);
  }
}

/** Escape closes Quick Documentation first. */
const closeQuickDoc: Command = (view) => {
  if (!view.state.field(quickDocField, false)) return false;
  view.dispatch({ effects: setQuickDoc.of(null) });
  return true;
};

/** Tab inserts the document's indentation unit at every caret. */
function insertIndent(unit: string): Command {
  return (view) => {
    view.dispatch(
      view.state.update(view.state.replaceSelection(unit), {
        scrollIntoView: true,
        userEvent: "input.type",
      }),
    );
    return true;
  };
}
