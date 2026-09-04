import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import path from "node:path";

// https://tauri.app/v1/guides/getting-started/setup/vite/
export default defineConfig(async () => ({
  plugins: [react()],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },

  // Vite dev server must run on a fixed port that matches src-tauri/tauri.conf.json's devUrl.
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: "0.0.0.0",
    watch: {
      // Don't watch the Rust backend, it isn't compiled during frontend dev.
      ignored: ["**/src-tauri/**"],
    },
  },

  build: {
    outDir: "dist",
    // Tauri uses Chromium on Windows/Linux and WebKit on macOS.
    target: process.env.TAURI_ENV_PLATFORM === "windows" ? "chrome105" : "safari13",
    minify: !process.env.TAURI_ENV_DEBUG ? "esbuild" : false,
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
  },
}));
