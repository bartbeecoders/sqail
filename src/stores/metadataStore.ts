import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import type {
  ObjectMetadata,
  MetadataProgress,
  MetadataDone,
  MetadataError,
} from "../types/metadata";

interface MetadataState {
  entries: ObjectMetadata[];
  generating: boolean;
  progress: MetadataProgress | null;
  error: string | null;

  loadMetadata: (connectionId: string) => Promise<void>;
  generateAll: (connectionId: string) => Promise<void>;
  generateSingle: (
    connectionId: string,
    schemaName: string,
    objectName: string,
    objectType: string,
  ) => Promise<void>;
  updateEntry: (entry: ObjectMetadata) => Promise<void>;
  deleteAll: (connectionId: string) => Promise<void>;

  setProgress: (progress: MetadataProgress) => void;
  setDone: (done: MetadataDone) => void;
  setError: (error: MetadataError) => void;

  getForObject: (
    schemaName: string,
    objectName: string,
  ) => ObjectMetadata | undefined;
}

export const useMetadataStore = create<MetadataState>((set, get) => ({
  entries: [],
  generating: false,
  progress: null,
  error: null,

  loadMetadata: async (connectionId) => {
    try {
      const entries = await invoke<ObjectMetadata[]>("list_metadata", {
        connectionId,
      });
      set({ entries });
    } catch (e) {
      console.error("Failed to load metadata:", e);
    }
  },

  generateAll: async (connectionId) => {
    set({ generating: true, progress: null, error: null });
    try {
      await invoke<string>("generate_all_metadata", { connectionId });
    } catch (e) {
      set({ generating: false, error: String(e) });
    }
  },

  generateSingle: async (connectionId, schemaName, objectName, objectType) => {
    try {
      await invoke<string>("generate_single_metadata", {
        connectionId,
        schemaName,
        objectName,
        objectType,
      });
      // Reload after single generation
      await get().loadMetadata(connectionId);
    } catch (e) {
      console.error("Failed to generate metadata:", e);
      throw e;
    }
  },

  updateEntry: async (entry) => {
    try {
      await invoke<void>("update_metadata", { entry });
      // Update local state
      set((s) => ({
        entries: s.entries.map((e) => (e.id === entry.id ? entry : e)),
      }));
    } catch (e) {
      console.error("Failed to update metadata:", e);
      throw e;
    }
  },

  deleteAll: async (connectionId) => {
    try {
      await invoke<void>("delete_all_metadata", { connectionId });
      set((s) => ({
        entries: s.entries.filter((e) => e.connectionId !== connectionId),
      }));
    } catch (e) {
      console.error("Failed to delete metadata:", e);
    }
  },

  setProgress: (progress) => {
    set({ progress });
  },

  setDone: (done) => {
    set({ generating: false, progress: null });
    // Reload metadata entries after generation completes
    get().loadMetadata(done.connectionId);
  },

  setError: (error) => {
    set({ generating: false, error: error.error });
  },

  getForObject: (schemaName, objectName) => {
    return get().entries.find(
      (e) => e.schemaName === schemaName && e.objectName === objectName,
    );
  },
}));
