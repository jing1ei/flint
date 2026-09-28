import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri drives the dev server from tauri.conf.json (`devUrl: http://127.0.0.1:1420`), so the port
// is fixed and must never silently move to 1421 — the webview would load nothing. Host and devUrl
// must also agree literally: `localhost` can resolve to ::1 while Vite listens on 127.0.0.1 only.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: "127.0.0.1",
    watch: {
      // Rust output churns constantly during `tauri dev`; watching it costs CPU and reloads nothing.
      ignored: ["**/src-tauri/**", "**/target/**", "**/crates/**"],
    },
  },
  preview: {
    port: 4173,
    strictPort: true,
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
    // Matches `bundle.macOS.minimumSystemVersion: "11.0"` (Big Sur ships the Safari 14 WebKit
    // generation). Syntax transforms do not polyfill newer browser APIs.
    target: ["safari14", "edge109"],
    sourcemap: false,
  },
});
