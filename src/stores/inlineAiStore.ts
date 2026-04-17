import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";

const STORAGE_KEY = "sqail_inline_ai_settings";

/** Persisted user preferences. */
export interface InlineAiSettings {
  enabled: boolean;
  modelId: string;
  autoStart: boolean;
  debounceMs: number;
  maxTokens: number;
  temperature: number;
  cpuOnly: boolean;
  ctxSize: number;
  /** When true, show a per-completion latency table in the settings tab. */
  devMode: boolean;
}

const DEFAULTS: InlineAiSettings = {
  enabled: false,
  modelId: "qwen-coder-3b-q4",
  autoStart: true,
  debounceMs: 150,
  maxTokens: 48,
  temperature: 0.2,
  cpuOnly: false,
  ctxSize: 4096,
  devMode: false,
};

/** Mirror of Rust-side `SidecarStatus`. */
export type SidecarState =
  | { state: "stopped" }
  | { state: "starting"; modelId: string }
  | { state: "ready"; modelId: string; port: number }
  | { state: "error"; message: string };

/** One entry in the model catalog with on-disk presence info. */
export interface ModelListItem {
  id: string;
  displayName: string;
  tier: "default" | "performance" | "low-end";
  filename: string;
  url: string;
  sizeBytes: number;
  minVramMib: number;
  sha256: string | null;
  downloaded: boolean;
  diskSize: number;
}

export interface DownloadState {
  downloaded: number;
  total: number;
  phase: "started" | "progress" | "verifying" | "completed" | "cancelled" | "error";
  error?: string;
}

export interface CompletionTelemetry {
  at: number;
  tokens: number;
  ttftMs: number;
  totalMs: number;
  stopReason: string;
  preview: string;
}

const TELEMETRY_CAPACITY = 20;

interface InlineAiState extends InlineAiSettings {
  sidecar: SidecarState;
  models: ModelListItem[];
  downloads: Record<string, DownloadState>;
  telemetry: CompletionTelemetry[];

  // Settings writers
  updateSetting: <K extends keyof InlineAiSettings>(
    key: K,
    value: InlineAiSettings[K],
  ) => void;
  toggleEnabled: () => Promise<void>;

  // Runtime
  refreshModels: () => Promise<void>;
  refreshStatus: () => Promise<void>;
  startSidecar: () => Promise<void>;
  stopSidecar: () => Promise<void>;
  downloadModel: (id: string) => Promise<void>;
  cancelDownload: (id: string) => Promise<void>;
  deleteModel: (id: string) => Promise<void>;

  // Event adapters (called from useInlineAiLifecycle)
  applySidecarStatus: (s: SidecarState) => void;
  applyDownloadProgress: (p: { id: string } & DownloadState) => void;
  recordCompletion: (sample: Omit<CompletionTelemetry, "at">) => void;
  clearTelemetry: () => void;
}

function loadSettings(): InlineAiSettings {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw) return { ...DEFAULTS, ...JSON.parse(raw) };
  } catch {
    /* ignore */
  }
  return { ...DEFAULTS };
}

function saveSettings(settings: InlineAiSettings): void {
  localStorage.setItem(STORAGE_KEY, JSON.stringify(settings));
}

export const useInlineAiStore = create<InlineAiState>((set, get) => ({
  ...loadSettings(),
  sidecar: { state: "stopped" },
  models: [],
  downloads: {},
  telemetry: [],

  updateSetting: (key, value) => {
    set({ [key]: value } as Partial<InlineAiSettings>);
    const snapshot = get();
    const toSave: InlineAiSettings = {
      enabled: snapshot.enabled,
      modelId: snapshot.modelId,
      autoStart: snapshot.autoStart,
      debounceMs: snapshot.debounceMs,
      maxTokens: snapshot.maxTokens,
      temperature: snapshot.temperature,
      cpuOnly: snapshot.cpuOnly,
      ctxSize: snapshot.ctxSize,
      devMode: snapshot.devMode,
    };
    saveSettings(toSave);
  },

  toggleEnabled: async () => {
    const was = get().enabled;
    get().updateSetting("enabled", !was);
    // Side effects: when flipping on, auto-start if the selected model
    // is ready on disk. When flipping off, stop the sidecar.
    if (!was) {
      const { modelId, models } = get();
      const m = models.find((x) => x.id === modelId);
      if (m?.downloaded && get().sidecar.state !== "ready") {
        try {
          await get().startSidecar();
        } catch {
          /* surfaced via sidecar state */
        }
      }
    } else if (get().sidecar.state !== "stopped") {
      await get().stopSidecar();
    }
  },

  refreshModels: async () => {
    try {
      const models = await invoke<ModelListItem[]>("inline_model_list");
      set({ models });
    } catch (e) {
      console.error("inline_model_list failed:", e);
    }
  },

  refreshStatus: async () => {
    try {
      const status = await invoke<SidecarState>("inline_sidecar_status");
      set({ sidecar: status });
    } catch (e) {
      console.error("inline_sidecar_status failed:", e);
    }
  },

  startSidecar: async () => {
    const { modelId, ctxSize, cpuOnly } = get();
    try {
      await invoke("inline_sidecar_start", {
        modelId,
        ctxSize,
        cpuOnly,
      });
    } catch (e) {
      set({ sidecar: { state: "error", message: String(e) } });
      throw e;
    }
  },

  stopSidecar: async () => {
    try {
      await invoke("inline_sidecar_stop");
    } catch (e) {
      console.error("inline_sidecar_stop failed:", e);
    }
  },

  downloadModel: async (id) => {
    try {
      await invoke("inline_model_download", { modelId: id });
      await get().refreshModels();
    } catch (e) {
      console.error("inline_model_download failed:", e);
    }
  },

  cancelDownload: async (id) => {
    try {
      await invoke("inline_model_cancel_download", { modelId: id });
    } catch (e) {
      console.error("inline_model_cancel_download failed:", e);
    }
  },

  deleteModel: async (id) => {
    try {
      await invoke("inline_model_delete", { modelId: id });
      await get().refreshModels();
    } catch (e) {
      console.error("inline_model_delete failed:", e);
    }
  },

  applySidecarStatus: (status) => {
    set({ sidecar: status });
  },

  recordCompletion: (sample) => {
    set((s) => ({
      telemetry: [
        { ...sample, at: Date.now() },
        ...s.telemetry,
      ].slice(0, TELEMETRY_CAPACITY),
    }));
  },

  clearTelemetry: () => set({ telemetry: [] }),

  applyDownloadProgress: (p) => {
    set((s) => ({
      downloads: {
        ...s.downloads,
        [p.id]: {
          downloaded: p.downloaded,
          total: p.total,
          phase: p.phase,
          error: p.error,
        },
      },
    }));
    // On terminal states, refresh the catalog list so the downloaded flag flips.
    if (p.phase === "completed" || p.phase === "error" || p.phase === "cancelled") {
      void get().refreshModels();
    }
  },
}));
