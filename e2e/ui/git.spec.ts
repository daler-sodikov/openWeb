// Git (M4b) on a real repository made from the fixture in each test's
// temp copy (local git commands only).
import fs from "node:fs";
import path from "node:path";

import type { Page } from "@playwright/test";

import {
  activeTab,
  caretToStart,
  editorText,
  expect,
  git,
  initGitRepo as initRepo,
  isMac,
  keys,
  openApp,
  openFile,
  test,
  treeRow,
} from "./harness";

const commitKey = isMac ? "Meta+0" : "Alt+0";
const logKey = isMac ? "Meta+9" : "Alt+9";

async function runAction(page: Page, name: string): Promise<void> {
  await page.keyboard.press(keys.gotoAction);
  await page.getByTestId("se-input").fill(name);
  await expect(page.locator('[data-type="action"]').first()).toContainText(name);
  await page.keyboard.press("Enter");
}

async function waitForBranch(page: Page, name: string): Promise<void> {
  await expect(page.getByTestId("status-branch")).toHaveText(`⎇ ${name}`, { timeout: 15_000 });
}

test("the tree shows Git status colors and follows changes", async ({ page, project }) => {
  initRepo(project);
  fs.appendFileSync(path.join(project, "src/App.tsx"), "// changed\n");
  fs.writeFileSync(path.join(project, "src/new-file.ts"), "export {};\n");
  await openApp(page, project);
  await waitForBranch(page, "main");
  await expect(treeRow(page, "src")).toHaveAttribute("data-git", "changed");
  await treeRow(page, "src").click();
  await expect(treeRow(page, "src/App.tsx")).toHaveAttribute("data-git", "modified");
  await expect(treeRow(page, "src/new-file.ts")).toHaveAttribute("data-git", "untracked");
  await expect(treeRow(page, "src/main.tsx")).not.toHaveAttribute("data-git", /.*/);

  // A change made outside Ferrule shows up (watcher, then focus refresh).
  fs.appendFileSync(path.join(project, "src/main.tsx"), "// outside\n");
  await expect(treeRow(page, "src/main.tsx")).toHaveAttribute("data-git", "modified", {
    timeout: 15_000,
  });
});

test("gutter markers follow edits; Rollback restores the hunk in one undo step", async ({
  page,
  project,
}) => {
  initRepo(project);
  await openApp(page, project);
  await waitForBranch(page, "main");
  await openFile(page, "src/App.tsx");
  const original = await editorText(page);
  await caretToStart(page);
  await page.keyboard.type("// new line\n");
  const marker = page.locator('[data-testid="git-marker"][data-kind="added"]');
  await expect(marker).toHaveCount(1);

  // Edit an existing line: a modified marker whose popup shows the old text.
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("End");
  await page.keyboard.type(" // edited");
  const modified = page.locator('[data-testid="git-marker"][data-kind="modified"]');
  await expect(modified).toHaveCount(1);
  await modified.click();
  await expect(page.getByTestId("git-hunk-old")).toContainText('from "./utils/format"');
  await page.getByTestId("git-hunk-rollback").click();
  await expect(modified).toHaveCount(0);
  expect(await editorText(page)).toBe(`// new line\n${original}`);

  // The rollback is one undo step.
  await page.locator(".cm-content").focus();
  await page.keyboard.press(keys.undo);
  await expect(modified).toHaveCount(1);

  // Ctrl+Alt+Z (Cmd+Alt+Z) rolls back the change at the caret.
  await caretToStart(page);
  await page.keyboard.press("ControlOrMeta+Alt+z");
  await expect(marker).toHaveCount(0);
});

