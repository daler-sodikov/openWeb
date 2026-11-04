import fs from "node:fs";
import path from "node:path";

import { caretToStart, editorText, expect, keys, openApp, openFile, test } from "./harness";

test("select next and all occurrences", async ({ page, project }) => {
  fs.writeFileSync(path.join(project, "multi.ts"), "foo bar foo\nfoo\n");
  await openApp(page, project);
  await openFile(page, "multi.ts");
  await caretToStart(page);

  await page.keyboard.press(keys.selectNext); // selects the word
  await page.keyboard.press(keys.selectNext); // + second occurrence
  await page.keyboard.press(keys.selectNext); // + third occurrence
  await expect(page.getByTestId("status-position")).toContainText("(3 carets)");
  await page.keyboard.type("x");
  await expect.poll(() => editorText(page)).toBe("x bar x\nx\n");

  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(() => editorText(page)).toBe("foo bar foo\nfoo\n");
  await caretToStart(page);
  await page.keyboard.press(keys.selectAll);
  await expect(page.getByTestId("status-position")).toContainText("(3 carets)");
  await page.keyboard.type("y");
  await expect.poll(() => editorText(page)).toBe("y bar y\ny\n");
});

test("clone caret below with double-tap modifier", async ({ page, project }) => {
  fs.writeFileSync(path.join(project, "lines.ts"), "aaa\nbbb\nccc\n");
  await openApp(page, project);
  await openFile(page, "lines.ts");
  await caretToStart(page);

  await page.keyboard.down(keys.caretModifier);
  await page.keyboard.up(keys.caretModifier);
  await page.keyboard.down(keys.caretModifier);
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("ArrowDown");
  await page.keyboard.up(keys.caretModifier);
  await expect(page.getByTestId("status-position")).toContainText("(3 carets)");

  await page.keyboard.type("-");
  await expect.poll(() => editorText(page)).toBe("-aaa\n-bbb\n-ccc\n");
});

test("Alt+Shift+Click adds a caret", async ({ page, project }) => {
  fs.writeFileSync(path.join(project, "click.ts"), "one\ntwo\n");
  await openApp(page, project);
  await openFile(page, "click.ts");
  await caretToStart(page);

  const second = page.locator(".editor-host:not([hidden]) .cm-line").nth(1);
  await second.click({ modifiers: ["Alt", "Shift"], position: { x: 1, y: 5 } });
  await expect(page.getByTestId("status-position")).toContainText("(2 carets)");
  await page.keyboard.type("#");
  await expect.poll(() => editorText(page)).toBe("#one\n#two\n");
});
