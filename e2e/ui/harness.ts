// Shared setup for UI tests: a fresh copy of fixtures/react-ts per test and a
// `window.__TAURI_INTERNALS__` shim forwarding IPC to the headless backend.
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

import { expect, type Locator, type Page, test as base } from "@playwright/test";

import { BACKEND_URL, repoRoot } from "../paths";

export const isMac = process.platform === "darwin";

/** Runs in the page before any app script. */
function installTauriShim(backend: string): void {
  type Handler = (event: { event: string; id: number; payload: unknown }) => void;
  const callbacks = new Map<number, Handler>();
  const listeners = new Map<number, { event: string; handler: number }>();
  let nextId = 1;
  let polling = false;

  // Delivers backend events (long poll on the headless backend).
  const poll = async (): Promise<void> => {
    let after = "latest";
    for (;;) {
      try {
        const response = await fetch(`${backend}/events?after=${after}`);
        const body = (await response.json()) as {
          next: number;
          events: { event: string; payload: unknown }[];
        };
        after = String(body.next);
        for (const e of body.events) {
          for (const [id, l] of listeners) {
            if (l.event === e.event)
              callbacks.get(l.handler)?.({ event: e.event, id, payload: e.payload });
          }
        }
      } catch {
        await new Promise((resolve) => setTimeout(resolve, 500));
      }
    }
  };

  const internals = {
    invoke: async (cmd: string, args?: unknown): Promise<unknown> => {
      if (cmd === "plugin:event|listen") {
        const { event, handler } = args as { event: string; handler: number };
        const id = nextId++;
        listeners.set(id, { event, handler });
        if (!polling) {
          polling = true;
          void poll();
        }
        return id;
      }
      if (cmd === "plugin:event|unlisten") {
        listeners.delete((args as { eventId: number }).eventId);
        return null;
      }
      const response = await fetch(`${backend}/invoke/${cmd}`, {
        method: "POST",
        body: JSON.stringify(args ?? {}),
      });
      const body = (await response.json()) as { ok?: unknown; err?: unknown };
      if ("err" in body) {
        // Tauri rejects with the plain error value (not an Error instance);
        // the generated bindings rely on that to return typed errors.
        throw body.err;
      }
      return body.ok;
    },
    transformCallback: (callback: Handler): number => {
      const id = nextId++;
      callbacks.set(id, callback);
      return id;
    },
    metadata: {
      currentWindow: { label: "main" },
      currentWebview: { windowLabel: "main", label: "main" },
    },
  };
  Object.defineProperty(window, "__TAURI_INTERNALS__", { value: internals });
}

/** Calls a backend command directly from the test process. */
export async function backend(cmd: string, args: unknown): Promise<unknown> {
  const response = await fetch(`${BACKEND_URL}/invoke/${cmd}`, {
    method: "POST",
    body: JSON.stringify(args),
  });
  const body = (await response.json()) as { ok?: unknown; err?: unknown };
  if ("err" in body) throw new Error(JSON.stringify(body.err));
  return body.ok;
}

/** Runs `git` in `cwd` (local commands only) and returns its output. */
export function git(cwd: string, ...args: string[]): string {
  return execFileSync("git", args, { cwd, encoding: "utf8" });
}

/**
 * Makes `project` a repository with one commit, independent of the user's
 * global config where it matters (identity, signing, hooks, line endings).
 */
export function initGitRepo(project: string): void {
  git(project, "init", "-q", "-b", "main");
  git(project, "config", "user.name", "Test User");
  git(project, "config", "user.email", "test@example.com");
  git(project, "config", "commit.gpgsign", "false");
  git(project, "config", "core.hooksPath", ".git/hooks");
  git(project, "config", "core.autocrlf", "false");
  git(project, "add", ".");
  git(project, "commit", "-q", "-m", "Initial commit");
}

/** Waits until the project index is built. */
export async function waitForIndex(): Promise<void> {
  await expect
    .poll(
      async () => {
        const r = (await backend("search_everywhere", { query: "", tab: "files" })) as {
          indexing: boolean;
        };
        return r.indexing;
      },
      { timeout: 20_000 },
    )
    .toBe(false);
}

export const test = base.extend<{ project: string }>({
  // eslint-disable-next-line no-empty-pattern -- Playwright requires object destructuring here
  project: async ({}, use) => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "ferrule-ui-"));
    const root = path.join(dir, "react-ts");
    fs.cpSync(path.join(repoRoot, "fixtures", "react-ts"), root, { recursive: true });
    await use(root);
    // The backend keeps the project open (file watcher, language server
    // with the project as its working directory). Windows cannot delete a
    // folder a process still holds, so switch the backend to an empty idle
    // project first (which stops those) and retry while they exit.
    const idle = path.join(os.tmpdir(), "ferrule-ui-idle");
    fs.mkdirSync(idle, { recursive: true });
    await backend("open_project", { path: idle }).catch(() => undefined);
    fs.rmSync(dir, { recursive: true, force: true, maxRetries: 40, retryDelay: 250 });
  },
  page: async ({ page }, use) => {
    await page.addInitScript(installTauriShim, BACKEND_URL);
    await use(page);
  },
});

