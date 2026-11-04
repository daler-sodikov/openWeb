import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

/** Repository root. */
export const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

/** Debug build of the desktop binary produced by `bun tauri build --debug --no-bundle`. */
export const appBinary = path.join(
  repoRoot,
  "target",
  "debug",
  process.platform === "win32" ? "ferrule.exe" : "ferrule",
);

/** Vite dev server used by the `ui` project. */
export const FRONTEND_PORT = 1430;
export const FRONTEND_URL = `http://127.0.0.1:${String(FRONTEND_PORT)}`;

/** Headless Rust backend used by the `ui` project. */
export const BACKEND_PORT = 1431;
export const BACKEND_URL = `http://127.0.0.1:${String(BACKEND_PORT)}`;

/** Data directory of the headless backend (its `settings.json` lives here). */
export const BACKEND_DATA = path.join(os.tmpdir(), "ferrule-e2e-backend");
