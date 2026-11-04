import { defineConfig } from "vite";

// Tauri expects a fixed dev-server port; see src-tauri/tauri.conf.json.
const DEV_PORT = 1420;

export default defineConfig({
  clearScreen: false,
  server: {
    port: DEV_PORT,
    strictPort: true,
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
