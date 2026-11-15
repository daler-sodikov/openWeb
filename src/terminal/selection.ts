// Selection commands and copying for a terminal view.
//
// Each IPC call is handled on its own (a Tauri async task, or a request
// thread of the test backend), so calls made one after another can
// overtake each other: the "clear selection" of a double click's first
// click could land after its "select word" and leave nothing selected.
// OrderedCalls runs them one at a time, in the order they were made.

/** Runs calls one at a time in order; a drag keeps only its newest step. */
export class OrderedCalls {
  private tail: Promise<unknown> = Promise.resolve();
  /** A `replace` call still waiting at the end of the queue. */
  private waiting: { call: () => Promise<unknown> } | null = null;

  /** Runs `call` after every call made before it. */
  run<T>(call: () => Promise<T>): Promise<T> {
    // Anything made later must not be overtaken by a replaced call.
    this.waiting = null;
    return this.enqueue(call);
  }

  /**
   * Like `run`, but if the previous call was also a `replace` and has not
   * started yet, this one takes its place (selection updates while
   * dragging: only the newest matters).
   */
  replace(call: () => Promise<unknown>): void {
    if (this.waiting) {
      this.waiting.call = call;
      return;
    }
    const slot = { call };
    this.waiting = slot;
    void this.enqueue(() => {
      if (this.waiting === slot) this.waiting = null;
      return slot.call();
    });
  }

  private enqueue<T>(call: () => Promise<T>): Promise<T> {
    const next = this.tail.then(call);
    this.tail = next.catch(() => undefined);
    return next;
  }
}

/** The parts of `navigator.clipboard` used here. */
export interface ClipboardLike {
  write(items: ClipboardItem[]): Promise<void>;
  writeText(text: string): Promise<void>;
}

type ItemConstructor = new (items: Record<string, Promise<Blob>>) => ClipboardItem;

/**
 * Puts text that is still being fetched on the clipboard; resolves false
 * if there was none (nothing selected), leaving the clipboard as it was.
 *
 * The clipboard call is made at once, inside the key press: WebKit (the
 * app's web view on Linux and macOS) allows clipboard writes only during a
 * user gesture, and that is over once an IPC call has been awaited.
 * Chromium is more lenient, so an `await` first works there but not in
 * the app. A ClipboardItem may hold a promise, so the text can follow.
 */
export async function copyWhenReady(
  text: Promise<string | null>,
  clipboard: ClipboardLike = navigator.clipboard,
  Item: ItemConstructor | null = "ClipboardItem" in globalThis ? ClipboardItem : null,
): Promise<boolean> {
  if (Item) {
    const blob = text.then((t) => {
      if (!t) throw new Error("nothing selected");
      return new Blob([t], { type: "text/plain" });
    });
    // Called before any await: still inside the gesture.
    const written = clipboard.write([new Item({ "text/plain": blob })]);
    try {
      await written;
      return true;
    } catch (error: unknown) {
      // Nothing selected: the item had no text, the clipboard is unchanged.
      if (!(await text.catch(() => null))) return false;
      throw error;
    }
  }
  const value = await text;
  if (!value) return false;
  await clipboard.writeText(value);
  return true;
}
