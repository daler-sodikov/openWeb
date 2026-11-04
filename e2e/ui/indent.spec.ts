import fs from "node:fs";
import path from "node:path";

import { activeTab, caretToStart, editorText, expect, openApp, openFile, test } from "./harness";

test("Enter indents in Rust and splits bracket pairs", async ({ page, project }) => {
  fs.writeFileSync(path.join(project, "f.ts"), "function f() {}\n");
  await openApp(page, project);
  await openFile(page, "f.ts");
  await caretToStart(page);
  await page.keyboard.press("End");
  await page.keyboard.press("ArrowLeft");
  await page.keyboard.press("Enter");
  await page.keyboard.type("x");
  await expect.poll(() => editorText(page)).toBe("function f() {\n  x\n}\n");
  await expect(page.getByTestId("status-position")).toHaveText("2:4");
});

test("Tab inserts the file's indentation unit", async ({ page, project }) => {
  fs.writeFileSync(path.join(project, "four.ts"), "if (a) {\n    b();\n}\n");
  await openApp(page, project);
  await openFile(page, "four.ts");
  await caretToStart(page);
  await page.keyboard.press("Tab");
  await expect.poll(() => editorText(page)).toBe("    if (a) {\n    b();\n}\n");
});

test("Enter and typing undo together; undo to saved text clears the marker", async ({
  page,
  project,
}) => {
  fs.writeFileSync(path.join(project, "g.ts"), "start\n");
  await openApp(page, project);
  await openFile(page, "g.ts");
  await caretToStart(page);
  await page.keyboard.press("End");
  await page.keyboard.type("a");
  await page.keyboard.press("Enter");
  await page.keyboard.type("b");
  await expect.poll(() => editorText(page)).toBe("starta\nb\n");
  const dirty = activeTab(page).getByTestId("tab-dirty");
  await expect(dirty).toHaveText("●");

  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(() => editorText(page)).toBe("start\n");
  await expect(dirty).toHaveText("");
  await expect(page.getByTestId("status-state")).toHaveText("Saved");

  await page.keyboard.press("ControlOrMeta+Shift+Z");
  await expect.poll(() => editorText(page)).toBe("starta\nb\n");
  await expect(dirty).toHaveText("●");
});
