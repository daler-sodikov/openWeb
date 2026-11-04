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
