import fs from "node:fs";
import path from "node:path";

import {
  activeTab,
  caretToStart,
  editor,
  editorText,
  expect,
  openApp,
  openFile,
  test,
} from "./harness";

test("opens files in tabs and keeps each file's state", async ({ page, project }) => {
  await openApp(page, project);
  await openFile(page, "src/App.tsx");
  await expect(editor(page)).toContainText("export function App");

  await openFile(page, "src/utils/format.ts");
  await expect(page.getByTestId("tab")).toHaveCount(2);
  await expect(editor(page)).toContainText("formatPrice");
  await page.keyboard.press("ControlOrMeta+End");
  await expect(page.getByTestId("status-position")).toHaveText("9:1");

  await page.getByTestId("tab").first().click();
  await expect(activeTab(page)).toHaveAttribute("data-path", "src/App.tsx");
  await expect(editor(page)).toContainText("export function App");

  await page.getByTestId("tab").nth(1).click();
  await expect(page.getByTestId("status-position")).toHaveText("9:1");

  await activeTab(page).getByTestId("tab-close").click();
  await expect(page.getByTestId("tab")).toHaveCount(1);
  await expect(activeTab(page)).toHaveAttribute("data-path", "src/App.tsx");
});

test("editor layout works under the production CSP", async ({ page, project }) => {
  // CodeMirror injects its base styles at runtime; a CSP blocking inline
  // styles stacks the gutter above the text and collapses indentation.
  await openApp(page, project);
  await openFile(page, "src/App.tsx");
  const gutter = await page.locator(".editor-host:not([hidden]) .cm-gutters").boundingBox();
  const content = await editor(page).boundingBox();
  if (!gutter || !content) throw new Error("editor not rendered");
  expect(gutter.x + gutter.width).toBeLessThanOrEqual(content.x + 1);
  expect(Math.abs(gutter.y - content.y)).toBeLessThan(5);
  await expect(editor(page)).toHaveCSS("white-space", /pre/);
});

test("highlights files with tree-sitter and follows edits", async ({ page, project }) => {
  await openApp(page, project);
  const visible = (cls: string, text: string) =>
    page.locator(`.editor-host:not([hidden]) .tok-${cls}`, { hasText: text }).first();

  await openFile(page, "src/App.tsx");
  await expect(visible("keyword", "import")).toBeVisible();
  await expect(visible("tag", "main")).toBeVisible();
  await expect(visible("string", "./utils/format")).toBeVisible();

  await openFile(page, "src/data/items.json");
  await expect(visible("property", '"name"')).toBeVisible();
  await expect(visible("number", "2.5")).toBeVisible();

  await openFile(page, "src/styles.css");
  await expect(visible("property", "max-width")).toBeVisible();

  await openFile(page, "index.html");
  await expect(visible("tag", "html")).toBeVisible();

  await openFile(page, "src/utils/format.ts");
  await caretToStart(page);
  await page.keyboard.type("const answer = 42;\n");
  await expect(visible("number", "42")).toBeVisible();
  await expect(visible("keyword", "const")).toBeVisible();
});

test("edits and saves, keeping the file's line endings", async ({ page, project }) => {
  const file = path.join(project, "notes.ts");
  fs.writeFileSync(file, "let a = 1;\r\nlet b = 2;\r\n");
  await openApp(page, project);
  await openFile(page, "notes.ts");
  await expect(page.getByTestId("status-state")).toHaveText("Saved");

  await caretToStart(page);
  await page.keyboard.type("// hi\n");
  await expect(activeTab(page).getByTestId("tab-dirty")).toHaveText("●");
  await expect(page.getByTestId("status-state")).toHaveText("Modified");

  await page.keyboard.press("ControlOrMeta+s");
  await expect(page.getByTestId("status-state")).toHaveText("Saved");
  await expect(activeTab(page).getByTestId("tab-dirty")).toHaveText("");
  expect(fs.readFileSync(file, "utf8")).toBe("// hi\r\nlet a = 1;\r\nlet b = 2;\r\n");
});

test("undo and redo run in Rust and group typing", async ({ page, project }) => {
  await openApp(page, project);
  await openFile(page, "src/utils/format.ts");
  const original = await editorText(page);

  await caretToStart(page);
  await page.keyboard.type("abc def ");
  await expect(editor(page)).toContainText("abc def // Formatting helpers.");

  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(() => editorText(page)).toBe(original);
  await expect(page.getByTestId("status-position")).toHaveText("1:1");

  // Uppercase key, as a real keyboard reports it with Shift held.
  await page.keyboard.press("ControlOrMeta+Shift+Z");
  await expect(editor(page)).toContainText("abc def // Formatting helpers.");
  await expect(page.getByTestId("status-position")).toHaveText("1:9");
});

test("closing a modified tab keeps its text", async ({ page, project }) => {
  await openApp(page, project);
  await openFile(page, "src/utils/format.ts");
  await caretToStart(page);
  await page.keyboard.type("// draft\n");
  await activeTab(page).getByTestId("tab-close").click();
  await expect(page.getByTestId("tab")).toHaveCount(0);

  await openFile(page, "src/utils/format.ts");
  await expect(editor(page)).toContainText("// draft");
  await expect(page.getByTestId("status-notice")).toContainText("Restored unsaved changes");
  await expect(activeTab(page).getByTestId("tab-dirty")).toHaveText("●");
  expect(fs.readFileSync(path.join(project, "src/utils/format.ts"), "utf8")).not.toContain("draft");
});
