// Run configurations, the Run window and the test runner (M5a) on a copy of
// fixtures/test-runners. By default runs use the in-memory run backend of
// the test server (recorded Vitest/Playwright reports, no real tools);
// with FERRULE_E2E_REAL_RUNS=1 they start real Node and Vitest, with the
// repository's node_modules linked into the project copy. The results are
// the same either way: the reports were recorded from this fixture.
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";

import type { Page } from "@playwright/test";

import { repoRoot } from "../paths";
import {
  activeTab,
  backend,
  expect,
  isMac,
  keys as appKeys,
  openApp,
  openFile,
  test,
} from "./harness";

test.use({ fixture: "test-runners" });

const real = process.env.FERRULE_E2E_REAL_RUNS === "1";
const keys = {
  run: isMac ? "Control+r" : "Shift+F10",
  runContext: isMac ? "Control+Shift+r" : "Control+Shift+F10",
  stop: isMac ? "Meta+F2" : "Control+F2",
  rerun: isMac ? "Meta+r" : "Control+F5",
  choose: isMac ? "Control+Alt+r" : "Alt+Shift+F10",
};

function hasNode(): boolean {
  try {
    execFileSync("node", ["--version"]);
    return true;
  } catch {
    return false;
  }
}

/**
 * Makes the test tools findable: the real packages (linked) for real runs,
 * else package.json files with the `bin` entries the command lines name.
 */
function prepareTools(project: string): void {
  // Commands start `node <tool>`; Node must be on PATH either way.
  test.skip(!hasNode(), "node is not on PATH");
  const modules = path.join(project, "node_modules");
  if (real) {
    const target = path.join(repoRoot, "node_modules");
    test.skip(
      !fs.existsSync(path.join(target, "vitest", "package.json")),
      "vitest is not installed in the repository (bun install)",
    );
    fs.symlinkSync(target, modules, "junction");
    return;
  }
  for (const [name, bin, script] of [
    ["vitest", "vitest", "vitest.mjs"],
    ["@playwright/test", "playwright", "cli.js"],
  ] as const) {
    const dir = path.join(modules, name);
    fs.mkdirSync(dir, { recursive: true });
    fs.writeFileSync(
      path.join(dir, "package.json"),
      JSON.stringify({ name, bin: { [bin]: script } }),
    );
    fs.writeFileSync(path.join(dir, script), "");
  }
}

function panel(page: Page) {
  return page.locator('[data-testid="run-panel"]:not([hidden])');
}

async function output(page: Page): Promise<string> {
  const rows = await panel(page).locator('[data-testid="terminal-row"]').allTextContents();
  return rows.join("\n");
}

/**
 * Center of the character `offset` into `text` on the last output row of
 * the visible run that shows it, from that character's own DOM rectangle.
 * Found and measured in one step in the page (a row element replaced in
 * between would have no box), retried until the row is laid out.
 */
async function outputPoint(
  page: Page,
  text: string,
  offset: number,
): Promise<{ x: number; y: number }> {
  const rows = panel(page).locator('[data-testid="terminal-row"]', { hasText: text });
  await expect(rows.last()).toBeVisible();
  const found: { point: { x: number; y: number } | null } = { point: null };
  await expect
    .poll(async () => {
      found.point = await panel(page).evaluate(
        (el, { text, offset }) => {
          const rows = [...el.querySelectorAll('[data-testid="terminal-row"]')].filter((r) =>
            r.textContent.includes(text),
          );
          const row = rows[rows.length - 1];
          if (!row) return null;
          let index = row.textContent.lastIndexOf(text) + offset;
          const walker = document.createTreeWalker(row, NodeFilter.SHOW_TEXT);
          for (let node = walker.nextNode(); node; node = walker.nextNode()) {
            const length = node.textContent?.length ?? 0;
            if (index < length) {
              const range = document.createRange();
              range.setStart(node, index);
              range.setEnd(node, index + 1);
              const rect = range.getBoundingClientRect();
              if (rect.width === 0 || rect.height === 0) return null;
              return { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 };
            }
            index -= length;
          }
          return null;
        },
        { text, offset },
      );
      return found.point !== null;
    })
    .toBe(true);
  if (!found.point) throw new Error(`no position for "${text}"`);
  return found.point;
}

