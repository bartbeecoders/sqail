import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import type {
  ObjectMetadata,
  MetadataProgress,
  MetadataDone,
  MetadataError,
  MetadataLogEntry,
} from "../types/metadata";

interface MetadataState {
  entries: ObjectMetadata[];
  generating: boolean;
  progress: MetadataProgress | null;
  error: string | null;

  /** Keys currently generating: "schema.object" for single, "schema:*" for whole schema */
  generatingKeys: Set<string>;

  loadMetadata: (connectionId: string) => Promise<void>;
  generateAll: (connectionId: string) => Promise<void>;
  generateSingle: (
    connectionId: string,
    schemaName: string,
    objectName: string,
    objectType: string,
  ) => Promise<void>;
  generateSchema: (connectionId: string, schemaName: string) => Promise<void>;
  updateEntry: (entry: ObjectMetadata) => Promise<void>;
  deleteAll: (connectionId: string) => Promise<void>;

  logEntries: MetadataLogEntry[];
  addLogEntry: (entry: MetadataLogEntry) => void;
  clearLog: () => void;

  setProgress: (progress: MetadataProgress) => void;
  setDone: (done: MetadataDone) => void;
  setError: (error: MetadataError) => void;

  isGenerating: (schemaName: string, objectName?: string) => boolean;

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
  generatingKeys: new Set(),
  logEntries: [],

  addLogEntry: (entry) => {
    set((s) => ({ logEntries: [...s.logEntries, entry] }));
  },

  clearLog: () => {
    set({ logEntries: [] });
  },

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
    const key = `${schemaName}.${objectName}`;
    set((s) => {
      const next = new Set(s.generatingKeys);
      next.add(key);
      return { generatingKeys: next, error: null };
    });
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
      set({ error: String(e) });
    } finally {
      set((s) => {
        const next = new Set(s.generatingKeys);
        next.delete(key);
        return { generatingKeys: next };
      });
    }
  },

  generateSchema: async (connectionId, schemaName) => {
    const schemaKey = `${schemaName}:*`;
    set((s) => {
      const next = new Set(s.generatingKeys);
      next.add(schemaKey);
      return { generatingKeys: next, generating: true, progress: null, error: null };
    });
    try {
      await invoke<string>("generate_schema_metadata", {
        connectionId,
        schemaName,
      });
      // The async backend will emit metadata:done which clears generatingKeys
    } catch (e) {
      // On error, clean up the key ourselves
      set((s) => {
        const next = new Set(s.generatingKeys);
        next.delete(schemaKey);
        return { generatingKeys: next, generating: false, error: String(e) };
      });
    }
    // Don't remove the key in finally — setDone (from metadata:done event) handles cleanup
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
    // When a batch completes, reload entries so spinners disappear for finished objects
    if (progress.status === "complete") {
      get().loadMetadata(progress.connectionId);
    }
  },

  setDone: (done) => {
    set({ generating: false, progress: null, generatingKeys: new Set() });
    // Reload metadata entries after generation completes
    get().loadMetadata(done.connectionId);
  },

  setError: (error) => {
    set({ generating: false, error: error.error, generatingKeys: new Set() });
  },

  isGenerating: (schemaName, objectName) => {
    const { generatingKeys: keys, entries } = get();
    if (objectName) {
      // Check if this specific object is generating
      if (keys.has(`${schemaName}.${objectName}`)) return true;
      // If schema-wide generation is running, only show generating for objects without metadata
      if (keys.has(`${schemaName}:*`)) {
        return !entries.some(
          (e) => e.schemaName === schemaName && e.objectName === objectName,
        );
      }
      return false;
    }
    // Check if the whole schema is generating
    return keys.has(`${schemaName}:*`);
  },

  getForObject: (schemaName, objectName) => {
    return get().entries.find(
      (e) => e.schemaName === schemaName && e.objectName === objectName,
    );
  },
}));
