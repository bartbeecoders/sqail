import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { readFileSync } from "fs";
import { resolve } from "path";

const host = process.env.TAURI_DEV_HOST;

function getBuildInfo() {
  try {
    const raw = readFileSync(resolve(__dirname, "build-number.json"), "utf-8");
    return JSON.parse(raw);
  } catch {
    return { date: "dev", revision: 0, buildNumber: "dev" };
  }
}

const buildInfo = getBuildInfo();
const pkg = JSON.parse(readFileSync(resolve(__dirname, "package.json"), "utf-8"));

export default defineConfig({
  plugins: [react(), tailwindcss()],
  define: {
    __APP_VERSION__: JSON.stringify(pkg.version),
    __BUILD_NUMBER__: JSON.stringify(buildInfo.buildNumber),
  },
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || "127.0.0.1",
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
});
