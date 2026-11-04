import { spawn, spawnSync } from "node:child_process";

import { expect, test } from "@playwright/test";

import { appBinary, repoRoot } from "../paths";

interface AppRun {
  code: number | null;
  output: string;
}

/**
 * Builds the debug app with the frontend bundled in (no dev server), so the
 * smoke test exercises the same asset path as a release build. Set
 * FERRULE_E2E_SKIP_BUILD=1 when CI has already built it.
 */
test.beforeAll(() => {
  test.setTimeout(900_000);
  if (process.env.FERRULE_E2E_SKIP_BUILD === "1") return;
  const result = spawnSync("bun", ["tauri", "build", "--debug", "--no-bundle"], {
    cwd: repoRoot,
    stdio: "inherit",
    shell: process.platform === "win32",
  });
  if (result.status !== 0) {
    throw new Error(`building the app failed with exit code ${String(result.status)}`);
  }
});

/** Launches the app in smoke mode and resolves when it exits. */
function runSmoke(timeoutMs: number): Promise<AppRun> {
  return new Promise((resolve, reject) => {
    const child = spawn(appBinary, [], {
      env: { ...process.env, FERRULE_E2E_SMOKE: "1", RUST_LOG: "info" },
      stdio: ["ignore", "pipe", "pipe"],
    });
    let output = "";
    child.stdout.on("data", (chunk: Buffer) => (output += chunk.toString()));
    child.stderr.on("data", (chunk: Buffer) => (output += chunk.toString()));
    const timer = setTimeout(() => {
      child.kill();
      reject(new Error(`app did not exit within ${String(timeoutMs)} ms\n${output}`));
    }, timeoutMs);
    child.on("error", (error) => {
      clearTimeout(timer);
      reject(error);
    });
    child.on("exit", (code) => {
      clearTimeout(timer);
      resolve({ code, output });
    });
  });
}

test("app window opens, loads the frontend and completes an IPC round trip", async () => {
  // In smoke mode the app exits 0 only after the webview rendered the product
  // name fetched over IPC and reported it back (see src-tauri/src/lib.rs).
  const run = await runSmoke(90_000);
  expect(run.output).toContain("frontend ready");
  expect(run.code, run.output).toBe(0);
});
