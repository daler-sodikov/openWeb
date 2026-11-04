// Shared setup for UI tests: a fresh copy of fixtures/react-ts per test and a
// `window.__TAURI_INTERNALS__` shim forwarding IPC to the headless backend.
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
    fs.rmSync(dir, { recursive: true, force: true });
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
};
