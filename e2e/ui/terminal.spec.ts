// Terminal tool window (M4a). Real shells need the native backend: these
// tests skip when the headless backend was built without it
// (FERRULE_E2E_NO_NATIVE=1) and fail on CI, which always builds it. With
// FERRULE_E2E_FAKE_TERMINAL=1 they run against the in-memory backend
// (its "shell" only echoes input), which checks the UI without real shells;
// steps that need a real shell are skipped then.
import type { Page } from "@playwright/test";

import { activeTab, backend, expect, isMac, openApp, test } from "./harness";

const toggle = "Alt+F12";
const fake = process.env.FERRULE_E2E_FAKE_TERMINAL === "1";

/** Why terminals cannot run, or null. */
async function terminalsUnavailable(): Promise<string | null> {
  const list = (await backend("terminals", {})) as { unavailable: string | null };
  return list.unavailable;
}

async function requireTerminals(): Promise<void> {
  const reason = await terminalsUnavailable();
  if (reason && !process.env.CI) test.skip(true, reason);
  expect(reason).toBeNull();
}

function view(page: Page) {
  return page.locator('[data-testid="terminal-view"]:not([hidden])');
}

async function screenText(page: Page): Promise<string> {
  const rows = await view(page).locator('[data-testid="terminal-row"]').allTextContents();
  return rows.join("\n");
}

/** Waits for `text` as a whole line of output (the fake shell only echoes, so anywhere). */
async function expectOutput(page: Page, text: string): Promise<void> {
  const escaped = text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const pattern = fake ? new RegExp(escaped) : new RegExp(`^${escaped}\\s*$`, "m");
  await expect.poll(() => screenText(page), { timeout: 20_000 }).toMatch(pattern);
}

/** Center of the cell `offset` characters into `text` on the last row showing it. */
async function cellOf(page: Page, text: string, offset = 0): Promise<{ x: number; y: number }> {
  const row = view(page).locator('[data-testid="terminal-row"]', { hasText: text }).last();
  await expect(row).toBeVisible();
  const box = await row.boundingBox();
  if (!box) throw new Error("row not visible");
  const rowText = (await row.textContent()) ?? "";
  const cell = await view(page).evaluate((el) =>
    Number.parseFloat(getComputedStyle(el).getPropertyValue("--term-cell-w")),
  );
  const col = rowText.lastIndexOf(text) + offset;
  return { x: box.x + cell * (col + 0.5), y: box.y + box.height / 2 };
}

/** Types a command and presses Enter. */
async function run(page: Page, command: string): Promise<void> {
  await page.keyboard.type(command);
  await page.keyboard.press("Enter");
}

async function openTerminal(page: Page): Promise<void> {
  await page.keyboard.press(toggle);
  await expect(page.getByTestId("terminal")).toBeVisible();
  await expect(page.getByTestId("terminal-tab")).toHaveCount(1);
  await expect(page.getByTestId("terminal-input")).toBeFocused();
  // The shell is ready once it has drawn its prompt.
  await expect.poll(() => screenText(page), { timeout: 20_000 }).not.toMatch(/^\s*$/);
}

test("says why terminals are unavailable in a build without them", async ({ page, project }) => {
  await openApp(page, project);
  const reason = await terminalsUnavailable();
  test.skip(reason === null, "this backend has terminals");
  await page.keyboard.press(toggle);
  await expect(page.getByTestId("terminal-unavailable")).toContainText("native");
  await expect(page.getByTestId("terminal-tab")).toHaveCount(0);
});

test("Alt+F12 opens a shell in the project root; keys reach it", async ({ page, project }) => {
  await openApp(page, project);
  await requireTerminals();
  await openTerminal(page);

  // Typed fast: every key arrives, in order.
  await run(page, "echo hello-ferrule");
  await expectOutput(page, "hello-ferrule");
  if (!fake) {
    await run(page, "pwd");
    await expect.poll(() => screenText(page)).toContain("react-ts");
    // History: Up brings the last command back.
    await page.keyboard.press("ArrowUp");
    await page.keyboard.press("ArrowUp");
    await page.keyboard.press("Enter");
    await expect
      .poll(async () => (await screenText(page)).match(/^hello-ferrule\s*$/gm)?.length ?? 0)
      .toBe(2);
  }

  // Editor shortcuts do not fire while the terminal has focus.
  await page.keyboard.press("ControlOrMeta+Shift+A");
  await expect(page.locator(".popup")).toHaveCount(0);

  // Alt+F12 in the focused terminal hides it; again shows the same tab.
  await page.keyboard.press(toggle);
  await expect(page.getByTestId("terminal")).toBeHidden();
  await page.keyboard.press(toggle);
  await expect(page.getByTestId("terminal-tab")).toHaveCount(1);
  await expect(page.getByTestId("terminal-input")).toBeFocused();
  await expectOutput(page, "hello-ferrule");
});

