import { describe, expect, it } from "vitest";

import { chordMatches, formatChord } from "./keys";

const ev = (
  code: string,
  key: string,
  mods: Partial<Record<"ctrlKey" | "altKey" | "shiftKey" | "metaKey", boolean>> = {},
) => ({
  code,
  key,
  ctrlKey: false,
  altKey: false,
  shiftKey: false,
  metaKey: false,
  ...mods,
});

describe("chordMatches", () => {
  it("maps Mod to Ctrl or Cmd", () => {
    const e = ev("KeyN", "N", { ctrlKey: true, shiftKey: true });
    expect(chordMatches(e, "Mod-Shift-n", false)).toBe(true);
    expect(chordMatches(e, "Mod-Shift-n", true)).toBe(false);
    expect(
      chordMatches(ev("KeyN", "N", { metaKey: true, shiftKey: true }), "Mod-Shift-n", true),
    ).toBe(true);
  });

  it("requires exact modifiers and uses the physical key", () => {
    expect(chordMatches(ev("KeyN", "n", { ctrlKey: true }), "Mod-Shift-n", false)).toBe(false);
    // Option+O on macOS produces "ø" but the code is still KeyO.
    expect(chordMatches(ev("KeyO", "ø", { metaKey: true, altKey: true }), "Mod-Alt-o", true)).toBe(
      true,
    );
    expect(chordMatches(ev("F4", "F4", { ctrlKey: true }), "Ctrl-F4", false)).toBe(true);
  });

  it("formats shortcuts per platform", () => {
    expect(formatChord("Mod-Shift-n", false)).toBe("Ctrl+Shift+N");
    expect(formatChord("Mod-Shift-o", true)).toBe("⇧⌘O");
    expect(formatChord("Ctrl-F4", false)).toBe("Ctrl+F4");
  });
});
