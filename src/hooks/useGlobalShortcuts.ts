import { useEffect } from "react";
import { useShortcutStore } from "../stores/shortcutStore";
import { matchesShortcut } from "../types/shortcuts";

export type ShortcutHandlers = Record<string, () => void>;

/**
 * Global keyboard shortcut listener.
 * Attaches a single keydown handler on `window` that checks all registered
 * shortcuts from the store and dispatches the corresponding action handler.
 */
export function useGlobalShortcuts(handlers: ShortcutHandlers) {
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      // Don't intercept shortcuts when typing in input/textarea (except our known global ones)
      const tag = (e.target as HTMLElement)?.tagName;
      const isInput = tag === "INPUT" || tag === "TEXTAREA";

      const shortcuts = useShortcutStore.getState().shortcuts;

      for (const [actionId, keyCombo] of Object.entries(shortcuts)) {
        if (!keyCombo) continue;
        if (matchesShortcut(e, keyCombo)) {
          // Allow some shortcuts even in inputs (save, open, settings)
          const allowInInput = [
            "save-query",
            "save-query-as",
            "open-query",
            "open-settings",
            "run-query",
            "open-ai-palette",
          ].includes(actionId);

          if (isInput && !allowInInput) continue;

          const handler = handlers[actionId];
          if (handler) {
            e.preventDefault();
            e.stopPropagation();
            handler();
            return;
          }
        }
      }
    };

    window.addEventListener("keydown", onKeyDown, true); // capture phase to beat Monaco
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, [handlers]);
}