test("tabs: new, rename, close; an exited shell shows its code", async ({ page, project }) => {
  await openApp(page, project);
  await requireTerminals();
  await openTerminal(page);

  // Typing right after "+" goes to the new shell, nothing is lost.
  await page.getByTestId("terminal-new").click();
  await run(page, "echo second-tab");
  await expect(page.getByTestId("terminal-tab")).toHaveCount(2);
  await expectOutput(page, "second-tab");
  await expect(page.getByTestId("terminal-tab").nth(1)).toHaveAttribute("aria-selected", "true");

  await page.getByTestId("terminal-tab-title").nth(1).dblclick();
  await page.getByTestId("terminal-tab-rename").fill("build");
  await page.getByTestId("terminal-tab-rename").press("Enter");
  await expect(page.getByTestId("terminal-tab-title").nth(1)).toHaveText("build");
  expect(
    ((await backend("terminals", {})) as { terminals: { title: string }[] }).terminals.map(
      (t) => t.title,
    ),
  ).toContain("build");

  await page.getByTestId("terminal-tab-close").nth(1).click();
  await expect(page.getByTestId("terminal-tab")).toHaveCount(1);
  expect(((await backend("terminals", {})) as { terminals: unknown[] }).terminals).toHaveLength(1);

  // Ctrl+Shift+T (Cmd+T) opens another tab from the keyboard.
  await page.getByTestId("terminal-input").focus();
  await page.keyboard.press(isMac ? "Meta+t" : "Control+Shift+T");
  await expect(page.getByTestId("terminal-tab")).toHaveCount(2);
  await page.getByTestId("terminal-tab-close").nth(1).click();
  await expect(page.getByTestId("terminal-tab")).toHaveCount(1);

  // An exited shell keeps its tab and shows the exit code.
  await page.getByTestId("terminal-input").focus();
  await run(page, "exit");
  await expect(view(page).getByTestId("terminal-exit")).toHaveText("Process exited with code 0.", {
    timeout: 20_000,
  });
  await expect(page.getByTestId("terminal-tab")).toHaveAttribute("data-exited", "true");
  await page.getByTestId("terminal-tab-close").click();
  await expect(page.getByTestId("terminal-tab")).toHaveCount(0);
  await expect(page.getByTestId("terminal")).toBeHidden();
});

test("Ctrl/Cmd+click on file:line:col opens the file", async ({ page, project }) => {
  await openApp(page, project);
  await requireTerminals();
  await openTerminal(page);
  await run(page, `echo "at src/App.tsx:3:5 here"`);
  await expectOutput(page, "at src/App.tsx:3:5 here");
  const point = await cellOf(page, "src/App.tsx:3:5", 4);
  await page.keyboard.down(isMac ? "Meta" : "Control");
  // Hovering with Ctrl/Cmd underlines the whole link.
  await page.mouse.move(point.x, point.y);
  await expect(view(page).getByTestId("terminal-link")).toBeVisible();
  const cell = await view(page).evaluate((el) =>
    Number.parseFloat(getComputedStyle(el).getPropertyValue("--term-cell-w")),
  );
  const width = (await view(page).getByTestId("terminal-link").boundingBox())?.width ?? 0;
  expect(Math.round(width / cell)).toBe("src/App.tsx:3:5".length);
  await page.mouse.click(point.x, point.y);
  await page.keyboard.up(isMac ? "Meta" : "Control");
  await expect(activeTab(page)).toHaveAttribute("data-path", "src/App.tsx");
  await expect(page.getByTestId("status-position")).toHaveText("3:5");
});

test("selection copies, paste sends text, resize follows the window", async ({
  page,
  project,
  context,
}) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  await openApp(page, project);
  await requireTerminals();
  await openTerminal(page);
  await run(page, "echo copy-me-please");
  await expectOutput(page, "copy-me-please");

  // Drag over the word, then copy.
  const word = "copy-me-please";
  const from = await cellOf(page, word, 0);
  const to = await cellOf(page, word, word.length - 1);
  await page.mouse.move(from.x, from.y);
  await page.mouse.down();
  await page.mouse.move(to.x, to.y, { steps: 4 });
  await page.mouse.up();
  await page.keyboard.press(isMac ? "Meta+c" : "Control+Shift+C");
  await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toBe(word);
  if (!fake) {
    // Double click selects the word under the mouse.
    await page.evaluate(() => navigator.clipboard.writeText(""));
    const middle = await cellOf(page, word, 5);
    await page.mouse.dblclick(middle.x, middle.y);
    await page.keyboard.press(isMac ? "Meta+c" : "Control+Shift+C");
    await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toBe(word);
  }

  await page.getByTestId("terminal-input").evaluate((input) => {
    const data = new DataTransfer();
    data.setData("text/plain", "echo pasted-text");
    input.dispatchEvent(new ClipboardEvent("paste", { clipboardData: data, bubbles: true }));
  });
  await page.keyboard.press("Enter");
  await expectOutput(page, "pasted-text");

  const cols = Number(await view(page).getAttribute("data-cols"));
  const size = page.viewportSize() ?? { width: 1280, height: 720 };
  await page.setViewportSize({ width: size.width - 200, height: size.height });
  await expect
    .poll(async () => Number(await view(page).getAttribute("data-cols")))
    .toBeLessThan(cols);
});

test("a flood of output stays responsive and ends on the last line", async ({ page, project }) => {
  await openApp(page, project);
  await requireTerminals();
  test.skip(fake, "needs a real shell and node");
  await openTerminal(page);
  await run(page, `node -e "for (let i = 0; i < 20000; i++) console.log('line ' + i)"`);
  await expect.poll(() => screenText(page), { timeout: 60_000 }).toMatch(/^line 19999\s*$/m);
  // Still interactive afterwards.
  await run(page, "echo after-flood");
  await expectOutput(page, "after-flood");
});
