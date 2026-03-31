import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import type { QueryHistoryEntry, SavedQuery } from "../types/queryHistory";

interface QueryHistoryState {
  history: QueryHistoryEntry[];
  savedQueries: SavedQuery[];

  loadHistory: () => Promise<void>;
  addHistoryEntry: (entry: QueryHistoryEntry) => Promise<void>;
  deleteHistoryEntry: (id: string) => Promise<void>;
  clearHistory: () => Promise<void>;

  loadSavedQueries: () => Promise<void>;
  createSavedQuery: (query: SavedQuery) => Promise<void>;
  updateSavedQuery: (query: SavedQuery) => Promise<void>;
  deleteSavedQuery: (id: string) => Promise<void>;
}

export const useQueryHistoryStore = create<QueryHistoryState>((set, get) => ({
  history: [],
  savedQueries: [],

  loadHistory: async () => {
    try {
      const history = await invoke<QueryHistoryEntry[]>("list_query_history");
      set({ history });
    } catch (e) {
      console.error("Failed to load query history:", e);
    }
  },

  addHistoryEntry: async (entry) => {
    try {
      await invoke("save_query_history_entry", { entry });
      const history = [...get().history, entry];
      set({ history });
    } catch (e) {
      console.error("Failed to save history entry:", e);
    }
  },

  deleteHistoryEntry: async (id) => {
    try {
      await invoke("delete_query_history_entry", { id });
      set({ history: get().history.filter((e) => e.id !== id) });
    } catch (e) {
      console.error("Failed to delete history entry:", e);
    }
  },

  clearHistory: async () => {
    try {
      await invoke("clear_query_history");
      set({ history: [] });
    } catch (e) {
      console.error("Failed to clear history:", e);
    }
  },

  loadSavedQueries: async () => {
    try {
      const savedQueries = await invoke<SavedQuery[]>("list_saved_queries");
      set({ savedQueries });
    } catch (e) {
      console.error("Failed to load saved queries:", e);
    }
  },

  createSavedQuery: async (query) => {
    try {
      await invoke("create_saved_query", { query });
      set({ savedQueries: [...get().savedQueries, query] });
    } catch (e) {
      console.error("Failed to save query:", e);
    }
  },

  updateSavedQuery: async (query) => {
    try {
      await invoke("update_saved_query", { query });
      set({
        savedQueries: get().savedQueries.map((q) =>
          q.id === query.id ? query : q,
        ),
      });
    } catch (e) {
      console.error("Failed to update saved query:", e);
    }
  },

  deleteSavedQuery: async (id) => {
    try {
      await invoke("delete_saved_query", { id });
      set({ savedQueries: get().savedQueries.filter((q) => q.id !== id) });
    } catch (e) {
      console.error("Failed to delete saved query:", e);
    }
  },
}));
