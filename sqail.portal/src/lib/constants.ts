export const VERSION = "0.5.4";
export const BUILD_NUMBER = "20260411-1";
export const GITHUB_URL = "https://github.com/bartbeecoders/sqail";

export type Platform = "windows" | "macos" | "linux";

export interface DownloadInfo {
  platform: Platform;
  label: string;
  fileName: string;
  icon: string;
  ext: string;
}

const FILE_PREFIX = `sqail_${VERSION}_${BUILD_NUMBER}`;

export const DOWNLOADS: DownloadInfo[] = [
  {
    platform: "windows",
    label: "Windows",
    fileName: `${FILE_PREFIX}_x64-setup.exe`,
    icon: "Monitor",
    ext: ".exe",
  },
  {
    platform: "macos",
    label: "macOS (Universal)",
    fileName: `${FILE_PREFIX}_universal.dmg`,
    icon: "Apple",
    ext: ".dmg",
  },
];

export interface LinuxDownloadInfo {
  label: string;
  fileName: string;
  description: string;
}

export const LINUX_DOWNLOADS: LinuxDownloadInfo[] = [
  {
    label: ".deb",
    fileName: `${FILE_PREFIX}_amd64.deb`,
    description: "Ubuntu, Debian, Pop!_OS, Mint",
  },
  {
    label: ".rpm",
    fileName: `${FILE_PREFIX}_amd64.rpm`,
    description: "Fedora, RHEL, openSUSE",
  },
  {
    label: "Arch (pacman)",
    fileName: `${FILE_PREFIX}_x86_64.pkg.tar.zst`,
    description: "Arch Linux, EndeavourOS, Manjaro",
  },
  {
    label: "AppImage",
    fileName: `${FILE_PREFIX}_amd64.AppImage`,
    description: "Portable — runs on any distro",
  },
];

export function detectPlatform(): Platform {
  const ua = navigator.userAgent.toLowerCase();
  if (ua.includes("win")) return "windows";
  if (ua.includes("mac")) return "macos";
  return "linux";
}

export function getDownloadUrl(fileName: string): string {
  return `/releases/${fileName}`;
}

export interface DbServiceDownloadInfo {
  platform: Platform;
  label: string;
  fileName: string;
  ext: string;
}

export const DBSERVICE_DOWNLOADS: DbServiceDownloadInfo[] = [
  {
    platform: "windows",
    label: "Windows (x64)",
    fileName: `${FILE_PREFIX}_dbservice_win-x64.zip`,
    ext: ".zip",
  },
];

export const FEATURES = [
  {
    icon: "Zap",
    title: "Fast",
    headline: "Opens before your terminal.",
    description: "Under 20 MB, sub-second launch, native Tauri webview. No Electron, no JVM, no Chromium bundle.",
  },
  {
    icon: "Sparkles",
    title: "Smart",
    headline: "AI that actually knows your schema.",
    description: "Schema-aware autocomplete and NL-to-SQL with your tables injected as context. Bring your own key.",
  },
  {
    icon: "GitBranch",
    title: "Free",
    headline: "Open source. Forever.",
    description: "MIT licensed, hosted on Codeberg with a GitHub mirror. No account, no freemium, no feature gates.",
  },
  {
    icon: "ShieldCheck",
    title: "Private",
    headline: "Your queries stay on your machine.",
    description: "No telemetry. Credentials live in a local encrypted store. AI providers are only called when you configure them.",
  },
  {
    icon: "Database",
    title: "Universal",
    headline: "Postgres, MySQL, SQLite, SQL Server — one editor.",
    description: "Multi-driver single UI with SSH tunnels, split editor, query history, and Monaco-powered editing.",
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
  { label: "Screenshots", href: "#screenshots" },
  { label: "AI", href: "#ai" },
  { label: "Databases", href: "#databases" },
  { label: "Compare", href: "#compare" },
  { label: "Download", href: "#download" },
  { label: "Docs", href: "#docs" },
  { label: "Changelog", href: "#changelog" },
] as const;
