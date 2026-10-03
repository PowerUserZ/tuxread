import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri serves the built files; the dev server is only for `tauri dev`.
export default defineConfig({
  plugins: [react()],
  // The version shown in About; npm sets this when it runs `build` or `dev`.
  define: { __APP_VERSION__: JSON.stringify(process.env.npm_package_version ?? "dev") },
  clearScreen: false,
  server: { port: 5173, strictPort: true },
  build: { target: "es2022", outDir: "dist" },
});
