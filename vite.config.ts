import fs from "node:fs";

import { defineConfig } from "vite";

// Tauri expects a fixed dev-server port; see src-tauri/tauri.conf.json.
const DEV_PORT = 1420;

/**
 * In UI tests (FERRULE_E2E_CSP set to the headless backend URL), serve pages
 * with the production CSP from tauri.conf.json, so CSP regressions (e.g.
 * CodeMirror's injected styles being blocked) fail the tests. The backend
 * URL is added to connect-src because tests bridge IPC over HTTP.
 */
function e2eCspHeaders(): Record<string, string> {
  const backend = process.env.FERRULE_E2E_CSP;
  if (!backend) return {};
  const config = JSON.parse(fs.readFileSync("src-tauri/tauri.conf.json", "utf8")) as {
    app: { security: { csp: string } };
  };
  const csp = config.app.security.csp.replace("connect-src", `connect-src ${backend} ws:`);
  return { "Content-Security-Policy": csp };
}

export default defineConfig({
  clearScreen: false,
  server: {
    port: DEV_PORT,
    strictPort: true,
    headers: e2eCspHeaders(),
    watch: {
      ignored: ["**/src-tauri/**", "**/crates/**", "**/target/**"],
    },
  },
  envPrefix: ["VITE_", "TAURI_ENV_"],
  build: {
    target: "es2022",
    sourcemap: true,
  },
});