export { expect };

/** Loads the app with `project` opened. */
export async function openApp(page: Page, project: string): Promise<void> {
  await page.goto(`/?project=${encodeURIComponent(project)}`);
  await expect(page.getByTestId("project-name")).toHaveText(path.basename(project));
  await waitForIndex();
}

export function treeRow(page: Page, relPath: string): Locator {
  return page.locator(`[data-testid="tree-row"][data-path="${relPath}"]`);
}

/** Opens a file through the tree, expanding its parent folders. */
export async function openFile(page: Page, relPath: string): Promise<void> {
  const parts = relPath.split("/");
  for (let i = 1; i < parts.length; i++) {
    const dir = treeRow(page, parts.slice(0, i).join("/"));
    if ((await dir.getAttribute("aria-expanded")) !== "true") await dir.click();
  }
  await treeRow(page, relPath).click();
  await expect(activeTab(page)).toHaveAttribute("data-path", relPath);
}

export function activeTab(page: Page): Locator {
  return page.locator('[data-testid="tab"][aria-selected="true"]');
}

/** The content element of the visible editor. */
export function editor(page: Page): Locator {
  return page.locator(".editor-host:not([hidden]) .cm-content");
}

/** Text of the visible editor (all lines are rendered for small files). */
export async function editorText(page: Page): Promise<string> {
  const lines = await page.locator(".editor-host:not([hidden]) .cm-line").allTextContents();
  return lines.join("\n");
}

/** Focuses the editor and moves the caret to the start of the document. */
export async function caretToStart(page: Page): Promise<void> {
  await editor(page).click();
  await page.keyboard.press("ControlOrMeta+Home");
}

/** IntelliJ-style keys for the current OS (see crates/settings). */
export const keys = {
  selectNext: isMac ? "Control+g" : "Alt+j",
  selectAll: isMac ? "Meta+Control+g" : "Control+Alt+Shift+j",
  caretModifier: isMac ? "Alt" : "Control",
  gotoFile: isMac ? "Meta+Shift+O" : "Control+Shift+N",
  gotoSymbol: isMac ? "Meta+Alt+O" : "Control+Alt+Shift+N",
  gotoAction: "ControlOrMeta+Shift+A",
  gotoLine: isMac ? "Meta+l" : "Control+g",
  recentFiles: "ControlOrMeta+e",
  findInFiles: "ControlOrMeta+Shift+F",
  replaceInFiles: "ControlOrMeta+Shift+R",
  matchingBrace: isMac ? "Meta+Control+m" : "Control+Shift+M",
  quickDoc: isMac ? "F1" : "Control+q",
  gotoDeclaration: "ControlOrMeta+b",
  gotoImplementation: isMac ? "Meta+Alt+b" : "Control+Alt+b",
  problems: isMac ? "Meta+6" : "Alt+6",
  parameterInfo: "ControlOrMeta+p",
  intentions: "Alt+Enter",
  reformat: isMac ? "Meta+Alt+l" : "Control+Alt+l",
  undo: "ControlOrMeta+z",
};

/**
 * Why the real TypeScript server tests cannot run here, or null when
 * `typescript-language-server` and `node` are on PATH (the headless backend
 * inherits this process's environment).
 */
export function tsServerMissing(): string | null {
  const dirs = (process.env.PATH ?? "").split(path.delimiter).filter(Boolean);
  const exts = process.platform === "win32" ? [".cmd", ".exe", ""] : [""];
  const found = (name: string) =>
    dirs.some((dir) => exts.some((ext) => fs.existsSync(path.join(dir, name + ext))));
  const missing = ["typescript-language-server", "node"].filter((name) => !found(name));
  return missing.length === 0
    ? null
    : `${missing.join(" and ")} not on PATH; install with \`bun add -g typescript-language-server typescript\``;
}

/**
 * Records every text the server status shows (call before opening a file),
 * so a short "indexing" phase is not missed between polls.
 */
export async function recordServerStatus(page: Page): Promise<void> {
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
export async function projectLoaded(page: Page): Promise<void> {
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
export async function clickWord(
  page: Page,
  line: string,
  word: string,
  modifier?: string,
): Promise<void> {
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
