// Readability in both themes: WCAG AA contrast (4.5:1) of text against the
// background it is painted on, for the editor selection, the completion
// list (rows, selected row, documentation panel), parameter info, hover,
// and every popup, panel, bar and input, the Git surfaces (tree colors,
// commit window, diff, branches, blame). Also checks that the completion
// popup is not clipped and stays inside the window, and saves screenshots
// of the selection and the completion popup.
import fs from "node:fs";
import path from "node:path";

import type { Page, TestInfo } from "@playwright/test";

import {
  activeTab,
  clickWord,
  editor,
  expect,
  initGitRepo,
  isMac,
  keys,
  openApp,
  openFile,
  projectLoaded,
  recordServerStatus,
  test,
  treeRow,
  tsServerMissing,
} from "./harness";

const AA = 4.5;

interface Finding {
  text: string;
  ratio: number;
  fg: string;
  bg: string;
}

interface Scan {
  checked: number;
  failures: Finding[];
}

/**
 * Browser side: contrast of every visible text element (and input value)
 * under `selector` against its effective background (backgrounds and
 * opacity composited up the tree). Self-contained: runs in the page.
 */
function scanContrast(args: { selector: string; min: number }): Scan {
  type Rgba = [number, number, number, number];
  const parse = (value: string): Rgba => {
    const srgb = /color\(srgb ([\d.e-]+) ([\d.e-]+) ([\d.e-]+)(?: \/ ([\d.e-]+))?\)/.exec(value);
    if (srgb) {
      return [
        Number(srgb[1]) * 255,
        Number(srgb[2]) * 255,
        Number(srgb[3]) * 255,
        srgb[4] === undefined ? 1 : Number(srgb[4]),
      ];
    }
    const rgb = /rgba?\(([^)]+)\)/.exec(value);
    if (!rgb?.[1]) return [0, 0, 0, 0];
    const p = rgb[1]
      .split(/[\s,/]+/)
      .filter(Boolean)
      .map(Number);
    return [p[0] ?? 0, p[1] ?? 0, p[2] ?? 0, p[3] ?? 1];
  };
  const over = (top: Rgba, bottom: Rgba): Rgba => {
    const a = top[3];
    return [
      top[0] * a + bottom[0] * (1 - a),
      top[1] * a + bottom[1] * (1 - a),
      top[2] * a + bottom[2] * (1 - a),
      1,
    ];
  };
  const background = (el: Element): Rgba => {
    const layers: Rgba[] = [];
    for (let e: Element | null = el; e; e = e.parentElement) {
      const c = parse(getComputedStyle(e).backgroundColor);
      if (c[3] > 0) layers.push(c);
      if (c[3] >= 1) break;
    }
    let result: Rgba = [255, 255, 255, 1];
    for (const layer of layers.reverse()) result = over(layer, result);
    return result;
  };
  const opacity = (el: Element): number => {
    let o = 1;
    for (let e: Element | null = el; e; e = e.parentElement) {
      o *= Number(getComputedStyle(e).opacity);
    }
    return o;
  };
  const lum = (c: Rgba): number => {
    const ch = c
      .slice(0, 3)
      .map((v) => v / 255)
      .map((v) => (v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4));
    return 0.2126 * (ch[0] ?? 0) + 0.7152 * (ch[1] ?? 0) + 0.0722 * (ch[2] ?? 0);
  };
  const css = (c: Rgba) => `rgb(${c.slice(0, 3).map(Math.round).join(", ")})`;
  const failures: Finding[] = [];
  let checked = 0;
  for (const root of document.querySelectorAll(args.selector)) {
    for (const el of [root, ...root.querySelectorAll("*")]) {
      const style = getComputedStyle(el);
      if (style.visibility === "hidden") continue;
      const rect = el.getBoundingClientRect();
      if (rect.width === 0 || rect.height === 0) continue;
      const input = el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement;
      const text = input
        ? el.value
        : [...el.childNodes]
            .filter((n) => n.nodeType === Node.TEXT_NODE)
            .map((n) => n.textContent ?? "")
            .join("")
            .trim();
      if (!text) continue;
      const bg = background(el);
      const fg = over([...parse(style.color).slice(0, 3), opacity(el)] as Rgba, bg);
      const [hi, lo] = [lum(fg), lum(bg)].sort((a, b) => b - a);
      const ratio = ((hi ?? 0) + 0.05) / ((lo ?? 0) + 0.05);
      checked++;
      if (ratio < args.min) {
        failures.push({
          text: text.slice(0, 40),
          ratio: Math.round(ratio * 100) / 100,
          fg: css(fg),
          bg: css(bg),
        });
      }
    }
  }
  return { checked, failures };
}

