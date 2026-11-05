import { defineConfig, devices } from "@playwright/test";

import { BACKEND_DATA, BACKEND_PORT, BACKEND_URL, FRONTEND_PORT, FRONTEND_URL } from "./e2e/paths";

const ci = !!process.env.CI;
// FERRULE_E2E_NO_NATIVE=1 builds the test backend without real terminals
// (for machines that do not have the terminal crates); terminal tests then
// skip themselves. CI always builds the full backend.
const native = process.env.FERRULE_E2E_NO_NATIVE === "1" ? " --no-default-features" : "";
// Optional local override: path to an already installed Chromium/Chrome
// binary, for machines where `bunx playwright install chromium` is not
// possible. CI does not set it.
const chromium = process.env.FERRULE_E2E_CHROMIUM;

// Two projects (docs/ARCHITECTURE.md, section 7):
// - smoke: launches the real desktop binary (real webview, bundled assets);
// - ui: drives Chromium against the Vite dev server, with IPC forwarded to the
//   headless Rust backend (same command handler as the app).
export default defineConfig({
  testDir: "e2e",
  timeout: 120_000,
  expect: { timeout: 10_000 },
  fullyParallel: false,
  workers: 1,
  forbidOnly: ci,
  reporter: ci ? [["list"], ["github"]] : "list",
  projects: [
    { name: "smoke", testMatch: "smoke/*.spec.ts" },
    {
      name: "ui",
      testMatch: "ui/*.spec.ts",
      use: {
        ...devices["Desktop Chrome"],
        baseURL: FRONTEND_URL,
        launchOptions: chromium ? { executablePath: chromium } : {},
      },
    },
  ],
  webServer: [
    {
      command: `bun run gen:ipc && bunx vite --port ${String(FRONTEND_PORT)} --strictPort --host 127.0.0.1`,
      url: FRONTEND_URL,
      // Serve the production CSP (see vite.config.ts).
      env: { FERRULE_E2E_CSP: BACKEND_URL },
      reuseExistingServer: false,
      timeout: 600_000,
    },
    {
      command: `cargo run --quiet -p ferrule-app${native} --features headless --bin ferrule-headless -- --port ${String(BACKEND_PORT)} --data-dir "${BACKEND_DATA}"`,
      url: `${BACKEND_URL}/health`,
      reuseExistingServer: false,
      timeout: 900_000,
    },
  ],
});
