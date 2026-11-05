import { describe, expect, it } from "vitest";

import { type InputItem, InputQueue, routeKey } from "./input";

const ev = (key: string, code: string, mods: Partial<KeyboardEvent> = {}) => ({
  key,
  code,
  ctrlKey: false,
  altKey: false,
  shiftKey: false,
  metaKey: false,
  isComposing: false,
  ...mods,
});

describe("routeKey", () => {
  it("lets printable characters through the textarea", () => {
    expect(routeKey(ev("a", "KeyA"), false)).toEqual({ kind: "text" });
    expect(routeKey(ev("A", "KeyA", { shiftKey: true }), false)).toEqual({ kind: "text" });
    expect(routeKey(ev("é", "KeyE", { isComposing: true }), false)).toEqual({ kind: "ignore" });
  });

  it("sends named keys and control combinations by name", () => {
    expect(routeKey(ev("Enter", "Enter"), false)).toEqual({
      kind: "key",
      key: { key: "Enter", ctrl: false, alt: false, shift: false },
    });
    expect(routeKey(ev("c", "KeyC", { ctrlKey: true }), false)).toEqual({
      kind: "key",
      key: { key: "c", ctrl: true, alt: false, shift: false },
    });
    // Option+B on macOS types "∫"; the terminal gets Alt+b.
    expect(routeKey(ev("∫", "KeyB", { altKey: true }), true)).toEqual({
      kind: "key",
      key: { key: "b", ctrl: false, alt: true, shift: false },
    });
    expect(routeKey(ev("ArrowUp", "ArrowUp", { shiftKey: true }), false)).toMatchObject({
      key: { key: "ArrowUp", shift: true },
    });
  });

  it("maps copy and paste shortcuts per OS", () => {
    expect(routeKey(ev("C", "KeyC", { ctrlKey: true, shiftKey: true }), false)).toEqual({
      kind: "copy",
    });
    expect(routeKey(ev("V", "KeyV", { ctrlKey: true, shiftKey: true }), false)).toEqual({
      kind: "paste",
    });
    expect(routeKey(ev("Insert", "Insert", { shiftKey: true }), false)).toEqual({
      kind: "paste",
    });
    expect(routeKey(ev("c", "KeyC", { metaKey: true }), true)).toEqual({ kind: "copy" });
    expect(routeKey(ev("v", "KeyV", { metaKey: true }), true)).toEqual({ kind: "paste" });
    expect(routeKey(ev("w", "KeyW", { metaKey: true }), true)).toEqual({ kind: "ignore" });
  });

  it("opens a new tab with Ctrl+Shift+T or Cmd+T", () => {
    expect(routeKey(ev("T", "KeyT", { ctrlKey: true, shiftKey: true }), false)).toEqual({
      kind: "newTab",
    });
    expect(routeKey(ev("t", "KeyT", { metaKey: true }), true)).toEqual({ kind: "newTab" });
    // Plain Ctrl+T is the shell's (transpose characters).
    expect(routeKey(ev("t", "KeyT", { ctrlKey: true }), false)).toMatchObject({ kind: "key" });
  });

  it("ignores lone modifiers", () => {
    expect(routeKey(ev("Shift", "ShiftLeft", { shiftKey: true }), false)).toEqual({
      kind: "ignore",
    });
  });
});

describe("InputQueue", () => {
  it("sends one item at a time, in order, merging text typed meanwhile", async () => {
    const sent: InputItem[] = [];
    const pending: (() => void)[] = [];
    const queue = new InputQueue(async (item) => {
      sent.push(structuredClone(item));
      await new Promise<void>((resolve) => pending.push(resolve));
    });
    queue.push({ kind: "text", text: "e" });
    queue.push({ kind: "text", text: "c" });
    queue.push({ kind: "text", text: "ho" });
    queue.push({ kind: "key", key: { key: "Enter", ctrl: false, alt: false, shift: false } });
    queue.push({ kind: "text", text: "x" });
    expect(sent).toEqual([{ kind: "text", text: "e" }]);
    for (let i = 0; i < 4; i++) {
      pending.shift()?.();
      await new Promise((resolve) => setTimeout(resolve, 0));
    }
    expect(sent.map((s) => (s.kind === "key" ? s.key.key : s.text))).toEqual([
      "e",
      "cho",
      "Enter",
      "x",
    ]);
  });
});
