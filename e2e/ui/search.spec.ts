import fs from "node:fs";
import path from "node:path";

import {
  activeTab,
  caretToStart,
  editor,
  editorText,
  expect,
  keys,
  openApp,
  openFile,
  test,
  waitForIndex,
} from "./harness";

test("double Shift opens Search Everywhere with files and symbols", async ({ page, project }) => {
  await openApp(page, project);
  await page.getByTestId("project-tree").click();
  await page.keyboard.press("Shift");
  await page.keyboard.press("Shift");
  const popup = page.getByTestId("search-everywhere");
  await expect(popup).toBeVisible();
  await expect(page.getByTestId("se-tab-all")).toHaveAttribute("aria-pressed", "true");

  await page.getByTestId("se-input").fill("cnt");
  const file = popup.locator('[data-type="file"][data-path="src/components/Counter.tsx"]');
  const symbol = popup.locator('[data-type="symbol"]', { hasText: "Counter" });
  await expect(file).toBeVisible();
  await expect(symbol).toBeVisible();

  // Layout: the popup sits near the top of the window and rows have height
  // (the DOM alone would also be "visible" when unstyled).
  const box = await popup.boundingBox();
  const row = await symbol.first().boundingBox();
  expect(box && box.y < 200 && box.height > 150).toBe(true);
  expect(row && row.height >= 20).toBe(true);

  await symbol.first().click();
  await expect(popup).toHaveCount(0);
  await expect(activeTab(page)).toHaveAttribute("data-path", "src/components/Counter.tsx");
  await expect(page.getByTestId("status-position")).toHaveText("3:17");

  // Tab cycles the tabs; Escape closes.
  await page.keyboard.press("Shift");
  await page.keyboard.press("Shift");
  await page.keyboard.press("Tab");
  await expect(page.getByTestId("se-tab-files")).toHaveAttribute("aria-pressed", "true");
  await page.keyboard.press("Escape");
  await expect(popup).toHaveCount(0);
});

test("Go to File, Symbol, Line, Recent Files and Find Action", async ({ page, project }) => {
  await openApp(page, project);
  await page.getByTestId("project-tree").click();

  await page.keyboard.press(keys.gotoFile);
  await expect(page.getByTestId("se-tab-files")).toHaveAttribute("aria-pressed", "true");
  await page.getByTestId("se-input").fill("format");
  await expect(
    page.locator('[data-testid="se-item"][data-path="src/utils/format.ts"]'),
  ).toBeVisible();
  await page.keyboard.press("Enter");
  await expect(activeTab(page)).toHaveAttribute("data-path", "src/utils/format.ts");

  await page.keyboard.press(keys.gotoSymbol);
  await expect(page.getByTestId("se-tab-symbols")).toHaveAttribute("aria-pressed", "true");
  await page.getByTestId("se-input").fill("Counter");
  await expect(page.locator('[data-type="symbol"]').first()).toContainText("Counter");
  await page.keyboard.press("Enter");
  await expect(activeTab(page)).toHaveAttribute("data-path", "src/components/Counter.tsx");

  await page.keyboard.press(keys.gotoLine);
  await page.getByTestId("goto-line-input").fill("5:3");
  await page.keyboard.press("Enter");
  await expect(page.getByTestId("status-position")).toHaveText("5:3");

  // Recent Files preselects the previous file (format.ts).
  await page.keyboard.press(keys.recentFiles);
  const recent = page.getByTestId("recent-item");
  await expect(recent.first()).toHaveAttribute("data-path", "src/components/Counter.tsx");
  await expect(recent.nth(1)).toHaveAttribute("data-path", "src/utils/format.ts");
  await page.keyboard.press("Enter");
  await expect(activeTab(page)).toHaveAttribute("data-path", "src/utils/format.ts");

  await page.keyboard.press(keys.gotoAction);
  await expect(page.getByTestId("se-tab-actions")).toHaveAttribute("aria-pressed", "true");
  await page.getByTestId("se-input").fill("find in files");
  await expect(page.locator('[data-type="action"]').first()).toContainText("Find in Files");
  await page.keyboard.press("Enter");
  await expect(page.getByTestId("find-in-files")).toBeVisible();
});

test("Find in Files: options, unsaved text, navigation and errors", async ({ page, project }) => {
  await openApp(page, project);
  await openFile(page, "src/utils/format.ts");
  await caretToStart(page);
  await page.keyboard.type("// needle\n");

  await page.keyboard.press(keys.findInFiles);
  const find = page.getByTestId("find-input");
  await find.fill("needle");
  const results = page.getByTestId("find-match");
  await expect(results).toHaveCount(1);
  await expect(results.first()).toHaveAttribute("data-path", "src/utils/format.ts");

  await find.fill("count");
  // Wait for this query's results (the previous query had one match).
  await expect.poll(() => results.count()).toBeGreaterThan(1);
  const all = await results.count();
  await page.getByTestId("find-word").click();
  await expect.poll(() => results.count()).toBeLessThan(all);

  await page.getByTestId("find-word").click();
  await page.getByTestId("find-regex").click();
  await find.fill("set\\w+\\(");
  await expect(results.first()).toContainText("setCount(");

  await find.fill("(");
  await expect(page.getByTestId("find-status")).toContainText("invalid regular expression");

  await find.fill("useState");
  await expect(results.first()).toHaveAttribute("data-path", "src/components/Counter.tsx");
  await page.keyboard.press("Enter");
  await expect(activeTab(page)).toHaveAttribute("data-path", "src/components/Counter.tsx");
  await expect(page.getByTestId("status-position")).toHaveText("1:10");
});

test("Replace in Files: preview, open documents and closed files", async ({ page, project }) => {
  fs.mkdirSync(path.join(project, "notes"));
  fs.writeFileSync(path.join(project, "notes/todo.ts"), "let count = 0;\r\n");
  await openApp(page, project);
  await waitForIndex();
  await openFile(page, "src/components/Counter.tsx");

  await page.keyboard.press(keys.replaceInFiles);
  await page.getByTestId("find-word").click();
  await page.getByTestId("find-case").click();
  await page.getByTestId("find-input").fill("count");
  await page.getByTestId("find-replace-input").fill("total");
  const preview = page.getByTestId("find-match").first();
  await expect(preview.locator("del")).toHaveText("count");
  await expect(preview.locator("ins")).toHaveText("total");

  await page.getByTestId("find-replace-all").click();
  // Counter.tsx (open, 4), notes/todo.ts (1) and src/utils/format.ts (2).
  await expect(page.getByTestId("find-status")).toContainText("Replaced 7 occurrences in 3 files");
  expect(fs.readFileSync(path.join(project, "notes/todo.ts"), "utf8")).toBe("let total = 0;\r\n");
  expect(fs.readFileSync(path.join(project, "src/utils/format.ts"), "utf8")).toContain(
    "total === 1",
  );

  await page.keyboard.press("Escape");
  await expect(editor(page)).toContainText("total is {total}");
  await expect(activeTab(page).getByTestId("tab-dirty")).toHaveText("●");
  // The replacement in the open document is undoable.
  await editor(page).click();
  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(() => editorText(page)).toContain("count is {count}");
});
