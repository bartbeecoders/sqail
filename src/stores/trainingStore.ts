import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  DatasetOptions,
  DatasetStats,
  EnvCheck,
  TrainedModel,
  TrainingHyperparams,
  TrainingJob,
  TrainingLogLine,
} from "../types/training";
import {
  DEFAULT_DATASET_OPTIONS,
  DEFAULT_HYPERPARAMS,
} from "../types/training";
import { useInlineAiStore } from "./inlineAiStore";

const LOG_CAPACITY = 300;

interface TrainingState {
  env: EnvCheck | null;
  envChecking: boolean;
  jobs: TrainingJob[];
  trainedModels: TrainedModel[];
  previewStats: DatasetStats | null;
  previewing: boolean;
  previewError: string | null;
  /** Logs keyed by job id; capped per job. */
  logs: Record<string, string[]>;
  /** User-visible composer state — form for kicking off a new job. */
  form: {
    connectionId: string;
    baseModelId: string;
    options: DatasetOptions;
    hyperparams: TrainingHyperparams;
  };

  setFormConnection: (id: string) => void;
  setFormBaseModel: (id: string) => void;
  setFormOptions: (edit: (o: DatasetOptions) => DatasetOptions) => void;
  setFormHyperparams: (edit: (h: TrainingHyperparams) => TrainingHyperparams) => void;

  checkEnv: () => Promise<void>;
  refreshJobs: () => Promise<void>;
  refreshTrainedModels: () => Promise<void>;
  previewDataset: () => Promise<void>;
  startJob: () => Promise<string | null>;
  cancelJob: (id: string) => Promise<void>;
  deleteTrainedModel: (id: string) => Promise<void>;
  /** Pull the full on-disk trainer log for a finished (or failed) job. */
  fetchLog: (job: TrainingJob) => Promise<void>;
  /** Convert + (re)start the inline-AI sidecar with this adapter. */
  activateTrainedModel: (id: string) => Promise<void>;
  /** Restart the sidecar on the base model only. */
  deactivateTrainedModel: () => Promise<void>;
  /** Convert the adapter to GGUF without touching the sidecar. */
  convertTrainedModel: (id: string) => Promise<string | null>;

  activating: string | null;
  activateError: string | null;

  attachListeners: () => Promise<UnlistenFn>;
  applyJobUpdate: (job: TrainingJob) => void;
  appendLog: (line: TrainingLogLine) => void;
}