/** Runs a detected or stored configuration from the chooser. */
async function choose(page: Page, name: string): Promise<void> {
  await page.keyboard.press(keys.choose);
  await expect(page.getByTestId("run-chooser")).toBeVisible();
  await page.getByTestId("run-config-filter").fill(name);
  await expect(page.getByTestId("run-config-item").first()).toContainText(name);
  await page.keyboard.press("Enter");
}

test("the first run shows the command and asks for trust", async ({ page, project }) => {
  prepareTools(project);
  await openApp(page, project);
  await choose(page, "Vitest");
  const dialog = page.getByTestId("trust-dialog");
  await expect(dialog).toBeVisible();
  await expect(page.getByTestId("trust-command")).toContainText("vitest.mjs run");
  await expect(page.getByTestId("trust-command")).toContainText("--reporter=json");
  await expect(page.getByTestId("trust-cwd")).toContainText(path.basename(project));

  // Cancel: nothing runs.
  await page.getByTestId("trust-cancel").click();
  await expect(dialog).toBeHidden();
  await expect(page.getByTestId("run-tab")).toHaveCount(0);
  await expect(page.getByTestId("status-notice")).toContainText("not trusted");
  expect(fs.existsSync(path.join(project, ".ferrule"))).toBe(false);

  // Shift+F10 runs the chosen configuration; trust it this time.
  await page.keyboard.press(keys.run);
  await page.getByTestId("trust-confirm").click();
  await expect(page.getByTestId("run-tab")).toHaveCount(1);
  await expect(panel(page).getByTestId("run-summary")).toHaveText(
    "1 failed, 3 passed, 1 skipped of 5 tests",
    { timeout: 60_000 },
  );
  await expect(panel(page).getByTestId("run-status")).toContainText("Exit code 1");

  // The first failure is selected; its stack location opens the file.
  const failed = panel(page).locator(
    '[data-testid="test-row"][data-kind="test"][data-status="failed"]',
  );
  await expect(failed).toHaveAttribute("data-name", "math > divides");
  await expect(panel(page).getByTestId("run-failure")).toContainText("expected 0.25");
  await panel(page).getByTestId("run-frame").filter({ hasText: "src/math.test.ts:12:26" }).click();
  await expect(activeTab(page)).toHaveAttribute("data-path", "src/math.test.ts");
  await expect(page.getByTestId("status-position")).toHaveText("12:26");

  // Trusted now: a rerun starts at once.
  await panel(page).getByTestId("run-rerun").click();
  await expect(page.getByTestId("run-tab")).toHaveCount(2);
  await expect(page.getByTestId("trust-dialog")).toHaveCount(0);
});

test("Revoke Project Trust makes the next run ask again", async ({ page, project }) => {
  prepareTools(project);
  await openApp(page, project);
  await backend("trust_project", {});
  await choose(page, "Vitest");
  await expect(page.getByTestId("run-tab")).toHaveCount(1);
  await expect(page.getByTestId("trust-dialog")).toHaveCount(0);

  // The run's output has the focus and, like a terminal, keeps typed keys
  // for the program; Go to Action works from the rest of the window.
  await page.getByTestId("project-tree").click();
  await page.keyboard.press(appKeys.gotoAction);
  await page.getByTestId("se-input").fill("Revoke Project Trust");
  await expect(page.locator('[data-type="action"]').first()).toContainText("Revoke Project Trust");
  await page.keyboard.press("Enter");
  await expect(page.getByTestId("status-notice")).toContainText("trust revoked");
  const configs = (await backend("run_configurations", {})) as { trusted: boolean };
  expect(configs.trusted).toBe(false);

  // Shift+F10 reruns the selected configuration: the question is back.
  await page.keyboard.press(keys.run);
  await expect(page.getByTestId("trust-dialog")).toBeVisible();
  await expect(page.getByTestId("trust-command")).toContainText("vitest.mjs run");
  await page.getByTestId("trust-cancel").click();
  await expect(page.getByTestId("run-tab")).toHaveCount(1);
});

