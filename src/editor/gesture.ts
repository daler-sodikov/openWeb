// IntelliJ-style "double-tap a modifier, keep holding, press a key" gesture
// (used for clone caret above/below). Pure input handling for the view.

/** Maximum delay between releasing the modifier and pressing it again. */
export const DOUBLE_TAP_MS = 400;

export type GestureModifier = "Control" | "Alt";

export class DoubleModifierGesture {
  private lastRelease = new Map<string, number>();
  private armed: string | null = null;
  private dirty = false;

  /** Feed every keydown. Returns true if `key` completes an armed gesture for `modifier`. */
  keydown(key: string, now: number): string | null {
    if (key === "Control" || key === "Alt") {
      const released = this.lastRelease.get(key);
      this.armed = released !== undefined && now - released <= DOUBLE_TAP_MS ? key : null;
      this.dirty = false;
      return null;
    }
    this.dirty = true;
    return this.armed;
  }

  /** Feed every keyup. */
  keyup(key: string, now: number): void {
    if (key === "Control" || key === "Alt") {
      // A modifier used in a chord (e.g. Ctrl+C) does not count as a tap.
      if (this.dirty) this.lastRelease.delete(key);
      else this.lastRelease.set(key, now);
      if (this.armed === key) this.armed = null;
      this.dirty = false;
    }
  }
}

/**
 * Detects a modifier tapped twice (pressed and released with no other key in
 * between, the second press within {@link DOUBLE_TAP_MS} of the first
 * release). Fires on the second release, so Shift used for typing capitals
 * never triggers it.
 */
export class DoubleTapDetector {
  private lastTap = 0;
  private pressed = false;
  private clean = false;

  constructor(private readonly modifier: string) {}

  keydown(key: string): void {
    if (key === this.modifier) {
      this.pressed = true;
      this.clean = true;
    } else {
      this.clean = false;
      this.lastTap = 0;
    }
  }

  /** Returns true when this release completes a double tap. */
  keyup(key: string, now: number): boolean {
    if (key !== this.modifier || !this.pressed) return false;
    this.pressed = false;
    if (!this.clean) {
      this.lastTap = 0;
      return false;
    }
    if (this.lastTap !== 0 && now - this.lastTap <= DOUBLE_TAP_MS) {
      this.lastTap = 0;
      return true;
    }
    this.lastTap = now;
    return false;
  }
}
