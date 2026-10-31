import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri serves the built frontend from the app bundle, so assets must be relative and the dev server must
// listen on a fixed port for `devUrl`. The strict port matters: a second dev server would silently move to
// 1421 and Tauri would keep loading the stale one.
export default defineConfig({
  plugins: [react()],
  base: "./",
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: "127.0.0.1",
  },
  build: {
    target: "chrome110",
    outDir: "dist",
    emptyOutDir: true,
    sourcemap: false,
  },
});
