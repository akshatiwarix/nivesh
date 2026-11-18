/// <reference types="vitest/config" />
import { defineConfig } from "vite";
import solid from "vite-plugin-solid";

// Set by the Tauri CLI during `tauri dev` / `tauri build`.
const tauriDebug = Boolean(process.env.TAURI_ENV_DEBUG);
const tauriPlatform = process.env.TAURI_ENV_PLATFORM;

export default defineConfig({
  plugins: [solid()],
  // Keep Rust compiler output visible when run by `tauri dev`.
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: "localhost",
    watch: {
      ignored: ["**/src-tauri/**", "**/crates/**", "**/target/**"],
    },
  },
  envPrefix: ["VITE_", "TAURI_ENV_"],
  build: {
    // WebView2 on Windows is Chromium; macOS (12+) and Linux use WebKit.
    target: tauriPlatform === "windows" ? "chrome105" : "safari15",
    minify: !tauriDebug,
    sourcemap: tauriDebug,
    outDir: "dist",
    emptyOutDir: true,
  },
  test: {
    environment: "happy-dom",
    include: ["ui/**/*.test.{ts,tsx}"],
  },
});
