import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

export default defineConfig({
  plugins: [react(), tailwindcss()],
  build: { outDir: "dist" },
  // The changelog is read from the repository root (../CHANGELOG.md).
  server: { fs: { allow: [".."] } },
});
