import { describe, expect, it } from "vitest";

import { type ClipboardLike, copyWhenReady, OrderedCalls } from "./selection";

/** A call that finishes when `release` is called, logging start and end. */
function deferred(log: string[], name: string) {
  let release: () => void = () => undefined;
  const call = () => {
    log.push(`start ${name}`);
    return new Promise<string>((resolve) => {
      release = () => {
        log.push(`end ${name}`);
        resolve(name);
      };
    });
  };
  return {
    call,
    release: () => {
      release();
    },
  };
}

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

describe("OrderedCalls", () => {
  it("runs a later call only after the earlier one finished", async () => {
    const log: string[] = [];
    const calls = new OrderedCalls();
    const clear = deferred(log, "clear");
    const select = deferred(log, "select");
    void calls.run(clear.call);
    calls.replace(select.call);
    const text = calls.run(() => Promise.resolve("text"));
    await tick();
    expect(log).toEqual(["start clear"]);
    clear.release();
    await tick();
    expect(log).toEqual(["start clear", "end clear", "start select"]);
    select.release();
    expect(await text).toBe("text");
  });

  it("keeps only the newest waiting drag step, never ahead of a later call", async () => {
    const log: string[] = [];
    const calls = new OrderedCalls();
    const first = deferred(log, "first");
    void calls.run(first.call);
    calls.replace(() => {
      log.push("drag 1");
      return Promise.resolve();
    });
    calls.replace(() => {
      log.push("drag 2");
      return Promise.resolve();
    });
    void calls.run(() => {
      log.push("clear");
      return Promise.resolve();
    });
    calls.replace(() => {
      log.push("drag 3");
      return Promise.resolve();
    });
    await tick();
    first.release();
    await tick();
    await tick();
    expect(log).toEqual(["start first", "end first", "drag 2", "clear", "drag 3"]);
  });

  it("goes on after a failed call", async () => {
    const calls = new OrderedCalls();
    const failed = calls.run(() => Promise.reject(new Error("gone")));
    await expect(failed).rejects.toThrow("gone");
    expect(await calls.run(() => Promise.resolve(1))).toBe(1);
  });
});

class FakeItem {
  constructor(readonly items: Record<string, Promise<Blob>>) {}
}

function fakeClipboard() {
  const state = { text: "old" };
  const clipboard: ClipboardLike = {
    async write(items) {
      const item = items[0] as unknown as FakeItem;
      const blob = await item.items["text/plain"];
      if (!blob) throw new Error("no text");
      state.text = await blob.text();
    },
    writeText(text) {
      state.text = text;
      return Promise.resolve();
    },
  };
  return { clipboard, state };
}

describe("copyWhenReady", () => {
  it("calls the clipboard at once and writes the text when it arrives", async () => {
    const { clipboard, state } = fakeClipboard();
    let resolve: (text: string) => void = () => undefined;
    const text = new Promise<string | null>((r) => {
      resolve = r;
    });
    let called = false;
    const spy: ClipboardLike = {
      write: (items) => {
        called = true;
        return clipboard.write(items);
      },
      writeText: (t) => clipboard.writeText(t),
    };
    const done = copyWhenReady(text, spy, FakeItem as never);
    // Synchronously, inside the (simulated) key press.
    expect(called).toBe(true);
    resolve("copy-me");
    expect(await done).toBe(true);
    expect(state.text).toBe("copy-me");
  });

  it("leaves the clipboard alone when nothing is selected", async () => {
    const { clipboard, state } = fakeClipboard();
    expect(await copyWhenReady(Promise.resolve(null), clipboard, FakeItem as never)).toBe(false);
    expect(state.text).toBe("old");
  });

  it("falls back to writeText without ClipboardItem", async () => {
    const { clipboard, state } = fakeClipboard();
    expect(await copyWhenReady(Promise.resolve("plain"), clipboard, null)).toBe(true);
    expect(state.text).toBe("plain");
  });
});