export const useTrainingStore = create<TrainingState>((set, get) => ({
  env: null,
  envChecking: false,
  jobs: [],
  trainedModels: [],
  previewStats: null,
  previewing: false,
  previewError: null,
  logs: {},
  activating: null,
  activateError: null,
  form: {
    connectionId: "",
    baseModelId: "qwen-coder-3b-q4",
    options: { ...DEFAULT_DATASET_OPTIONS },
    hyperparams: { ...DEFAULT_HYPERPARAMS },
  },

  setFormConnection: (id) =>
    set((s) => ({ form: { ...s.form, connectionId: id } })),
  setFormBaseModel: (id) =>
    set((s) => ({ form: { ...s.form, baseModelId: id } })),
  setFormOptions: (edit) =>
    set((s) => ({ form: { ...s.form, options: edit(s.form.options) } })),
  setFormHyperparams: (edit) =>
    set((s) => ({
      form: { ...s.form, hyperparams: edit(s.form.hyperparams) },
    })),

  checkEnv: async () => {
    set({ envChecking: true });
    try {
      const env = await invoke<EnvCheck>("training_check_env");
      set({ env, envChecking: false });
    } catch (e) {
      console.error("training_check_env failed", e);
      set({ envChecking: false });
    }
  },

  refreshJobs: async () => {
    try {
      const jobs = await invoke<TrainingJob[]>("training_list_jobs");
      set({ jobs });
    } catch (e) {
      console.error("training_list_jobs", e);
    }
  },

  refreshTrainedModels: async () => {
    try {
      const trainedModels = await invoke<TrainedModel[]>(
        "training_list_models",
      );
      // The Inline AI picker's selection is stored on `inlineAiStore`.
      // If the selected adapter just got deleted, drop the selection
      // so the next sidecar restart goes back to the raw base model.
      const inline = useInlineAiStore.getState();
      if (
        inline.trainedModelId &&
        !trainedModels.some((m) => m.id === inline.trainedModelId)
      ) {
        inline.updateSetting("trainedModelId", null);
      }
      set({ trainedModels });
    } catch (e) {
      console.error("training_list_models", e);
    }
  },

  previewDataset: async () => {
    const { form } = get();
    if (!form.connectionId) {
      set({ previewError: "Select a connection first." });
      return;
    }
    set({ previewing: true, previewError: null });
    try {
      const stats = await invoke<DatasetStats>("training_preview_dataset", {
        connectionId: form.connectionId,
        options: form.options,
      });
      set({ previewStats: stats, previewing: false });
    } catch (e) {
      set({
        previewing: false,
        previewError: typeof e === "string" ? e : String(e),
      });
    }
  },

  startJob: async () => {
    const { form } = get();
    if (!form.connectionId) return null;
    try {
      const id = await invoke<string>("training_start", {
        connectionId: form.connectionId,
        baseModelId: form.baseModelId,
        options: form.options,
        hyperparams: form.hyperparams,
      });
      await get().refreshJobs();
      return id;
    } catch (e) {
      console.error("training_start failed", e);
      set({ previewError: typeof e === "string" ? e : String(e) });
      return null;
    }
  },

  cancelJob: async (id) => {
    try {
      await invoke("training_cancel", { jobId: id });
    } catch (e) {
      console.error("training_cancel failed", e);
    }
  },

  deleteTrainedModel: async (id) => {
    try {
      await invoke("training_delete_model", { modelId: id });
      await get().refreshTrainedModels();
    } catch (e) {
      console.error("training_delete_model failed", e);
    }
  },

  fetchLog: async (job) => {
    try {
      const contents = await invoke<string>("training_read_log", {
        connectionId: job.connectionId,
        jobId: job.id,
      });
      // Replace the in-memory ring buffer with whatever's on disk.
      set((s) => ({
        logs: { ...s.logs, [job.id]: contents.split("\n") },
      }));
    } catch (e) {
      console.error("training_read_log failed", e);
    }
  },

  activateTrainedModel: async (id) => {
    set({ activating: id, activateError: null });
    try {
      const adapter = get().trainedModels.find((m) => m.id === id);
      if (!adapter) throw new Error(`trained model not found: ${id}`);
      // Single source of truth: write the selection into the inline-AI
      // store and let its startSidecar() pick up the adapter via the
      // standard `inline_sidecar_start` command. Avoids the Training-
      // tab and Inline-AI-tab getting out of sync.
      const inline = useInlineAiStore.getState();
      inline.updateSetting("modelId", adapter.baseModelId);
      inline.updateSetting("trainedModelId", adapter.id);
      await inline.startSidecar();
    } catch (e) {
      set({ activateError: typeof e === "string" ? e : String(e) });
    } finally {
      set({ activating: null });
    }
  },

  deactivateTrainedModel: async () => {
    set({ activating: "__base__", activateError: null });
    try {
      const inline = useInlineAiStore.getState();
      inline.updateSetting("trainedModelId", null);
      await inline.startSidecar();
    } catch (e) {
      set({ activateError: typeof e === "string" ? e : String(e) });
    } finally {
      set({ activating: null });
    }
  },

  convertTrainedModel: async (id) => {
    try {
      const path = await invoke<string>("training_convert_model", {
        modelId: id,
      });
      await get().refreshTrainedModels();
      return path;
    } catch (e) {
      set({ activateError: typeof e === "string" ? e : String(e) });
      return null;
    }
  },

  applyJobUpdate: (job) =>
    set((s) => {
      const idx = s.jobs.findIndex((j) => j.id === job.id);
      const next = [...s.jobs];
      if (idx >= 0) {
        next[idx] = job;
      } else {
        next.unshift(job);
      }
      return { jobs: next };
    }),

  appendLog: ({ id, line }) =>
    set((s) => {
      const existing = s.logs[id] ?? [];
      const next = [...existing, line];
      if (next.length > LOG_CAPACITY) {
        next.splice(0, next.length - LOG_CAPACITY);
      }
      return { logs: { ...s.logs, [id]: next } };
    }),

  attachListeners: async () => {
    const unlistenUpdate = await listen<TrainingJob>(
      "training:update",
      (event) => {
        get().applyJobUpdate(event.payload);
      },
    );
    const unlistenDone = await listen<TrainingJob | null>(
      "training:done",
      (event) => {
        if (event.payload) {
          get().applyJobUpdate(event.payload);
        }
        get().refreshTrainedModels();
      },
    );
    const unlistenLog = await listen<TrainingLogLine>(
      "training:log",
      (event) => {
        get().appendLog(event.payload);
      },
    );
    return () => {
      unlistenUpdate();
      unlistenDone();
      unlistenLog();
    };
  },
}));