test("gutter markers run single tests and show their status", async ({ page, project }) => {
  prepareTools(project);
  await openApp(page, project);
  await backend("trust_project", {});
  await openFile(page, "src/math.test.ts");
  const marker = (line: number) =>
    page.locator(
      `.editor-host:not([hidden]) [data-testid="test-marker"][data-line="${String(line)}"]`,
    );
  // describe("math") on line 5, it("divides") on line 10 (zero-based 4, 9).
  await expect(marker(4)).toHaveAttribute("data-status", "none");
  await marker(9).click();
  await expect(page.getByTestId("run-tab")).toContainText("divides");
  await expect(panel(page).getByTestId("run-status")).toContainText("Exit code 1", {
    timeout: 60_000,
  });
  await expect(marker(9)).toHaveAttribute("data-status", "failed");
  await expect(marker(4)).toHaveAttribute("data-status", "failed");
  // The fake backend reports every test whatever ran.
  if (!real) await expect(marker(5)).toHaveAttribute("data-status", "passed");

  // Ctrl+Shift+F10 on a line of "adds" runs that test.
  await page.locator(".editor-host:not([hidden]) .cm-line").nth(6).click();
  await page.keyboard.press(keys.runContext);
  await expect(page.getByTestId("run-tab").last()).toContainText("adds");
});

test("the test tree filters, reruns failed and single tests", async ({ page, project }) => {
  prepareTools(project);
  await openApp(page, project);
  await backend("trust_project", {});
  await choose(page, "Vitest");
  await expect(panel(page).getByTestId("run-summary")).toContainText("1 failed", {
    timeout: 60_000,
  });
  const tests = panel(page).locator('[data-testid="test-row"][data-kind="test"]');
  await expect(tests).toHaveCount(5);

  // Hide passed and skipped tests: the failed one (with its suite and file) stays.
  await panel(page).getByTestId("run-filter-passed").click();
  await panel(page).getByTestId("run-filter-skipped").click();
  await expect(panel(page).getByTestId("run-filter-passed")).toHaveAttribute(
    "aria-pressed",
    "false",
  );
  await expect(tests).toHaveCount(1);
  await expect(tests).toHaveAttribute("data-name", "math > divides");

  await panel(page).getByTestId("run-rerun-failed").click();
  await expect(page.getByTestId("run-tab")).toHaveCount(2);
  await expect(page.getByTestId("run-tab").last()).toContainText("(failed)");
  await expect(panel(page).getByTestId("run-command")).toContainText("-t");

  // Back to the first run: run one passed test again from its details.
  await page.getByTestId("run-tab").first().locator(".terminal-tab-title").click();
  await panel(page).getByTestId("run-filter-passed").click();
  await panel(page).locator('[data-testid="test-row"][data-name="math > adds"]').click();
  await panel(page).getByTestId("run-test-again").click();
  await expect(page.getByTestId("run-tab")).toHaveCount(3);
  await expect(page.getByTestId("run-tab").last()).toContainText("(adds)");
});

