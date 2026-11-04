import { defineConfig } from "@playwright/test";

// The e2e tests drive the real desktop binary, not a browser page; see
// docs/ARCHITECTURE.md, section 7. The binary is built in global setup.
export default defineConfig({
  testDir: "e2e",
  globalSetup: "./e2e/global-setup.ts",
  timeout: 120_000,
  fullyParallel: false,
  workers: 1,
  forbidOnly: !!process.env.CI,
  reporter: process.env.CI ? [["list"], ["github"]] : "list",
});
