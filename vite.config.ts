import { defineConfig } from "vite";

// Tauri loads the built files from dist/ and uses this dev server during `tauri dev`.
export default defineConfig({
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  build: { target: "es2022" },
});
