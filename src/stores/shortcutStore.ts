import { create } from "zustand";
import { SHORTCUT_ACTIONS } from "../types/shortcuts";

const STORAGE_KEY = "sqail_shortcuts";

type ShortcutMap = Record<string, string>; // actionId → key combo string

function getDefaults(): ShortcutMap {
  const map: ShortcutMap = {};
  for (const action of SHORTCUT_ACTIONS) {
    map[action.id] = action.defaultKey;
  }
  return map;
}

function loadShortcuts(): ShortcutMap {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw) {
      const saved = JSON.parse(raw) as ShortcutMap;
      // Merge with defaults so new actions are always present
      return { ...getDefaults(), ...saved };
    }
  } catch {
    // ignore
  }
  return getDefaults();
}

interface ShortcutState {
  shortcuts: ShortcutMap;

  getShortcut: (actionId: string) => string;
  updateShortcut: (actionId: string, key: string) => void;
  resetShortcut: (actionId: string) => void;
  resetDefaults: () => void;
}

export const useShortcutStore = create<ShortcutState>((set, get) => ({
  shortcuts: loadShortcuts(),

  getShortcut: (actionId) => {
    const { shortcuts } = get();
    return shortcuts[actionId] ?? "";
  },

  updateShortcut: (actionId, key) => {
    set((s) => {
      const shortcuts = { ...s.shortcuts, [actionId]: key };
      localStorage.setItem(STORAGE_KEY, JSON.stringify(shortcuts));
      return { shortcuts };
    });
  },

  resetShortcut: (actionId) => {
    const defaults = getDefaults();
    const defaultKey = defaults[actionId] ?? "";
    set((s) => {
      const shortcuts = { ...s.shortcuts, [actionId]: defaultKey };
      localStorage.setItem(STORAGE_KEY, JSON.stringify(shortcuts));
      return { shortcuts };
    });
  },

  resetDefaults: () => {
    const defaults = getDefaults();
    localStorage.setItem(STORAGE_KEY, JSON.stringify(defaults));
    set({ shortcuts: defaults });
  },
}));