/**
 * Browser side: contrast of the text on the selected line against the
 * selection as painted (the line's own background, e.g. the active line,
 * is composited over the selection layer, which sits behind the text).
 * Returns the plain-text ratio and the lowest syntax-token ratio.
 */
function selectionContrast(): { plain: number; lowestToken: number; token: string } | null {
  type Rgba = [number, number, number, number];
  const parse = (value: string): Rgba => {
    const rgb = /rgba?\(([^)]+)\)/.exec(value);
    if (!rgb?.[1]) return [0, 0, 0, 0];
    const p = rgb[1]
      .split(/[\s,/]+/)
      .filter(Boolean)
      .map(Number);
    return [p[0] ?? 0, p[1] ?? 0, p[2] ?? 0, p[3] ?? 1];
  };
  const over = (top: Rgba, bottom: Rgba): Rgba => [
    top[0] * top[3] + bottom[0] * (1 - top[3]),
    top[1] * top[3] + bottom[1] * (1 - top[3]),
    top[2] * top[3] + bottom[2] * (1 - top[3]),
    1,
  ];
  const lum = (c: Rgba): number => {
    const ch = c
      .slice(0, 3)
      .map((v) => v / 255)
      .map((v) => (v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4));
    return 0.2126 * (ch[0] ?? 0) + 0.7152 * (ch[1] ?? 0) + 0.0722 * (ch[2] ?? 0);
  };
  const ratio = (a: Rgba, b: Rgba) => {
    const [hi, lo] = [lum(a), lum(b)].sort((x, y) => y - x);
    return ((hi ?? 0) + 0.05) / ((lo ?? 0) + 0.05);
  };
  const host = document.querySelector(".editor-host:not([hidden])");
  const layer = host?.querySelector(".cm-selectionBackground");
  const line = host?.querySelector(".cm-activeLine");
  const scroller = host?.querySelector(".cm-editor");
  if (!layer || !line || !scroller) return null;
  const editorBg = parse(getComputedStyle(scroller).backgroundColor);
  let bg = over(
    parse(getComputedStyle(layer).backgroundColor),
    editorBg[3] > 0 ? editorBg : [0, 0, 0, 1],
  );
  bg = over(parse(getComputedStyle(line).backgroundColor), bg);
  const plain = ratio(parse(getComputedStyle(line).color), bg);
  let lowestToken = Infinity;
  let token = "";
  for (const span of line.querySelectorAll("span")) {
    const r = ratio(parse(getComputedStyle(span).color), bg);
    if (r < lowestToken) {
      lowestToken = r;
      token = `${span.className} "${span.textContent}"`;
    }
  }
  return {
    plain: Math.round(plain * 100) / 100,
    lowestToken: Math.round(lowestToken * 100) / 100,
    token,
  };
}

/** Fails with the list of unreadable texts under `selector`. */
async function expectReadable(page: Page, selector: string, what: string): Promise<void> {
  const scan = await page.evaluate(scanContrast, { selector, min: AA });
  expect(scan.checked, `${what}: no text found under ${selector}`).toBeGreaterThan(0);
  expect(scan.failures, `${what}: text below ${String(AA)}:1`).toEqual([]);
}

