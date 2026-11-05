// Editor theme: tells CodeMirror whether the UI is dark (so its base styles
// for selection, tooltips and the completion list use the dark variants)
// and styles those parts through the CSS variables in styles.css. Colors
// live only in styles.css; this file maps CodeMirror's parts to them.
import type { autocompletion } from "@codemirror/autocomplete";
import { Compartment, type Extension } from "@codemirror/state";
import { EditorView } from "@codemirror/view";

const darkQuery = window.matchMedia("(prefers-color-scheme: dark)");

/** Whether the OS (and so the UI) uses the dark theme. */
export function prefersDark(): boolean {
  return darkQuery.matches;
}

/** Calls `listener` when the OS switches between light and dark. */
export function onColorSchemeChange(listener: () => void): () => void {
  darkQuery.addEventListener("change", listener);
  return () => {
    darkQuery.removeEventListener("change", listener);
  };
}

/** Holds the dark/light flag so it can follow the OS setting. */
export const colorScheme = new Compartment();

export function colorSchemeFlag(dark: boolean): Extension {
  return EditorView.darkTheme.of(dark);
}

const popup = {
  backgroundColor: "var(--fr-popup-bg)",
  color: "var(--fr-text)",
  border: "1px solid var(--fr-popup-border)",
};

/** Rows of the completion list are whole pixels high so it never shows a cut row. */
const ROW = 22;
const ROWS = 10;

export const editorChrome: Extension = EditorView.theme({
  // Selection: unfocused first, then focused (CodeMirror's own selectors).
  ".cm-selectionLayer .cm-selectionBackground": {
    background: "var(--fr-selection-inactive)",
  },
  "&.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground": {
    background: "var(--fr-selection)",
  },
  ".cm-content ::selection": { backgroundColor: "var(--fr-selection)" },
  ".cm-selectionMatch": { backgroundColor: "var(--fr-selection-match)" },

  // Every tooltip (completion, info, signature help, hover).
  ".cm-tooltip": { ...popup, borderRadius: "4px" },

  // Completion list.
  ".cm-tooltip.cm-tooltip-autocomplete": { padding: "2px 0" },
  ".cm-tooltip.cm-tooltip-autocomplete > ul": {
    fontFamily: "var(--fr-font-code)",
    maxHeight: `${String(ROW * ROWS)}px`,
    minWidth: "240px",
  },
  ".cm-tooltip.cm-tooltip-autocomplete > ul > li": {
    color: "var(--fr-text)",
    height: `${String(ROW)}px`,
    lineHeight: `${String(ROW)}px`,
    padding: "0 8px 0 4px",
    boxSizing: "border-box",
  },
  ".cm-tooltip.cm-tooltip-autocomplete > ul > li:hover": {
    backgroundColor: "var(--fr-hover-bg)",
  },
  ".cm-tooltip.cm-tooltip-autocomplete > ul > li[aria-selected]": {
    backgroundColor: "var(--fr-selection)",
    color: "var(--fr-text)",
  },
  ".cm-completionDetail": { color: "var(--fr-text-muted)", marginLeft: "1em" },
  // Muted text cannot reach 4.5:1 on the selected row; use the text color.
  "li[aria-selected] .cm-completionDetail": { color: "var(--fr-text)" },
  ".cm-completionIcon": { color: "var(--fr-text-muted)", opacity: "1" },
  "li[aria-selected] .cm-completionIcon": { color: "var(--fr-text)" },
  ".cm-completionMatchedText": {
    textDecoration: "none",
    color: "var(--fr-link)",
    fontWeight: "600",
  },
  "li[aria-selected] .cm-completionMatchedText": { color: "inherit" },

  // Documentation panel next to the list: stays inside the window.
  ".cm-tooltip.cm-completionInfo": {
    ...popup,
    padding: "6px 10px",
    // Width and height limits are set per position (positionCompletionInfo).
    overflowY: "auto",
  },
});

/** Largest size of the completion documentation panel (also in the theme). */
const INFO_WIDTH = 420;
const INFO_HEIGHT = 320;
const INFO_GAP = 4;

/**
 * Places the completion documentation panel next to the list and inside the
 * window. Its content loads after it opens and can grow, so it is placed as
 * if it already had its largest height, and its height is capped to the room
 * below it (it scrolls inside rather than leaving the window).
 */
type PositionInfo = NonNullable<NonNullable<Parameters<typeof autocompletion>[0]>["positionInfo"]>;

export const positionCompletionInfo: PositionInfo = (_view, list, option, _info, space) => {
  const roomRight = space.right - list.right;
  const roomLeft = list.left - space.left;
  const left = roomRight < INFO_WIDTH && roomLeft > roomRight;
  const width = Math.max(0, Math.min(INFO_WIDTH, (left ? roomLeft : roomRight) - INFO_GAP));
  const top = Math.max(space.top, Math.min(option.top, space.bottom - INFO_HEIGHT));
  const height = Math.max(0, Math.min(INFO_HEIGHT, space.bottom - top - INFO_GAP));
  return {
    style: `top: ${String(top - list.top)}px; max-width: ${String(width)}px; max-height: ${String(height)}px`,
    class: left ? "cm-completionInfo-left" : "cm-completionInfo-right",
  };
};
