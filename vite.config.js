import { defineConfig } from "vite";
import tailwindcss from "@tailwindcss/vite";

export default defineConfig({
  root: "src/frontend",
  plugins: [tailwindcss()],
  build: {
    outDir: "../../dist",
    emptyOutDir: true,
  },
  optimizeDeps: {
    exclude: ["@sqlite.org/sqlite-wasm"],
  },
  server: {
    headers: {
      "Cross-Origin-Opener-Policy": "same-origin",
      "Cross-Origin-Embedder-Policy": "credentialless",
    },
    proxy: {
      "/api": "http://localhost:2592",
    },
  },
});
