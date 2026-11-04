// Matching key chords (CodeMirror notation, e.g. "Mod-Shift-n") against
// keyboard events, for shortcuts that work outside the editor.

export interface ChordModifiers {
  ctrl: boolean;
  alt: boolean;
  shift: boolean;
  meta: boolean;
}

/** Parses a chord into modifiers and a lowercase key name. */
export function parseChord(chord: string, mac: boolean): { mods: ChordModifiers; key: string } {
  const parts = chord.split("-");
  const key = (parts.pop() ?? "").toLowerCase();
  const mods: ChordModifiers = { ctrl: false, alt: false, shift: false, meta: false };
  for (const part of parts) {
    switch (part) {
      case "Mod":
        if (mac) mods.meta = true;
        else mods.ctrl = true;
        break;
      case "Ctrl":
        mods.ctrl = true;
        break;
      case "Alt":
        mods.alt = true;
        break;
      case "Shift":
        mods.shift = true;
        break;
      case "Meta":
      case "Cmd":
        mods.meta = true;
        break;
      default:
        break;
    }
  }
  return { mods, key };
}

/**
 * Key name of an event: letters and digits come from the physical key
 * (`event.code`), because Option on macOS and Shift change `event.key`.
 */
export function eventKeyName(event: Pick<KeyboardEvent, "code" | "key">): string {
  if (event.code.startsWith("Key")) return event.code.slice(3).toLowerCase();
  if (event.code.startsWith("Digit")) return event.code.slice(5);
  return event.key.toLowerCase();
}

export function chordMatches(
  event: Pick<KeyboardEvent, "code" | "key" | "ctrlKey" | "altKey" | "shiftKey" | "metaKey">,
  chord: string,
  mac: boolean,
): boolean {
  const { mods, key } = parseChord(chord, mac);
  return (
    event.ctrlKey === mods.ctrl &&
    event.altKey === mods.alt &&
    event.shiftKey === mods.shift &&
    event.metaKey === mods.meta &&
    eventKeyName(event) === key
  );
}

/** Human-readable shortcut (for menus and Search Everywhere). */
export function formatChord(chord: string, mac: boolean): string {
  const { mods, key } = parseChord(chord, mac);
  const names: string[] = [];
  if (mods.ctrl) names.push(mac ? "⌃" : "Ctrl");
  if (mods.alt) names.push(mac ? "⌥" : "Alt");
  if (mods.shift) names.push(mac ? "⇧" : "Shift");
  if (mods.meta) names.push(mac ? "⌘" : "Meta");
  names.push(key.length === 1 ? key.toUpperCase() : key.charAt(0).toUpperCase() + key.slice(1));
  return names.join(mac ? "" : "+");
}
