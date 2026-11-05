// Which keys go to the terminal and how: plain text arrives through the
// hidden textarea's input event (so IME and dead keys work), other keys
// are sent by name and encoded in Rust.
import type { TerminalKey } from "../ipc/bindings";
import { eventKeyName } from "../ui/keys";

export type KeyRoute =
  /** Not for the terminal (lone modifiers, app shortcuts on macOS). */
  | { kind: "ignore" }
  | { kind: "copy" }
  /** New terminal tab (Ctrl+Shift+T, macOS Cmd+T). */
  | { kind: "newTab" }
  /** Let the browser fire its paste event (handled by the view). */
  | { kind: "paste" }
  /** Let the textarea receive the character. */
  | { kind: "text" }
  | { kind: "key"; key: TerminalKey };

type KeyInfo = Pick<
  KeyboardEvent,
  "key" | "code" | "ctrlKey" | "altKey" | "shiftKey" | "metaKey" | "isComposing"
>;

const MODIFIERS = new Set(["Shift", "Control", "Alt", "Meta", "AltGraph", "CapsLock"]);

export function routeKey(event: KeyInfo, mac: boolean): KeyRoute {
  if (event.isComposing || MODIFIERS.has(event.key)) return { kind: "ignore" };
  const name = eventKeyName(event);
  if (mac && event.metaKey) {
    if (name === "c") return { kind: "copy" };
    if (name === "v") return { kind: "paste" };
    if (name === "t" && !event.shiftKey) return { kind: "newTab" };
    return { kind: "ignore" };
  }
  if (!mac) {
    if (event.ctrlKey && event.shiftKey && name === "c") return { kind: "copy" };
    if (event.ctrlKey && event.shiftKey && name === "v") return { kind: "paste" };
    if (event.ctrlKey && event.shiftKey && name === "t") return { kind: "newTab" };
    if (event.ctrlKey && !event.shiftKey && event.key === "Insert") return { kind: "copy" };
    if (event.shiftKey && !event.ctrlKey && event.key === "Insert") return { kind: "paste" };
  }
  const named = event.key.length > 1;
  if (!named && !event.ctrlKey && !event.altKey) return { kind: "text" };
  // With Ctrl or Alt, use the physical letter: Option and AltGr change `key`.
  let key = event.key;
  if (!named && (event.code.startsWith("Key") || event.code.startsWith("Digit"))) {
    key = event.shiftKey && !event.ctrlKey ? name.toUpperCase() : name;
  }
  return {
    kind: "key",
    key: { key, ctrl: event.ctrlKey, alt: event.altKey, shift: event.shiftKey },
  };
}

export type InputItem =
  | { kind: "text"; text: string }
  | { kind: "paste"; text: string }
  | { kind: "key"; key: TerminalKey };

/**
 * Sends input in order, one call at a time: IPC calls may otherwise be
 * handled out of order. Text typed while a call is in flight is merged
 * into one call.
 */
export class InputQueue {
  private readonly items: InputItem[] = [];
  private running = false;

  constructor(private readonly send: (item: InputItem) => Promise<void>) {}

  push(item: InputItem): void {
    const last = this.items.at(-1);
    if (item.kind === "text" && last?.kind === "text") last.text += item.text;
    else this.items.push(item);
    void this.run();
  }

  private async run(): Promise<void> {
    if (this.running) return;
    this.running = true;
    try {
      for (let item = this.items.shift(); item; item = this.items.shift()) {
        await this.send(item);
      }
    } finally {
      this.running = false;
    }
  }
}
