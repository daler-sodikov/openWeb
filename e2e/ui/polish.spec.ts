import fs from "node:fs";
import path from "node:path";

import {
  activeTab,
  caretToStart,
  editorText,
  expect,
  keys,
  openApp,
  openFile,
  test,
  treeRow,
} from "./harness";

test("split editors show the same file and stay in sync", async ({ page, project }) => {
  await openApp(page, project);
  await openFile(page, "src/utils/format.ts");
  await page.getByTestId("split-right").click();
  const groups = page.getByTestId("editor-group");
  await expect(groups).toHaveCount(2);
  const left = groups.nth(0).locator(".editor-host:not([hidden]) .cm-content");
  const right = groups.nth(1).locator(".editor-host:not([hidden]) .cm-content");
  await expect(right).toContainText("formatPrice");

  await left.click();
  await page.keyboard.press("ControlOrMeta+Home");
  await page.keyboard.type("// from left\n");
  await expect(right).toContainText("// from left");

  await right.click();
  await page.keyboard.press("ControlOrMeta+End");
  await page.keyboard.type("// from right");
  await expect(left).toContainText("// from right");

  // Closing the right group's only tab removes the group; the document stays open.
  await groups.nth(1).getByTestId("tab-close").click();
  await expect(groups).toHaveCount(1);
  await expect(left).toContainText("// from right");
});

test("breadcrumbs follow the caret and navigate", async ({ page, project }) => {
  await openApp(page, project);
  await openFile(page, "src/App.tsx");
  await page.keyboard.press(keys.gotoLine);
  await page.getByTestId("goto-line-input").fill("14:8");
  await page.keyboard.press("Enter");
  const crumbs = page.locator(".editor-host:not([hidden]) [data-testid='crumb']");
  await expect(crumbs).toHaveText(["App", "main", "Counter"]);
  await crumbs.first().click();
  await expect(page.getByTestId("status-position")).toHaveText("9:8");
});

test("code folding from Rust ranges", async ({ page, project }) => {
  await openApp(page, project);
  await openFile(page, "src/App.tsx");
  const visible = page.locator(".editor-host:not([hidden])");
  await expect(visible.locator('.cm-foldGutter [title="Fold line"]').first()).toBeVisible();

  await page.keyboard.press(keys.gotoLine);
  await page.getByTestId("goto-line-input").fill("9:1");
  await page.keyboard.press("Enter");
  await page.keyboard.press("ControlOrMeta+Minus");
  await expect(visible.locator(".cm-foldPlaceholder")).toHaveCount(1);
  await expect(visible.locator(".cm-content")).not.toContainText("const total");
  await page.keyboard.press("ControlOrMeta+Equal");
  await expect(visible.locator(".cm-foldPlaceholder")).toHaveCount(0);

  await page.keyboard.press("ControlOrMeta+Shift+Minus");
  await expect.poll(() => visible.locator(".cm-foldPlaceholder").count()).toBeGreaterThan(1);
  await page.keyboard.press("ControlOrMeta+Shift+Equal");
  await expect(visible.locator(".cm-foldPlaceholder")).toHaveCount(0);
});

test("bracket matching, matching-brace jump and auto-close", async ({ page, project }) => {
  fs.writeFileSync(path.join(project, "b.ts"), 'f(a, [1, 2], "(not)");\n');
  await openApp(page, project);
  await openFile(page, "b.ts");
  await caretToStart(page);
  await page.keyboard.press("ArrowRight"); // before "("
  const marks = page.locator(".editor-host:not([hidden]) .cm-bracket-match");
  await expect(marks).toHaveCount(2);
  await expect(marks.first()).toHaveText("(");
  await page.keyboard.press(keys.matchingBrace);
  await expect(page.getByTestId("status-position")).toHaveText("1:21");

  await page.keyboard.press("End");
  await page.keyboard.type("g(");
  await expect.poll(() => editorText(page)).toContain(";g()");
  await page.keyboard.type(")"); // steps over the auto-inserted ")"
  await expect.poll(() => editorText(page)).toContain(";g()\n");
  await page.keyboard.type("[");
  await page.keyboard.press("Backspace"); // deletes the empty pair
  await page.keyboard.type('"x');
  await expect.poll(() => editorText(page)).toContain(';g()"x"');
});

test("EditorConfig drives indentation and save", async ({ page, project }) => {
  fs.writeFileSync(
    path.join(project, ".editorconfig"),
    "root = true\n[*.ts]\nindent_style = space\nindent_size = 4\ntrim_trailing_whitespace = true\ninsert_final_newline = true\n",
  );
  fs.writeFileSync(path.join(project, "e.ts"), "x  ");
  await openApp(page, project);
  await openFile(page, "e.ts");
  await caretToStart(page);
  await page.keyboard.press("Tab");
  await page.keyboard.type("y");
  await expect(page.getByTestId("status-state")).toHaveText("Modified");
  await page.keyboard.press("ControlOrMeta+s");
  await expect.poll(() => fs.readFileSync(path.join(project, "e.ts"), "utf8")).toBe("    yx\n");
  await expect(page.getByTestId("status-state")).toHaveText("Saved");
  await expect.poll(() => editorText(page)).toBe("    yx\n");
  await expect(activeTab(page).getByTestId("tab-dirty")).toHaveText("");
});

test("ignored entries are greyed in the tree", async ({ page, project }) => {
  fs.mkdirSync(path.join(project, "dist"));
  fs.mkdirSync(path.join(project, "node_modules"), { recursive: true });
  await openApp(page, project);
  await expect(treeRow(page, "dist")).toHaveAttribute("data-ignored", "true");
  await expect(treeRow(page, "node_modules")).toHaveAttribute("data-ignored", "true");
  await expect(treeRow(page, "src")).not.toHaveAttribute("data-ignored", "true");
});

test("Vue, Markdown, YAML and SCSS are highlighted", async ({ page, project }) => {
  fs.writeFileSync(
    path.join(project, "C.vue"),
    '<template><p>hi</p></template>\n<script lang="ts">\nconst n: number = 1;\n</script>\n',
  );
  fs.writeFileSync(path.join(project, "notes.md"), "# Notes\n\nSome *text*.\n");
  fs.writeFileSync(path.join(project, "ci.yaml"), "name: build\nretries: 3\n");
  fs.writeFileSync(path.join(project, "t.scss"), "$gap: 4px;\n.a { margin: $gap; }\n");
  await openApp(page, project);
  const tok = (cls: string, text: string) =>
    page.locator(`.editor-host:not([hidden]) .tok-${cls}`, { hasText: text }).first();

  await openFile(page, "C.vue");
  await expect(tok("keyword", "const")).toBeVisible();
  await expect(tok("tag", "template")).toBeVisible();
  await openFile(page, "notes.md");
  await expect(tok("heading", "Notes")).toBeVisible();
  await expect(tok("emphasis", "text")).toBeVisible();
  await openFile(page, "ci.yaml");
  await expect(tok("property", "retries")).toBeVisible();
  await openFile(page, "t.scss");
  await expect(tok("variable", "$gap")).toBeVisible();
});
