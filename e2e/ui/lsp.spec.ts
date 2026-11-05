// M3a: diagnostics, Problems, Quick Documentation and navigation with the
// real typescript-language-server, and the status when it is missing.
import fs from "node:fs";
import path from "node:path";

import type { Page } from "@playwright/test";

import { BACKEND_DATA } from "../paths";
import {
  activeTab,
  editor,
  expect,
  keys,
  openApp,
  openFile,
  test,
  tsServerMissing,
} from "./harness";

/**
 * Records every text the server status shows (call before opening a file),
 * so a short "indexing" phase is not missed between polls.
 */
async function recordServerStatus(page: Page): Promise<void> {
  await page.evaluate(() => {
    const el = document.querySelector('[data-testid="status-server"]');
    const seen: string[] = [];
    Object.assign(window, { serverStatusSeen: seen });
    if (!el) return;
    new MutationObserver(() => {
      seen.push(el.textContent);
    }).observe(el, { childList: true, characterData: true, subtree: true });
  });
}

/**
 * Waits until the server has loaded the project: it reported indexing and
 * is ready again. (While loading, typescript-language-server answers some
 * requests from syntax only, e.g. definitions stop at imports.)
 */
async function projectLoaded(page: Page): Promise<void> {
  await expect
    .poll(
      () =>
        page.evaluate(() => {
          const seen =
            (window as unknown as { serverStatusSeen?: string[] }).serverStatusSeen ?? [];
          const indexed = seen.indexOf("TypeScript: indexing");
          return indexed >= 0 && seen.slice(indexed).includes("TypeScript: ready");
        }),
      { timeout: 60_000 },
    )
    .toBe(true);
  await expect(page.getByTestId("status-server")).toHaveText("TypeScript: ready");
}

/**
 * Clicks the middle of `word` in the first editor line containing `line`
 * (exact text position, independent of how the line is highlighted).
 */
async function clickWord(page: Page, line: string, word: string, modifier?: string): Promise<void> {
  const point = await page.evaluate(
    ({ lineText, target }) => {
      const lines = document.querySelectorAll(".editor-host:not([hidden]) .cm-line");
      const row = [...lines].find((el) => el.textContent.includes(lineText));
      if (!row) return null;
      const start = row.textContent.indexOf(target, row.textContent.indexOf(lineText));
      // Walk the text nodes to the character offset of `target`.
      const walker = document.createTreeWalker(row, NodeFilter.SHOW_TEXT);
      let offset = 0;
      for (let node = walker.nextNode(); node; node = walker.nextNode()) {
        const length = node.textContent?.length ?? 0;
        if (start < offset + length) {
          const range = document.createRange();
          const at = start - offset + Math.floor(target.length / 2);
          range.setStart(node, Math.min(at, length - 1));
          range.setEnd(node, Math.min(at + 1, length));
          const rect = range.getBoundingClientRect();
          return { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 };
        }
        offset += length;
      }
      return null;
    },
    { lineText: line, target: word },
  );
  if (!point) throw new Error(`"${word}" not found in a line containing "${line}"`);
  if (modifier) await page.keyboard.down(modifier);
  await page.mouse.click(point.x, point.y);
  if (modifier) await page.keyboard.up(modifier);
  // Move away so the mouse hover tooltip does not open over the word.
  await page.mouse.move(0, 0);
}

/** The visible editor's caret line (1-based) from the status bar. */
async function caretLine(page: Page): Promise<number> {
  const text = (await page.getByTestId("status-position").textContent()) ?? "";
  return Number(text.split(":")[0]);
}