/** Saves a screenshot of `selector` (plus margin) into the test output. */
async function shot(
  page: Page,
  info: TestInfo,
  selectors: string[],
  name: string,
): Promise<string> {
  const boxes = await Promise.all(selectors.map((s) => page.locator(s).first().boundingBox()));
  const present = boxes.filter((b) => b !== null);
  const x = Math.max(0, Math.min(...present.map((b) => b.x)) - 12);
  const y = Math.max(0, Math.min(...present.map((b) => b.y)) - 12);
  const right = Math.max(...present.map((b) => b.x + b.width)) + 12;
  const bottom = Math.max(...present.map((b) => b.y + b.height)) + 12;
  const file = info.outputPath(`${name}.png`);
  await page.screenshot({ path: file, clip: { x, y, width: right - x, height: bottom - y } });
  return file;
}

for (const scheme of ["dark", "light"] as const) {
  test.describe(`${scheme} theme`, () => {
    test.use({ colorScheme: scheme });

    test("Git colors, commit window, diff, branches and blame are readable", async ({
      page,
      project,
    }) => {
      initGitRepo(project);
      fs.appendFileSync(path.join(project, "src", "App.tsx"), "// changed\n");
      fs.writeFileSync(path.join(project, "src", "new-file.ts"), "export {};\n");
      fs.rmSync(path.join(project, "src", "main.tsx"));
      fs.writeFileSync(path.join(project, "debug.log"), "");
      fs.appendFileSync(path.join(project, ".gitignore"), "*.log\n");
      await openApp(page, project);
      await expect(page.getByTestId("status-branch")).toBeVisible({ timeout: 15_000 });
      await treeRow(page, "src").click();
      await expect(treeRow(page, "src/new-file.ts")).toHaveAttribute("data-git", "untracked");
      await expectReadable(page, '[data-testid="project-tree"]', "tree with Git colors");
      // A selected changed file.
      await openFile(page, "src/App.tsx");
      await expectReadable(page, '[data-testid="project-tree"]', "selected changed file");
      await expectReadable(page, '[data-testid="status-bar"]', "status bar with branch");

      await page.keyboard.press(isMac ? "Meta+0" : "Alt+0");
      await expect(page.getByTestId("commit-file").first()).toBeVisible();
      await page.getByTestId("commit-files").focus();
      await page.keyboard.press("ArrowDown");
      await expectReadable(page, '[data-testid="commit"]', "commit window");
      await page.getByTestId("commit-button").click();
      await expect(page.getByTestId("commit-error")).toBeVisible();
      await expectReadable(page, '[data-testid="commit-error"]', "commit error");

      await page.locator('[data-testid="commit-file"][data-path="src/App.tsx"]').dblclick();
      await expect(page.getByTestId("diff-row").first()).toBeVisible();
      await expectReadable(page, '[data-testid="diff-view"]', "side-by-side diff");
      await page.getByTestId("diff-mode").click();
      await expect(page.locator('[data-testid="diff-row"][data-kind="added"]')).toBeVisible();
      await expectReadable(page, '[data-testid="diff-view"]', "unified diff");
      await page.keyboard.press("Escape");

      await page.getByTestId("status-branch").click();
      await expect(page.getByTestId("branch-item").nth(1)).toBeVisible();
      await expectReadable(page, '[data-testid="branches"]', "branches popup");
      await page.keyboard.press("Escape");

      await page.keyboard.press(keys.gotoAction);
      await page.getByTestId("se-input").fill("Annotate with Git Blame");
      await expect(page.locator('[data-type="action"]').first()).toContainText("Annotate");
      await page.keyboard.press("Enter");
      await expect(page.getByTestId("git-blame").first()).toBeVisible();
      await expectReadable(page, ".editor-host:not([hidden]) .cm-blame-gutter", "blame");
    });

    test("selection, completion, documentation and parameter info are readable", async ({
      page,
      project,
    }, info) => {
      const missing = tsServerMissing();
      if (missing !== null && process.env.CI) throw new Error(missing);
      test.skip(missing !== null, missing ?? "");
      await openApp(page, project);
      await recordServerStatus(page);
      await openFile(page, "src/App.tsx");
      await projectLoaded(page);

      // Selection, focused and unfocused.
      await clickWord(page, "const total", "total");
      await page.keyboard.press("Home");
      await page.keyboard.press("Shift+End");
      // The selection layer is drawn in the next animation frame.
      const layer = page.locator(".editor-host:not([hidden]) .cm-selectionBackground");
      await expect(layer.first()).toBeAttached();
      const focused = await page.evaluate(selectionContrast);
      expect(focused, "selection drawn").not.toBeNull();
      expect(focused?.plain, "text on the focused selection").toBeGreaterThanOrEqual(AA);
      info.annotations.push({
        type: `${scheme} selection`,
        description: `plain text ${String(focused?.plain)}:1, lowest token ${String(focused?.lowestToken)}:1 (${focused?.token ?? ""})`,
      });
      const selectionShot = await shot(
        page,
        info,
        [".editor-host:not([hidden]) .cm-activeLine"],
        `${scheme}-selection`,
      );
      // Unfocused: the selection stays, drawn in the inactive color.
      await page.evaluate(() => {
        (document.activeElement as HTMLElement | null)?.blur();
      });
      await expect(page.locator(".editor-host:not([hidden]) .cm-focused")).toHaveCount(0);
      await expect(layer.first()).toBeAttached();
      const unfocused = await page.evaluate(selectionContrast);
      expect(unfocused?.plain, "text on the unfocused selection").toBeGreaterThanOrEqual(AA);
      info.annotations.push({
        type: `${scheme} unfocused selection`,
        description: `plain text ${String(unfocused?.plain)}:1, lowest token ${String(unfocused?.lowestToken)}:1; screenshot ${selectionShot}`,
      });

      // Completion list and documentation panel.
      await openFile(page, "src/App.tsx");
      await clickWord(page, "const total", "total");
      await page.keyboard.press("End");
      await page.keyboard.press("Enter");
      await page.keyboard.type("items.");
      const list = page.locator(".cm-tooltip-autocomplete");
      await expect(list.locator("li").nth(3)).toBeVisible({ timeout: 15_000 });
      // The first row is fully visible and the list is a whole number of rows.
      const geometry = await page.evaluate(() => {
        const ul = document.querySelector(".cm-tooltip-autocomplete > ul");
        const first = ul?.querySelector("li");
        if (!ul || !first) return null;
        const u = ul.getBoundingClientRect();
        const f = first.getBoundingClientRect();
        return {
          listTop: u.top,
          firstTop: f.top,
          rowHeight: f.height,
          height: ul.clientHeight,
          scrollTop: ul.scrollTop,
        };
      });
      expect(geometry).not.toBeNull();
      if (geometry) {
        expect(geometry.scrollTop).toBe(0);
        expect(geometry.firstTop).toBeGreaterThanOrEqual(geometry.listTop - 0.5);
        expect(geometry.height % geometry.rowHeight, "whole rows only").toBeCloseTo(0, 0);
      }
      await expectReadable(page, ".cm-tooltip-autocomplete", "completion rows");
      await expectReadable(
        page,
        ".cm-tooltip-autocomplete li[aria-selected]",
        "selected completion row",
      );
      // Move to an item with documentation (Array methods have it).
      // CodeMirror ignores arrow keys for 75 ms after the list (re)appears.
      await page.waitForTimeout(200);
      await page.keyboard.press("ArrowDown");
      const docs = page.locator(".cm-tooltip.cm-completionInfo");
      await expect(docs).toBeVisible({ timeout: 15_000 });
      await expectReadable(page, ".cm-tooltip.cm-completionInfo", "documentation panel");
      // Popup and panel stay inside the window.
      // CodeMirror positions the panel in the frame after adding it; check
      // the settled position.
      await expect(docs).toHaveAttribute("style", /max-height/);
      // Popup and panel stay inside the window (rectangles reported on failure).
      const outside = await page.evaluate(() =>
        [".cm-tooltip-autocomplete", ".cm-tooltip.cm-completionInfo"].flatMap((selector) => {
          const r = document.querySelector(selector)?.getBoundingClientRect();
          if (!r) return [`${selector}: missing`];
          const fits =
            r.left >= 0 && r.top >= 0 && r.right <= innerWidth && r.bottom <= innerHeight;
          return fits
            ? []
            : [
                `${selector}: left ${String(r.left)} top ${String(r.top)} right ${String(r.right)} bottom ${String(r.bottom)} in ${String(innerWidth)}x${String(innerHeight)}`,
              ];
        }),
      );
      expect(outside, "completion popup and documentation inside the window").toEqual([]);
      const completionShot = await shot(
        page,
        info,
        [".cm-tooltip-autocomplete", ".cm-tooltip.cm-completionInfo"],
        `${scheme}-completion`,
      );
      info.annotations.push({
        type: `${scheme} completion screenshot`,
        description: completionShot,
      });
      await page.keyboard.press("Escape");

      // Parameter info.
      await page.keyboard.type("length;");
      await page.keyboard.press("Enter");
      await page.keyboard.type("formatPrice(");
      await expect(page.getByTestId("signature-help")).toBeVisible({ timeout: 15_000 });
      await expectReadable(page, '[data-testid="signature-help"]', "parameter info");
      await page.keyboard.press("Escape");

      // Hover documentation.
      await clickWord(page, "{formatPrice(total)}", "formatPrice");
      await page.keyboard.press(keys.quickDoc);
      await expect(page.getByTestId("quick-doc")).toBeVisible({ timeout: 15_000 });
      await expectReadable(page, ".cm-tooltip", "quick documentation");
      await page.keyboard.press("Escape");
    });

    test("rename, usages, Alt+Enter, chooser, diagnostics and Problems are readable", async ({
      page,
      project,
    }) => {
      const missing = tsServerMissing();
      if (missing !== null && process.env.CI) throw new Error(missing);
      test.skip(missing !== null, missing ?? "");
      fs.writeFileSync(
        path.join(project, "src", "shapes.ts"),
        [
          "export interface Shape { area(): number; }",
          "export class Square implements Shape { area() { return 1; } }",
          "export class Circle implements Shape { area() { return 3; } }",
          'export const word = pluralize("item", 2);',
          "",
        ].join("\n"),
      );
      await openApp(page, project);
      await recordServerStatus(page);
      await openFile(page, "src/shapes.ts");
      await projectLoaded(page);

      // An error squiggle and its hover message; the Problems list with items.
      const squiggle = page.locator(".cm-diag-error", { hasText: "pluralize" });
      await expect(squiggle).toBeVisible({ timeout: 30_000 });
      await squiggle.hover();
      await expect(page.getByTestId("insight")).toBeVisible();
      await expectReadable(page, ".cm-tooltip", "diagnostic tooltip");
      await page.mouse.move(0, 0);
      await page.keyboard.press(keys.problems);
      await expect(page.getByTestId("problem-item").first()).toBeVisible();
      await page.getByTestId("problem-item").first().hover();
      await expectReadable(page, '[data-testid="problems"]', "Problems with items");
      await page.keyboard.press(keys.problems);

      // Alt+Enter list.
      await clickWord(page, "export const word", "pluralize");
      await page.keyboard.press(keys.intentions);
      await expect(page.getByTestId("code-action").first()).toBeVisible({ timeout: 15_000 });
      await expectReadable(page, '[data-testid="code-actions"]', "Alt+Enter");
      await page.keyboard.press("Escape");

      // Go to Implementation chooser.
      await clickWord(page, "export interface Shape", "Shape");
      await page.keyboard.press(keys.gotoImplementation);
      await expect(page.getByTestId("nav-target")).toHaveCount(2, { timeout: 15_000 });
      await expectReadable(page, '[data-testid="nav-chooser"]', "implementation chooser");
      await page.keyboard.press("Escape");

      // Find Usages.
      await clickWord(page, "export interface Shape", "Shape");
      await page.keyboard.press("Alt+F7");
      await expect(page.getByTestId("usage-result").first()).toBeVisible({ timeout: 15_000 });
      await expectReadable(page, '[data-testid="find-usages"]', "Find Usages");
      await page.keyboard.press("Escape");

      // Rename: the name field and the preview.
      await clickWord(page, "export interface Shape", "Shape");
      await page.keyboard.press("Shift+F6");
      const field = page.getByRole("textbox", { name: "New name" });
      await expect(field).toBeVisible({ timeout: 15_000 });
      await field.fill("Figure");
      await page.keyboard.press("Enter");
      await expect(page.getByTestId("rename-files")).toContainText("shapes.ts");
      await expectReadable(page, '[data-testid="rename-dialog"]', "Rename");
      await page.keyboard.press("Escape");
    });

    test("popups, panels, bars and inputs are readable", async ({ page, project }) => {
      await openApp(page, project);
      await openFile(page, "src/App.tsx");
      await expectReadable(page, '[data-testid="project-tree"]', "project tree");
      await expectReadable(page, '[data-testid="tabs"]', "tabs");
      await expectReadable(page, '[data-testid="status-bar"]', "status bar");
      // Breadcrumbs appear once the caret is inside a scope.
      await clickWord(page, "const total", "total");
      await expect(page.getByTestId("crumb").first()).toBeVisible();
      await expectReadable(page, '[data-testid="breadcrumbs"]', "breadcrumbs");

      // Search Everywhere with results and a typed query.
      await editor(page).click();
      await page.keyboard.press("Shift");
      await page.keyboard.press("Shift");
      await page.keyboard.type("count");
      await expect(page.locator(".popup .vlist-row").first()).toBeVisible();
      await expectReadable(page, ".popup", "Search Everywhere");
      await page.keyboard.press("Escape");

      // Go to Line.
      await editor(page).click();
      await page.keyboard.press(keys.gotoLine);
      await expectReadable(page, ".popup", "Go to Line");
      await page.keyboard.press("Escape");

      // Find in Files with results, and Replace with a preview.
      await page.keyboard.press(keys.replaceInFiles);
      await page.keyboard.type("items");
      await expect(page.locator(".popup .vlist-row").first()).toBeVisible();
      await expectReadable(page, ".popup", "Find/Replace in Files");
      await page.keyboard.press("Escape");

      // Recent Files.
      await page.keyboard.press(keys.recentFiles);
      await expect(page.locator(".popup")).toBeVisible();
      await expectReadable(page, ".popup", "Recent Files");
      await page.keyboard.press("Escape");

      // The reload bar (file changed on disk while modified).
      await editor(page).click();
      await page.keyboard.press("ControlOrMeta+End");
      await page.keyboard.type("\n// local edit");
      fs.appendFileSync(path.join(project, "src", "App.tsx"), "// changed on disk\n");
      await expect(page.getByTestId("external-change")).toBeVisible({ timeout: 15_000 });
      await expectReadable(page, '[data-testid="external-change"]', "reload bar");
      await page.getByTestId("keep-changes").click();

      // Problems panel (empty state is text too).
      await page.keyboard.press(keys.problems);
      await expect(page.getByTestId("problems")).toBeVisible();
      await expectReadable(page, '[data-testid="problems"]', "Problems");
      await expect(activeTab(page)).toBeVisible();
    });
  });
}
