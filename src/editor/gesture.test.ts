import { describe, expect, it } from "vitest";

import { DOUBLE_TAP_MS, DoubleModifierGesture } from "./gesture";

describe("DoubleModifierGesture", () => {
  it("arms after tap + press-and-hold", () => {
    const g = new DoubleModifierGesture();
    g.keydown("Control", 0);
    g.keyup("Control", 50);
    g.keydown("Control", 150);
    expect(g.keydown("ArrowDown", 200)).toBe("Control");
    expect(g.keydown("ArrowDown", 250)).toBe("Control"); // repeat while held
    g.keyup("Control", 300);
    expect(g.keydown("ArrowDown", 350)).toBeNull();
  });

  it("does not arm when the second press is too late", () => {
    const g = new DoubleModifierGesture();
    g.keydown("Control", 0);
    g.keyup("Control", 10);
    g.keydown("Control", 10 + DOUBLE_TAP_MS + 1);
    expect(g.keydown("ArrowUp", 600)).toBeNull();
  });

  it("ignores modifiers used in chords", () => {
    const g = new DoubleModifierGesture();
    g.keydown("Control", 0);
    g.keydown("c", 10); // Ctrl+C
    g.keyup("Control", 20);
    g.keydown("Control", 100);
    expect(g.keydown("ArrowUp", 120)).toBeNull();
  });

  it("tracks modifiers independently", () => {
    const g = new DoubleModifierGesture();
    g.keydown("Alt", 0);
    g.keyup("Alt", 10);
    g.keydown("Control", 50);
    expect(g.keydown("ArrowUp", 60)).toBeNull();
  });
});
