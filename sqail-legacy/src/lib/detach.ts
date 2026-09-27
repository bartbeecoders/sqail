import { getCurrentWindow } from "@tauri-apps/api/window";
import { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import type { EditorTab } from "../types/editor";

const DETACHED_LABEL_PREFIX = "detached-";
const HANDOFF_KEY_PREFIX = "sqail_detach_handoff_";

/** True if the current window is a detached tab window. */
export function isDetachedWindow(): boolean {
  try {
    const label = getCurrentWindow().label;
    return label.startsWith(DETACHED_LABEL_PREFIX);
  } catch {
    return false;
  }
}

/** Storage key for this window's persisted tab list. Detached windows return a
 *  label-specific key so they never clobber the main window's tabs. */
export function tabStorageKey(): string {
  try {
    const label = getCurrentWindow().label;
    if (label.startsWith(DETACHED_LABEL_PREFIX)) {
      return `sqail_tabs_${label}`;
    }
  } catch {
    // Not running inside Tauri — fall through to the legacy key.
  }
  return "sqail_tabs";
}

function handoffKey(label: string): string {
  return `${HANDOFF_KEY_PREFIX}${label}`;
}

/** Read and clear the handoff payload that a parent window stashed for us. */
export function consumeHandoffTab(): EditorTab | null {
  try {
    const label = getCurrentWindow().label;
    if (!label.startsWith(DETACHED_LABEL_PREFIX)) return null;
    const key = handoffKey(label);
    const raw = localStorage.getItem(key);
    if (!raw) return null;
    localStorage.removeItem(key);
    return JSON.parse(raw) as EditorTab;
  } catch {
    return null;
  }
}

/** Open the given tab in a new OS window and remove it from the current window.
 *  The new window shares the same Rust backend (connections, AI, etc.) but
 *  owns its tab list in memory only — closing the window destroys the tab. */
export async function detachTab(
  tab: EditorTab,
  options?: { title?: string },
): Promise<void> {
  const label = `${DETACHED_LABEL_PREFIX}${crypto.randomUUID().replace(/-/g, "").slice(0, 12)}`;

  // Stash the full tab payload under a label-keyed handoff. The new window
  // consumes it on boot and then deletes it.
  localStorage.setItem(handoffKey(label), JSON.stringify(tab));

  const title = options?.title ?? tab.title ?? "sqail";

  // Create the new webview window. Pass the detached marker in the URL hash
  // so React boots into detached mode.
  const url = `index.html#detached=${encodeURIComponent(label)}`;
  const win = new WebviewWindow(label, {
    url,
    title: `sqail — ${title}`,
    width: 1100,
    height: 720,
    center: false,
    decorations: true,
    resizable: true,
  });

  await new Promise<void>((resolve, reject) => {
    win.once("tauri://created", () => resolve());
    win.once("tauri://error", (e) => {
      // Clean up the handoff so it doesn't linger.
      localStorage.removeItem(handoffKey(label));
      reject(new Error(String(e.payload ?? "Failed to create window")));
    });
  });
}
