import fs from "node:fs";
import path from "node:path";

import { expect, openApp, test, treeRow } from "./harness";

test("shows the project tree and expands folders lazily", async ({ page, project }) => {
  await openApp(page, project);
  await expect(page.getByTestId("tree-row")).toHaveText([
    "public",
    "src",
    ".gitignore",
    "index.html",
    "package.json",
    "README.md",
    "tsconfig.json",
  ]);
  await expect(treeRow(page, "src/App.tsx")).toHaveCount(0);

  await treeRow(page, "src").click();
  await expect(treeRow(page, "src")).toHaveAttribute("aria-expanded", "true");
  await expect(treeRow(page, "src/components")).toBeVisible();
  await expect(treeRow(page, "src/App.tsx")).toBeVisible();

  await treeRow(page, "src/components").click();
  await expect(treeRow(page, "src/components/Counter.tsx")).toBeVisible();

  await treeRow(page, "src").click();
  await expect(treeRow(page, "src/App.tsx")).toHaveCount(0);
});

test("renders only the visible rows of a large folder", async ({ page, project }) => {
  const many = path.join(project, "many");
  fs.mkdirSync(many);
  for (let i = 0; i < 2000; i++) {
    fs.writeFileSync(path.join(many, `file-${String(i).padStart(4, "0")}.ts`), "");
  }
  await openApp(page, project);
  await treeRow(page, "many").click();
  await expect(treeRow(page, "many/file-0000.ts")).toBeVisible();

  const rendered = await page.getByTestId("tree-row").count();
  expect(rendered).toBeLessThan(120);

  await page.getByTestId("project-tree").evaluate((el) => {
    el.scrollTop = el.scrollHeight;
  });
  await expect(treeRow(page, "many/file-1999.ts")).toBeVisible();
  await expect(treeRow(page, "many/file-0000.ts")).toHaveCount(0);
});