test("a script runs with output links; Stop keeps the output; Rerun", async ({ page, project }) => {
  prepareTools(project);
  await openApp(page, project);
  await backend("trust_project", {});

  // Ctrl+Shift+F10 in a plain file runs it with Node.
  await openFile(page, "src/main.js");
  await page.keyboard.press(keys.runContext);
  await expect(page.getByTestId("run-tab")).toContainText("src/main.js");
  await expect(panel(page).getByTestId("run-status")).toContainText("Exit code 0", {
    timeout: 60_000,
  });
  await expect.poll(() => output(page)).toContain("see src/math.ts:5:10");
  // Ctrl/Cmd+click on the location opens it (position from the DOM).
  const point = await outputPoint(page, "src/math.ts:5:10", 3);
  await page.keyboard.down(isMac ? "Meta" : "Control");
  await page.mouse.click(point.x, point.y);
  await page.keyboard.up(isMac ? "Meta" : "Control");
  await expect(activeTab(page)).toHaveAttribute("data-path", "src/math.ts");
  await expect(page.getByTestId("status-position")).toHaveText("5:10");

  // A dev server runs until stopped (Ctrl+F2); its output stays.
  await choose(page, "dev");
  await expect.poll(() => output(page), { timeout: 60_000 }).toContain("serving until stopped");
  await expect(panel(page).getByTestId("run-status")).toHaveText("Running…");
  await page.keyboard.press(keys.stop);
  await expect(panel(page).getByTestId("run-status")).toContainText("Stopped", {
    timeout: 20_000,
  });
  expect(await output(page)).toContain("serving until stopped");

  // Ctrl+F5 reruns it in a new tab; closing the tab ends it.
  await page.keyboard.press(keys.rerun);
  await expect(page.getByTestId("run-tab")).toHaveCount(3);
  await expect.poll(() => output(page), { timeout: 60_000 }).toContain("serving until stopped");
  await page.getByTestId("run-tab").last().getByTestId("run-tab-close").click();
  await expect(page.getByTestId("run-tab")).toHaveCount(2);
  await expect.poll(async () => ((await backend("runs", {})) as unknown[]).length).toBe(2);
});

test("configurations are edited and stored in .ferrule/run.json", async ({ page, project }) => {
  prepareTools(project);
  await openApp(page, project);
  await page.keyboard.press(keys.choose);
  await page.getByTestId("run-config-filter").fill("start");
  await expect(page.getByTestId("run-config-item").first()).toContainText("detected");
  await page.keyboard.press("F4");
  const editor = page.getByTestId("run-editor");
  await expect(editor).toBeVisible();
  await expect(page.getByTestId("run-form-kind")).toHaveValue("npmScript");
  await expect(page.getByTestId("run-form-target")).toHaveValue("start");
  await page.getByTestId("run-form-name").fill("Start app");
  await page.getByTestId("run-form-args").fill("--port\n3000");
  await page.getByTestId("run-form-env").fill("broken line");
  await page.getByTestId("run-form-save").click();
  await expect(page.getByTestId("run-config-error")).toContainText("NAME=value");
  await page.getByTestId("run-form-env").fill("DEBUG=1");
  await page.getByTestId("run-form-save").click();
  await expect(editor).toBeHidden();

  const runJson = path.join(project, ".ferrule", "run.json");
  const stored = JSON.parse(fs.readFileSync(runJson, "utf8")) as {
    configurations: { name: string; args: string[] }[];
  };
  expect(stored.configurations).toHaveLength(1);
  expect(stored.configurations[0]?.name).toBe("Start app");
  expect(stored.configurations[0]?.args).toEqual(["--port", "3000"]);
  await expect(page.getByTestId("run-config-select")).toHaveText("Start app");

  // It is listed as saved, and Delete removes it.
  await page.keyboard.press(keys.choose);
  await page.getByTestId("run-config-filter").fill("Start app");
  await expect(page.getByTestId("run-config-item").first()).toContainText("saved");
  await page.keyboard.press("F4");
  await page.getByTestId("run-form-delete").click();
  await expect(editor).toBeHidden();
  const after = JSON.parse(fs.readFileSync(runJson, "utf8")) as { configurations: unknown[] };
  expect(after.configurations).toHaveLength(0);
});