test.describe("with typescript-language-server", () => {
  test.beforeEach(() => {
    const missing = tsServerMissing();
    // CI installs the server; there a missing one is a setup error.
    if (missing !== null && process.env.CI) throw new Error(missing);
    test.skip(missing !== null, missing ?? "");
  });

  test("diagnostics show as squiggles, in Problems and in the status bar", async ({
    page,
    project,
  }) => {
    await openApp(page, project);
    await recordServerStatus(page);
    await openFile(page, "src/utils/format.ts");
    await projectLoaded(page);
    await editor(page).click();
    await page.keyboard.press("ControlOrMeta+End");
    await page.keyboard.type('export const broken: number = "text";');

    // Squiggle on `broken`, with the message on hover.
    const squiggle = page.locator(".cm-diag-error", { hasText: "broken" });
    await expect(squiggle).toBeVisible({ timeout: 30_000 });
    await squiggle.hover();
    const tooltip = page.getByTestId("insight");
    await expect(tooltip).toContainText("is not assignable to type 'number'");

    // Status bar counts and the Problems tool window.
    await expect
      .poll(async () =>
        Number(await page.getByTestId("status-problems").getAttribute("data-errors")),
      )
      .toBeGreaterThanOrEqual(1);
    await page.keyboard.press(keys.problems);
    const problems = page.getByTestId("problems");
    await expect(problems).toBeVisible();
    await expect(
      problems.locator('[data-testid="problem-file"][data-path="src/utils/format.ts"]'),
    ).toBeVisible();
    const item = problems.getByTestId("problem-item").filter({ hasText: "number" });
    await expect(item).toHaveAttribute("data-severity", "error");

    // Clicking a problem opens the file at its location.
    await openFile(page, "src/App.tsx");
    await item.click();
    await expect(activeTab(page)).toHaveAttribute("data-path", "src/utils/format.ts");
    await expect.poll(() => caretLine(page)).toBe(9);

    // Fixing the code clears the problem.
    await page.keyboard.press("End");
    await page.keyboard.press("Shift+Home");
    await page.keyboard.type("export const broken: number = 1;");
    await expect(squiggle).toHaveCount(0, { timeout: 30_000 });
    await expect(problems.getByTestId("problem-item").filter({ hasText: "number" })).toHaveCount(0);
  });

  test("Quick Documentation shows the signature", async ({ page, project }) => {
    await openApp(page, project);
    await recordServerStatus(page);
    await openFile(page, "src/utils/format.ts");
    await projectLoaded(page);
    await clickWord(page, "export function pluralize", "pluralize");
    await page.keyboard.press(keys.quickDoc);
    const doc = page.locator(".quick-doc [data-testid=quick-doc]");
    await expect(doc).toContainText("function pluralize(word: string, count: number): string");
    // Code in the documentation is highlighted like the editor.
    await expect(doc.locator(".tok-keyword").first()).toHaveText("function");
    await page.keyboard.press("Escape");
    await expect(doc).toHaveCount(0);
  });

  test("Go to Declaration with Ctrl+Click and Ctrl+B, and a chooser for several targets", async ({
    page,
    project,
  }) => {
    fs.writeFileSync(
      path.join(project, "src", "shapes.ts"),
      [
        "export interface Shape {",
        "  area(): number;",
        "}",
        "export class Square implements Shape {",
        "  area() { return 1; }",
        "}",
        "export class Circle implements Shape {",
        "  area() { return 3; }",
        "}",
        "",
      ].join("\n"),
    );
    await openApp(page, project);
    await recordServerStatus(page);
    await openFile(page, "src/App.tsx");
    await projectLoaded(page);

    // Ctrl/Cmd+Click on a call opens the declaring file at the declaration.
    await clickWord(page, "Total: {formatPrice(total)}", "formatPrice", "ControlOrMeta");
    await expect(activeTab(page)).toHaveAttribute("data-path", "src/utils/format.ts");
    await expect.poll(() => caretLine(page)).toBe(2);

    // Ctrl+B on a JSX element goes to the component.
    await openFile(page, "src/App.tsx");
    await clickWord(page, "<Counter start", "Counter");
    await page.keyboard.press(keys.gotoDeclaration);
    await expect(activeTab(page)).toHaveAttribute("data-path", "src/components/Counter.tsx");
    await expect.poll(() => caretLine(page)).toBe(3);

    // Two implementations: a chooser lists both.
    await openFile(page, "src/shapes.ts");
    await clickWord(page, "export interface Shape", "Shape");
    await page.keyboard.press(keys.gotoImplementation);
    const chooser = page.getByTestId("nav-chooser");
    await expect(chooser.getByTestId("nav-target")).toHaveCount(2);
    await expect(chooser).toContainText("class Circle implements Shape");
    await chooser.getByTestId("nav-target").filter({ hasText: "Circle" }).click();
    await expect(chooser).toHaveCount(0);
    await expect.poll(() => caretLine(page)).toBe(7);
  });

  test("Find Usages, Rename with preview and completion", async ({ page, project }) => {
    fs.writeFileSync(
      path.join(project, "src", "rename-me.ts"),
      "export function oldName(): number {\n  return 1;\n}\n",
    );
    const user = path.join(project, "src", "use-rename.ts");
    fs.writeFileSync(
      user,
      'import { oldName } from "./rename-me";\n\nexport const value = oldName();\n',
    );
    await openApp(page, project);
    await recordServerStatus(page);
    await openFile(page, "src/rename-me.ts");
    await projectLoaded(page);

    // Find Usages lists the declaration and the use in the closed file.
    await clickWord(page, "export function oldName", "oldName");
    await page.keyboard.press("Alt+F7");
    await expect(page.getByTestId("usage-result").first()).toBeVisible();
    expect(await page.getByTestId("usage-result").count()).toBeGreaterThanOrEqual(2);
    await page.keyboard.press("Escape");

    // Rename: preview first, then apply (open tab edited, closed file written).
    await clickWord(page, "export function oldName", "oldName");
    await page.keyboard.press("Shift+F6");
    // The dialog opens after the server confirmed the symbol can be renamed.
    const nameField = page.getByRole("textbox", { name: "New name" });
    await expect(nameField).toBeVisible();
    await expect(nameField).toHaveValue("oldName");
    await nameField.fill("newName");
    await page.keyboard.press("Enter");
    await expect(page.getByTestId("rename-files")).toContainText("use-rename.ts");
    await page.getByTestId("rename-apply").click();
    await expect(page.locator(".cm-content").first()).toContainText("function newName");
    await expect
      .poll(() => fs.readFileSync(user, "utf8"), { timeout: 15_000 })
      .toContain("newName()");

    // Completion offers the new name.
    await editor(page).click();
    await page.keyboard.press("ControlOrMeta+End");
    await page.keyboard.type("\nnewNa");
    await page.keyboard.press("Control+Space");
    await expect(page.locator(".cm-tooltip-autocomplete")).toContainText("newName", {
      timeout: 15_000,
    });
    await page.keyboard.press("Escape");
  });
});

test("a missing server is reported and editing keeps working", async ({ page, project }) => {
  const settings = path.join(BACKEND_DATA, "settings.json");
  fs.mkdirSync(BACKEND_DATA, { recursive: true });
  fs.writeFileSync(
    settings,
    JSON.stringify({ typescript: { serverPath: path.join(project, "no-such-server") } }),
  );
  try {
    await openApp(page, project);
    await openFile(page, "src/utils/format.ts");
    const status = page.getByTestId("status-server");
    await expect(status).toHaveText("TypeScript: not available");
    await expect(status).toHaveAttribute("title", /typescript\.serverPath/);
    await editor(page).click();
    await page.keyboard.press("ControlOrMeta+End");
    await page.keyboard.type("\n// still editable");
    await expect(page.getByTestId("tab-dirty")).toHaveText("●");
    await page.keyboard.press("ControlOrMeta+s");
    await expect(page.getByTestId("tab-dirty")).toHaveText("");
    expect(fs.readFileSync(path.join(project, "src/utils/format.ts"), "utf8")).toContain(
      "// still editable",
    );
  } finally {
    fs.rmSync(settings, { force: true });
  }
});
