export interface ShortcutAction {
  id: string;
  label: string;
  defaultKey: string;
  category: "editor" | "file" | "connections" | "ai" | "app";
}

/** Canonical list of all shortcut actions with their defaults. */
export const SHORTCUT_ACTIONS: ShortcutAction[] = [
  // Editor
  { id: "run-query", label: "Run Query", defaultKey: "F5", category: "editor" },
  { id: "validate-query", label: "Validate Query", defaultKey: "Ctrl+Shift+V", category: "editor" },
  { id: "format-query", label: "Format Query", defaultKey: "Ctrl+Shift+F", category: "editor" },
  { id: "new-tab", label: "New Tab", defaultKey: "Ctrl+N", category: "editor" },
  { id: "close-tab", label: "Close Tab", defaultKey: "Ctrl+W", category: "editor" },

  // File
  { id: "save-query", label: "Save Query", defaultKey: "Ctrl+S", category: "file" },
  { id: "open-query", label: "Open Query", defaultKey: "Ctrl+O", category: "file" },
  { id: "save-query-as", label: "Save Query As", defaultKey: "Ctrl+Shift+S", category: "file" },

  // Connections
  { id: "new-connection", label: "New Connection", defaultKey: "Ctrl+Shift+N", category: "connections" },

  // AI
  { id: "open-ai-palette", label: "AI Command Palette", defaultKey: "Ctrl+K", category: "ai" },
  { id: "toggle-ai-panel", label: "Toggle AI Settings", defaultKey: "Ctrl+Shift+A", category: "ai" },

  // App
  { id: "open-settings", label: "Open Settings", defaultKey: "Ctrl+,", category: "app" },
];

export const CATEGORY_LABELS: Record<string, string> = {
  editor: "Editor",
  file: "File",
  connections: "Connections",
  ai: "AI",
  app: "Application",
};

/** Parse a shortcut string like "Ctrl+Shift+F" into parts for matching against KeyboardEvent. */
export function parseShortcut(shortcut: string): {
  ctrl: boolean;
  shift: boolean;
  alt: boolean;
  meta: boolean;
  key: string;
} {
  const parts = shortcut.split("+");
  const key = parts[parts.length - 1];
  return {
    ctrl: parts.includes("Ctrl"),
    shift: parts.includes("Shift"),
    alt: parts.includes("Alt"),
    meta: parts.includes("Meta"),
    key: key.toLowerCase(),
  };
}

/** Convert a KeyboardEvent to a shortcut string like "Ctrl+Shift+F". */
export function eventToShortcut(e: KeyboardEvent): string | null {
  // Ignore standalone modifier keys
  if (["Control", "Shift", "Alt", "Meta"].includes(e.key)) return null;

  const parts: string[] = [];
  if (e.ctrlKey || e.metaKey) parts.push("Ctrl");
  if (e.shiftKey) parts.push("Shift");
  if (e.altKey) parts.push("Alt");

  // Normalize key names
  let key = e.key;
  if (key === " ") key = "Space";
  else if (key.length === 1) key = key.toUpperCase();
  else if (key === ",") key = ",";

  // Function keys are already like "F5"
  parts.push(key);
  return parts.join("+");
}

/** Check if a KeyboardEvent matches a shortcut string. */
export function matchesShortcut(e: KeyboardEvent, shortcut: string): boolean {
  const parsed = parseShortcut(shortcut);
  const eventKey = e.key.toLowerCase();

  // Handle special keys
  let matchKey = parsed.key;
  if (matchKey === "f5") matchKey = "f5";
  if (matchKey === ",") matchKey = ",";

  return (
    (e.ctrlKey || e.metaKey) === parsed.ctrl &&
    e.shiftKey === parsed.shift &&
    e.altKey === parsed.alt &&
    eventKey === matchKey
  );
}
