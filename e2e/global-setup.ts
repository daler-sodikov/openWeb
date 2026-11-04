import { spawnSync } from "node:child_process";

import { repoRoot } from "./paths";

/**
 * Builds the debug app with the frontend bundled in (no dev server), so the
 * smoke test exercises the same asset path as a release build. Set
 * FERRULE_E2E_SKIP_BUILD=1 when CI has already built it.
 */
export default function globalSetup(): void {
  if (process.env.FERRULE_E2E_SKIP_BUILD === "1") {
    return;
  }
  const result = spawnSync("bun", ["tauri", "build", "--debug", "--no-bundle"], {
    cwd: repoRoot,
    stdio: "inherit",
    shell: process.platform === "win32",
  });
  if (result.status !== 0) {
    throw new Error(`building the app failed with exit code ${String(result.status)}`);
  }
}
