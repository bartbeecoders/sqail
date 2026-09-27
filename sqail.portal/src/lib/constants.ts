export const GITHUB_URL = "https://github.com/bartbeecoders/sqail";
export const CODEBERG_URL = "https://codeberg.org/bartbeecoders/sqail";
export const RELEASES_URL = `${GITHUB_URL}/releases`;
/** Docs are Markdown files in the repository. */
export const docUrl = (file: string) => `${CODEBERG_URL}/src/branch/main/docs/${file}`;

/** Last release of the 0.x Tauri app (now in sqail-legacy/). */
export const LEGACY_VERSION = "0.6.9";
export const LEGACY_URL = `${GITHUB_URL}/releases/tag/v${LEGACY_VERSION}`;

export type Platform = "windows" | "macos" | "linux";

export function detectPlatform(): Platform {
  const ua = navigator.userAgent.toLowerCase();
  if (ua.includes("win")) return "windows";
  if (ua.includes("mac")) return "macos";
  return "linux";
}

export function getDownloadUrl(fileName: string): string {
  return `/releases/${fileName}`;
}

export const FEATURES = [
  {
    icon: "Zap",
    accent: "cyan",
    title: "Fast",
    headline: "A million rows in under a second.",
    description:
      "Native Rust with a GPU-rendered UI. Results stream in while you scroll, sort and copy, and the grid stays smooth with millions of rows.",
  },
  {
    icon: "ShieldCheck",
    accent: "yellow",
    title: "Secure",
    headline: "No database passwords on the desktop.",
    description:
      "Connections live in sqail-service, an HTTPS gateway. The editor holds only a scoped token in the OS credential store and pins the service's certificate.",
  },
  {
    icon: "Database",
    accent: "cyan",
    title: "Real SQL work",
    headline: "Transactions, plans, sessions.",
    description:
      "Every tab has its own server session. BEGIN … COMMIT spans runs, auto-commit can be switched off, and Explain shows one plan tree for every engine.",
  },
  {
    icon: "Table",
    accent: "cyan",
    title: "Edit & export",
    headline: "Change data safely, take it anywhere.",
    description:
      "Edit single-table results in place and apply them in one transaction. Export to CSV, JSON, Excel or SQL, or stream the whole query straight to a file.",
  },
  {
    icon: "Keyboard",
    accent: "yellow",
    title: "Keyboard-first",
    headline: "Every action is one shortcut away.",
    description:
      "Statement-aware Ctrl+Enter, schema-aware completion, a formatter, a command palette and quick open. Every key binding can be changed.",
  },
  {
    icon: "GitBranch",
    accent: "cyan",
    title: "Free",
    headline: "Open source, no telemetry.",
    description:
      "MIT licensed, hosted on Codeberg with a GitHub mirror. No account, no paid tier, nothing sent home.",
  },
] as const;

export const DATABASES = [
  {
    name: "PostgreSQL",
    description: "SSL modes like libpq, read-only sessions, EXPLAIN (ANALYZE) as a plan tree.",
    color: "#336791",
  },
  {
    name: "SQL Server",
    description: "SQL or Windows authentication, named instances, GO batches, PRINT and RAISERROR messages.",
    color: "#CC2927",
  },
  {
    name: "SQLite",
    description: "Files on the service's machine, restricted to the folders its administrator allows.",
    color: "#003B57",
  },
] as const;

export const NAV_ITEMS = [
  { label: "Features", href: "#features" },
  { label: "Screenshots", href: "#screenshots" },
  { label: "Service", href: "#service" },
  { label: "Databases", href: "#databases" },
  { label: "Download", href: "#download" },
  { label: "Docs", href: "#docs" },
  { label: "Changelog", href: "#changelog" },
] as const;
