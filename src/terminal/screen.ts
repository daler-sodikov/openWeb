// The view's copy of a terminal screen: rows of styled runs updated from
// frames, and how a run is drawn. Screen state itself lives in Rust.
import { TERMINAL_FLAGS, type TerminalFrame, type TerminalRun } from "../ipc/bindings";

/** CSS color of a run color (see `TerminalRun` in the bindings), or null for the default. */
export function cssColor(color: number): string | null {
  if (color === 0) return null;
  if (color <= 16) return `var(--fr-term-${String(color - 1)})`;
  return `#${(color & 0xffffff).toString(16).padStart(6, "0")}`;
}

export interface RunStyle {
  color: string | null;
  background: string | null;
  classes: string[];
}

const CLASSES: [keyof typeof TERMINAL_FLAGS, string][] = [
  ["bold", "t-bold"],
  ["dim", "t-dim"],
  ["italic", "t-italic"],
  ["underline", "t-underline"],
  ["strike", "t-strike"],
  ["wide", "t-wide"],
];

export function runStyle(run: TerminalRun): RunStyle {
  const has = (flag: keyof typeof TERMINAL_FLAGS) => (run.flags & TERMINAL_FLAGS[flag]) !== 0;
  let color = cssColor(run.fg);
  let background = cssColor(run.bg);
  if (has("inverse")) {
    [color, background] = [background ?? "var(--fr-term-bg)", color ?? "var(--fr-term-fg)"];
  }
  if (has("selected")) {
    background = "var(--fr-term-selection)";
    color ??= "var(--fr-term-fg)";
  }
  if (has("hidden")) color = "transparent";
  return {
    color,
    background,
    classes: CLASSES.filter(([flag]) => has(flag)).map(([, cls]) => cls),
  };
}

export class ScreenModel {
  rows: TerminalRun[][] = [];
  cols = 0;
  cursor: TerminalFrame["cursor"] = null;
  displayOffset = 0;
  history = 0;
  title = "";

  /** Applies a frame; returns the rows to redraw (all of them for a full frame). */
  apply(frame: TerminalFrame): number[] {
    const resized = frame.size.rows !== this.rows.length || frame.size.cols !== this.cols;
    this.cols = frame.size.cols;
    if (frame.full || resized) {
      this.rows = Array.from({ length: frame.size.rows }, () => []);
    }
    for (const row of frame.rows) {
      if (row.row < this.rows.length) this.rows[row.row] = row.runs;
    }
    this.cursor = frame.cursor;
    this.displayOffset = frame.displayOffset;
    this.history = frame.history;
    this.title = frame.title;
    return frame.full || resized ? this.rows.map((_, i) => i) : frame.rows.map((r) => r.row);
  }

  /** Text of a row (one character per cell). */
  rowText(row: number): string {
    return (this.rows[row] ?? []).map((r) => r.text).join("");
  }
}
