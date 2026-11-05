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
  clickWord,
  projectLoaded,
  recordServerStatus,
  test,
  tsServerMissing,
} from "./harness";

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

test.describe("M3b with typescript-language-server", () => {
  test.beforeEach(() => {
    const missing = tsServerMissing();
    if (missing !== null && process.env.CI) throw new Error(missing);
    test.skip(missing !== null, missing ?? "");
  });

  test("completion opens on `.` and parameter info on `(`", async ({ page, project }) => {
    fs.writeFileSync(
      path.join(project, "src", "typing.ts"),
      'import { formatPrice } from "./utils/format";\nexport const s = "abc";\n',
    );
    await openApp(page, project);
    await recordServerStatus(page);
    await openFile(page, "src/typing.ts");
    await projectLoaded(page);
    await editor(page).click();
    await page.keyboard.press("ControlOrMeta+End");

    // `.` is a completion trigger: members of a string, no Ctrl+Space.
    await page.keyboard.type("export const n = s.");
    await expect(page.locator(".cm-tooltip-autocomplete")).toContainText("length", {
      timeout: 15_000,
    });
    await page.keyboard.press("Escape");

    // `(` is a parameter-info trigger: the first parameter is active.
    await page.keyboard.type("length;\nexport const p = formatPrice(");
    const signature = page.getByTestId("signature-help");
    await expect(signature).toContainText("formatPrice(value: number", { timeout: 15_000 });
    await expect(signature.locator("b")).toHaveText("value: number");
    await page.keyboard.press("Escape");

    // Accepting a symbol of another module adds its import (resolved lazily).
    await page.keyboard.press("ControlOrMeta+End");
    await page.keyboard.type("\nexport const w = plurali");
    await page.keyboard.press("Control+Space");
    const option = page.locator(".cm-tooltip-autocomplete li", { hasText: "pluralize" }).first();
    await expect(option).toBeVisible({ timeout: 15_000 });
    // CodeMirror ignores Enter for 75 ms after the list (re)appears, so an
    // Enter typed just before it is not taken as accepting.
    await page.waitForTimeout(200);
    await page.keyboard.press("Enter");
    await expect(page.locator(".editor-host:not([hidden]) .cm-line").first()).toContainText(
      'pluralize } from "./utils/format";',
    );
    // Enter accepted the item instead of inserting a newline.
    await expect(editor(page)).toContainText("export const w = pluralize");
  });

  test("Alt+Enter adds a missing import", async ({ page, project }) => {
    fs.writeFileSync(
      path.join(project, "src", "fix.ts"),
      'export const word = pluralize("item", 2);\n',
    );
    await openApp(page, project);
    await recordServerStatus(page);
    await openFile(page, "src/fix.ts");
    await projectLoaded(page);
    await expect(page.locator(".cm-diag-error", { hasText: "pluralize" })).toBeVisible({
      timeout: 30_000,
    });
    await clickWord(page, "export const word", "pluralize");
    await page.keyboard.press(keys.intentions);
    const action = page
      .getByTestId("code-action")
      .filter({ hasText: /import/i })
      .first();
    await expect(action).toBeVisible({ timeout: 15_000 });
    await action.click();
    await expect(page.locator(".editor-host:not([hidden]) .cm-line").first()).toHaveText(
      'import { pluralize } from "./utils/format";',
    );
    await expect(page.locator(".cm-diag-error", { hasText: "pluralize" })).toHaveCount(0, {
      timeout: 30_000,
    });
  });

  test("Reformat Code uses the server and undoes in one step", async ({ page, project }) => {
    fs.writeFileSync(path.join(project, "src", "messy.ts"), "export const   messy={a:1};\n");
    await openApp(page, project);
    await recordServerStatus(page);
    await openFile(page, "src/messy.ts");
    await projectLoaded(page);
    await editor(page).click();
    await page.keyboard.press(keys.reformat);
    await expect(editor(page)).toContainText("export const messy = { a: 1 };");
    await page.keyboard.press(keys.undo);
    await expect(editor(page)).toContainText("export const   messy={a:1};");
  });

  test("library code opens read-only", async ({ page, project }) => {
    test.skip(
      !fs.existsSync(path.join(project, "node_modules", "@types", "react")),
      "run `bun install` in fixtures/react-ts first",
    );
    await openApp(page, project);
    await recordServerStatus(page);
    await openFile(page, "src/components/Counter.tsx");
    await projectLoaded(page);
    await clickWord(page, "useState(start)", "useState");
    await page.keyboard.press(keys.gotoDeclaration);
    const tab = activeTab(page);
    await expect(tab).toHaveAttribute("data-path", /node_modules\/@types\/react\/index\.d\.ts$/);
    await expect(tab.getByTestId("tab-read-only")).toHaveText("RO");
    const before = await editor(page).textContent();
    await page.keyboard.type("typed");
    await expect(tab.getByTestId("tab-dirty")).toHaveText("");
    expect(await editor(page).textContent()).toBe(before);
  });
});

test("oxc marks unused imports without a language server", async ({ page, project }) => {
  const settings = path.join(BACKEND_DATA, "settings.json");
  fs.mkdirSync(BACKEND_DATA, { recursive: true });
  fs.writeFileSync(
    settings,
    JSON.stringify({ typescript: { serverPath: path.join(project, "no-such-server") } }),
  );
  fs.writeFileSync(
    path.join(project, "src", "imports.ts"),
    'import { formatPrice, pluralize } from "./utils/format";\nexport const p = formatPrice(1);\n',
  );
  try {
    await openApp(page, project);
    await openFile(page, "src/imports.ts");
    await expect(page.getByTestId("status-server")).toHaveText("TypeScript: not available");
    const unused = page.locator(".cm-diag-unnecessary", { hasText: "pluralize" });
    await expect(unused).toBeVisible();
    await expect(page.locator(".cm-diag-unnecessary", { hasText: "formatPrice" })).toHaveCount(0);
    await unused.hover();
    await expect(page.getByTestId("insight")).toContainText("Unused import 'pluralize'");
    await expect(page.getByTestId("insight")).toContainText("oxc");
    // Using it removes the hint after the edit settles.
    await editor(page).click();
    await page.keyboard.press("ControlOrMeta+End");
    await page.keyboard.type('export const q = pluralize("a", 2);');
    await expect(unused).toHaveCount(0);
  } finally {
    fs.rmSync(settings, { force: true });
  }
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
