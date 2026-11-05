import { describe, expect, it } from "vitest";

import { TERMINAL_FLAGS, type TerminalFrame, type TerminalRun } from "../ipc/bindings";
import { cssColor, runStyle, ScreenModel } from "./screen";

const run = (text: string, flags = 0, fg = 0, bg = 0): TerminalRun => ({ text, fg, bg, flags });

const frame = (over: Partial<TerminalFrame>): TerminalFrame => ({
  id: 1,
  seq: 1,
  size: { cols: 10, rows: 3 },
  full: true,
  rows: [],
  cursor: { row: 0, col: 0 },
  displayOffset: 0,
  history: 0,
  title: "",
  ...over,
});

describe("cssColor", () => {
  it("decodes default, theme and 24-bit colors", () => {
    expect(cssColor(0)).toBeNull();
    expect(cssColor(1)).toBe("var(--fr-term-0)");
    expect(cssColor(16)).toBe("var(--fr-term-15)");
    expect(cssColor(0x01_00_0a_ff)).toBe("#000aff");
  });
});

describe("runStyle", () => {
  it("swaps colors for inverse and marks the selection", () => {
    expect(runStyle(run("x", TERMINAL_FLAGS.inverse, 2))).toEqual({
      color: "var(--fr-term-bg)",
      background: "var(--fr-term-1)",
      classes: [],
    });
    expect(runStyle(run("x", TERMINAL_FLAGS.selected | TERMINAL_FLAGS.bold))).toEqual({
      color: "var(--fr-term-fg)",
      background: "var(--fr-term-selection)",
      classes: ["t-bold"],
    });
    expect(runStyle(run("x", TERMINAL_FLAGS.hidden)).color).toBe("transparent");
  });
});

describe("ScreenModel", () => {
  it("replaces all rows on a full frame and only the changed ones otherwise", () => {
    const model = new ScreenModel();
    expect(model.apply(frame({ rows: [{ row: 0, runs: [run("$ ")] }] }))).toEqual([0, 1, 2]);
    expect(model.rowText(0)).toBe("$ ");
    const changed = model.apply(
      frame({ seq: 2, full: false, rows: [{ row: 1, runs: [run("ok")] }], title: "zsh" }),
    );
    expect(changed).toEqual([1]);
    expect(model.rowText(0)).toBe("$ ");
    expect(model.rowText(1)).toBe("ok");
    expect(model.title).toBe("zsh");
  });

  it("starts over when the size changes", () => {
    const model = new ScreenModel();
    model.apply(frame({ rows: [{ row: 0, runs: [run("a")] }] }));
    const changed = model.apply(frame({ full: false, size: { cols: 10, rows: 2 }, rows: [] }));
    expect(changed).toEqual([0, 1]);
    expect(model.rows).toHaveLength(2);
  });
});
