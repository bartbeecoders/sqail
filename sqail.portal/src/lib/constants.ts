export const VERSION = "0.3.0";
export const CODEBERG_URL = "https://codeberg.org/nicokosi/sqail";
export const RELEASES_URL = `${CODEBERG_URL}/releases`;

export type Platform = "windows" | "macos" | "linux";

export interface DownloadInfo {
  platform: Platform;
  label: string;
  fileName: string;
  icon: string;
}

export const DOWNLOADS: DownloadInfo[] = [
  {
    platform: "windows",
    label: "Windows",
    fileName: `sqail_${VERSION}_x64-setup.msi`,
    icon: "Monitor",
  },
  {
    platform: "macos",
    label: "macOS",
    fileName: `sqail_${VERSION}_x64.dmg`,
    icon: "Apple",
  },
  {
    platform: "linux",
    label: "Linux",
    fileName: `sqail_${VERSION}_amd64.AppImage`,
    icon: "Terminal",
  },
];

export function detectPlatform(): Platform {
  const ua = navigator.userAgent.toLowerCase();
  if (ua.includes("win")) return "windows";
  if (ua.includes("mac")) return "macos";
  return "linux";
}

export function getDownloadUrl(fileName: string): string {
  return `${RELEASES_URL}/download/v${VERSION}/${fileName}`;
}

export const FEATURES = [
  {
    icon: "Monitor",
    title: "Cross-Platform",
    description: "Native desktop app for Windows, macOS, and Linux. No browser needed.",
  },
  {
    icon: "Zap",
    title: "Lightweight",
    description: "Built with Tauri and Rust — fast startup, small footprint, no Electron bloat.",
  },
  {
    icon: "Database",
    title: "4 Databases",
    description: "Connect to PostgreSQL, MySQL, SQLite, and SQL Server from one app.",
  },
  {
    icon: "Code",
    title: "Monaco Editor",
    description: "VS Code-grade SQL editing with syntax highlighting and intelligent autocomplete.",
  },
  {
    icon: "History",
    title: "Query History",
    description: "Every query is saved. Search, filter, and re-run your past work instantly.",
  },
  {
    icon: "Moon",
    title: "Dark & Light",
    description: "Beautiful themes that adapt to your system preference or manual selection.",
  },
  {
    icon: "Keyboard",
    title: "Keyboard-First",
    description: "Fully customizable shortcuts for a fast, mouse-free workflow.",
  },
  {
    icon: "GitBranch",
    title: "Open Source",
    description: "Free forever. Hosted on Codeberg. Community-driven development.",
  },
] as const;

export const AI_PROVIDERS = [
  "Claude",
  "OpenAI",
  "Minimax",
  "Z.ai",
  "LM Studio",
  "Claude Code CLI",
  "OpenAI Compatible",
] as const;

export const DATABASES = [
  {
    name: "PostgreSQL",
    description: "Full support including schemas, functions, and advanced types.",
    color: "#336791",
  },
  {
    name: "MySQL",
    description: "Complete MySQL support with dialect-specific syntax highlighting.",
    color: "#4479A1",
  },
  {
    name: "SQLite",
    description: "Local and file-based databases with zero configuration.",
    color: "#003B57",
  },
  {
    name: "SQL Server",
    description: "MSSQL with Entra ID, Windows, and SQL Server authentication.",
    color: "#CC2927",
  },
] as const;

export const NAV_ITEMS = [
  { label: "Features", href: "#features" },
  { label: "AI", href: "#ai" },
  { label: "Databases", href: "#databases" },
  { label: "Download", href: "#download" },
] as const;
