import { defineConfig } from "vite";
import preact from "@preact/preset-vite";

// Tauri expects a fixed port; HMR over the devUrl in src-tauri/tauri.conf.json.
export default defineConfig({
  plugins: [preact()],
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
  },
  build: {
    target: "es2022",
    outDir: "dist",
  },
  test: {
    environment: "node",
    include: ["src/**/*.test.ts"],
  },
});
