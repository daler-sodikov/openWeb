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
  treeRow,
} from "./harness";

test("tree and unmodified tabs follow changes on disk", async ({ page, project }) => {
  await openApp(page, project);
  await openFile(page, "src/utils/format.ts");
  const file = path.join(project, "src/utils/format.ts");

  fs.writeFileSync(file, "// changed outside\nexport const x = 1;\n");
  await expect(editor(page)).toContainText("changed outside");
  await expect(activeTab(page).getByTestId("tab-dirty")).toHaveText("");
  await expect(page.getByTestId("external-change")).toHaveCount(0);

  fs.writeFileSync(path.join(project, "src/created.ts"), "export {};\n");
  await expect(treeRow(page, "src/created.ts")).toBeVisible();
  fs.rmSync(path.join(project, "src/created.ts"));
  await expect(treeRow(page, "src/created.ts")).toHaveCount(0);
});

test("a modified tab asks before reloading", async ({ page, project }) => {
  await openApp(page, project);
  await openFile(page, "src/utils/format.ts");
  const file = path.join(project, "src/utils/format.ts");
  await caretToStart(page);
  await page.keyboard.type("// mine\n");
  await expect(activeTab(page).getByTestId("tab-dirty")).toHaveText("●");

  fs.writeFileSync(file, "// theirs 1\n");
  const bar = page.getByTestId("external-change");
  await expect(bar).toBeVisible();
  await page.getByTestId("keep-changes").click();
  await expect(bar).toHaveCount(0);
  await expect(editor(page)).toContainText("// mine");

  fs.writeFileSync(file, "// theirs 2\n");
  await expect(bar).toBeVisible();
  await page.getByTestId("reload-from-disk").click();
  await expect.poll(() => editorText(page)).toBe("// theirs 2\n");
  await expect(activeTab(page).getByTestId("tab-dirty")).toHaveText("");
});

test("saving does not trigger the changed-on-disk prompt", async ({ page, project }) => {
  await openApp(page, project);
  await openFile(page, "src/utils/format.ts");
  await caretToStart(page);
  await page.keyboard.type("a");
  await page.keyboard.press("ControlOrMeta+s");
  await page.keyboard.type("b");
  // Give the watcher time to report our own save.
  await page.waitForTimeout(1000);
  await expect(page.getByTestId("external-change")).toHaveCount(0);
  await expect(editor(page)).toContainText("ab// Formatting helpers.");
});