test("commit window: stage, diff, commit, errors and amend", async ({ page, project }) => {
  initRepo(project);
  fs.writeFileSync(
    path.join(project, "src/utils/format.ts"),
    fs
      .readFileSync(path.join(project, "src/utils/format.ts"), "utf8")
      .replace("export", "// changed\nexport"),
  );
  fs.writeFileSync(path.join(project, "notes.md"), "notes\n");
  await openApp(page, project);
  await waitForBranch(page, "main");
  await page.keyboard.press(commitKey);
  await expect(page.getByTestId("commit")).toBeVisible();
  const row = (p: string) => page.locator(`[data-testid="commit-file"][data-path="${p}"]`);
  await expect(row("src/utils/format.ts")).toBeVisible();
  await expect(row("notes.md")).toBeVisible();

  // Diff from the commit window: side by side, then unified.
  await row("src/utils/format.ts").dblclick();
  await expect(page.getByTestId("diff-view")).toBeVisible();
  await expect(page.getByTestId("diff-new-label")).toHaveText("Working tree");
  await expect(page.locator(".diff-added").first()).toContainText("// changed");
  await page.getByTestId("diff-mode").click();
  await expect(page.locator('[data-testid="diff-row"][data-kind="added"]')).toContainText(
    "// changed",
  );
  await page.keyboard.press("Escape");

  // Commit without a message or files: clear errors.
  await page.getByTestId("commit-button").click();
  await expect(page.getByTestId("commit-error")).toHaveText("Enter a commit message.");
  await page.getByTestId("commit-message").fill("Change format");
  await page.getByTestId("commit-button").click();
  await expect(page.getByTestId("commit-error")).toContainText("Nothing to commit");

  // Missing identity is explained.
  await row("src/utils/format.ts").getByTestId("commit-check").check();
  await expect(row("src/utils/format.ts").getByTestId("commit-check")).toBeChecked();
  git(project, "config", "user.name", "");
  git(project, "config", "user.useConfigOnly", "true");
  await page.getByTestId("commit-button").click();
  await expect(page.getByTestId("commit-error")).toContainText("user.name");
  git(project, "config", "user.name", "Test User");

  await page.getByTestId("commit-button").click();
  await expect(page.getByTestId("status-notice")).toContainText("Committed");
  await expect(row("src/utils/format.ts")).toHaveCount(0);
  await expect(row("notes.md")).toBeVisible();
  expect(git(project, "log", "-1", "--format=%s").trim()).toBe("Change format");

  // Amend fills in the last message and rewrites the commit.
  await page.getByTestId("commit-amend").check();
  await expect(page.getByTestId("commit-message")).toHaveValue("Change format");
  await page.getByTestId("commit-message").fill("Change format, amended");
  await page.getByTestId("commit-button").click();
  await expect(page.getByTestId("status-notice")).toContainText("Committed");
  expect(git(project, "log", "--format=%s").trim().split("\n")).toEqual([
    "Change format, amended",
    "Initial commit",
  ]);

  // The history lists it.
  await page.keyboard.press(logKey);
  await expect(page.getByTestId("log-commit").first()).toContainText("Change format, amended");
  await expect(page.getByTestId("log-commit")).toHaveCount(2);
});

test("branches popup: create, checkout guard, checkout, delete", async ({ page, project }) => {
  initRepo(project);
  await openApp(page, project);
  await waitForBranch(page, "main");
  await page.getByTestId("status-branch").click();
  await expect(page.getByTestId("branches")).toBeVisible();
  await expect(page.locator('[data-testid="branch-item"][data-current="true"]')).toHaveAttribute(
    "data-name",
    "main",
  );
  await page.locator('[data-testid="branch-item"][data-name=""]').click();
  await page.getByTestId("branch-name").fill("feature");
  await page.keyboard.press("Enter");
  await waitForBranch(page, "feature");

  // A commit only on feature.
  fs.appendFileSync(path.join(project, "README.md"), "feature\n");
  git(project, "commit", "-q", "-am", "Feature work");

  // Unsaved edits block a checkout.
  await openFile(page, "src/App.tsx");
  await caretToStart(page);
  await page.keyboard.type("x");
  await page.getByTestId("status-branch").click();
  await page.getByTestId("branches-input").fill("main");
  await page.keyboard.press("Enter");
  await expect(page.getByTestId("branches-message")).toContainText("unsaved");
  await page.keyboard.press("Escape");
  await page.keyboard.press(keys.undo);
  await page.keyboard.press("ControlOrMeta+s");

  await page.getByTestId("status-branch").click();
  await page.getByTestId("branches-input").fill("main");
  await page.keyboard.press("Enter");
  await waitForBranch(page, "main");
  await expect(activeTab(page)).toHaveAttribute("data-path", "src/App.tsx");

  // Deleting the unmerged branch asks again.
  await page.getByTestId("status-branch").click();
  await page
    .locator('[data-testid="branch-item"][data-name="feature"]')
    .getByTestId("branch-delete")
    .click();
  await expect(page.getByTestId("branches-message")).toContainText("not fully merged");
  await page.getByTestId("branch-force-delete").click();
  await expect(page.locator('[data-testid="branch-item"][data-name="feature"]')).toHaveCount(0);
  expect(git(project, "branch", "--format=%(refname:short)").trim()).toBe("main");
});

test("blame shows authors in the gutter", async ({ page, project }) => {
  initRepo(project);
  await openApp(page, project);
  await waitForBranch(page, "main");
  await openFile(page, "src/App.tsx");
  await caretToStart(page);
  await page.keyboard.type("// mine\n");
  await runAction(page, "Annotate with Git Blame");
  const blame = page.getByTestId("git-blame");
  await expect(blame.nth(1)).toContainText("Test");
  await expect(blame.nth(1)).toHaveAttribute("title", /Initial commit/);
  // The new line is not committed yet: no label.
  await expect(blame.first()).toHaveText("");
  await runAction(page, "Annotate with Git Blame");
  await expect(blame).toHaveCount(0);
});

test("a folder that is not a repository has no Git UI", async ({ page, project }) => {
  await openApp(page, project);
  await expect(page.getByTestId("status-branch")).toBeHidden();
  await page.keyboard.press(commitKey);
  await expect(page.getByTestId("commit")).toContainText("not in a Git repository");
});
