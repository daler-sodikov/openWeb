import { describe, expect, it } from "vitest";

import { DOUBLE_TAP_MS, DoubleModifierGesture, DoubleTapDetector } from "./gesture";

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

describe("DoubleTapDetector", () => {
  it("fires on the second clean release", () => {
    const d = new DoubleTapDetector("Shift");
    d.keydown("Shift");
    expect(d.keyup("Shift", 100)).toBe(false);
    d.keydown("Shift");
    expect(d.keyup("Shift", 300)).toBe(true);
    // A third tap starts over.
    d.keydown("Shift");
    expect(d.keyup("Shift", 400)).toBe(false);
  });

  it("ignores Shift used for typing and slow taps", () => {
    const d = new DoubleTapDetector("Shift");
    d.keydown("Shift");
    d.keydown("A");
    expect(d.keyup("Shift", 100)).toBe(false);
    d.keydown("Shift");
    expect(d.keyup("Shift", 200)).toBe(false);
    d.keydown("Shift");
    expect(d.keyup("Shift", 200 + DOUBLE_TAP_MS + 1)).toBe(false);
  });

  it("is reset by another key between taps", () => {
    const d = new DoubleTapDetector("Shift");
    d.keydown("Shift");
    d.keyup("Shift", 100);
    d.keydown("x");
    d.keydown("Shift");
    expect(d.keyup("Shift", 200)).toBe(false);
  });
});
