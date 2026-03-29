import { create } from "zustand";

const STORAGE_KEY = "sqail_settings";

export interface AppSettings {
  // Editor
  editorFontSize: number;
  editorFontFamily: string;
  editorTabSize: number;
  editorMinimap: boolean;
  editorWordWrap: boolean;
  editorLineNumbers: boolean;

  // Appearance
  theme: "system" | "light" | "dark";

  // Query
  defaultRowLimit: number;
  queryTimeoutSeconds: number;
}

const DEFAULTS: AppSettings = {
  editorFontSize: 14,
  editorFontFamily: "'JetBrains Mono', 'Fira Code', 'Cascadia Code', Consolas, monospace",
  editorTabSize: 2,
  editorMinimap: false,
  editorWordWrap: true,
  editorLineNumbers: true,
  theme: "system",
  defaultRowLimit: 1000,
  queryTimeoutSeconds: 30,
};

function loadSettings(): AppSettings {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw) {
      return { ...DEFAULTS, ...JSON.parse(raw) };
    }
  } catch { /* ignore */ }
  return { ...DEFAULTS };
}

interface SettingsState extends AppSettings {
  updateSetting: <K extends keyof AppSettings>(key: K, value: AppSettings[K]) => void;
  resetSettings: () => void;
}

export const useSettingsStore = create<SettingsState>((set, get) => ({
  ...loadSettings(),

  updateSetting: (key, value) => {
    set({ [key]: value } as Partial<AppSettings>);
    const state = get();
    const toSave: Partial<AppSettings> = {};
    for (const k of Object.keys(DEFAULTS) as (keyof AppSettings)[]) {
      toSave[k] = state[k] as never;
    }
    // Apply after set
    toSave[key] = value as never;
    localStorage.setItem(STORAGE_KEY, JSON.stringify(toSave));

    // Apply theme immediately
    if (key === "theme") applyTheme(value as AppSettings["theme"]);
  },

  resetSettings: () => {
    localStorage.removeItem(STORAGE_KEY);
    set({ ...DEFAULTS });
    applyTheme(DEFAULTS.theme);
  },
}));

function applyTheme(theme: AppSettings["theme"]) {
  const root = document.documentElement;
  root.classList.remove("light", "dark");
  if (theme === "light" || theme === "dark") {
    root.classList.add(theme);
  }
}

// Apply theme on initial load
applyTheme(loadSettings().theme);
