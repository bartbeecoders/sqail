import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import type { QueryResponse, QueryResult } from "../types/query";

interface QueryState {
  results: QueryResult[];
  activeResultIndex: number;
  totalTimeMs: number;
  error: string | null;
  loading: boolean;

  executeQuery: (connectionId: string, sql: string) => Promise<void>;
  setActiveResultIndex: (index: number) => void;
  clear: () => void;
}

export const useQueryStore = create<QueryState>((set) => ({
  results: [],
  activeResultIndex: 0,
  totalTimeMs: 0,
  error: null,
  loading: false,

  executeQuery: async (connectionId, sql) => {
    set({ loading: true, error: null });
    try {
      const response = await invoke<QueryResponse>("execute_query", {
        connectionId,
        sql,
      });
      set({
        results: response.results,
        activeResultIndex: 0,
        totalTimeMs: response.totalTimeMs,
        error: response.error,
        loading: false,
      });
    } catch (e) {
      set({
        results: [],
        activeResultIndex: 0,
        totalTimeMs: 0,
        error: String(e),
        loading: false,
      });
    }
  },

  setActiveResultIndex: (index) => set({ activeResultIndex: index }),

  clear: () =>
    set({
      results: [],
      activeResultIndex: 0,
      totalTimeMs: 0,
      error: null,
      loading: false,
    }),
}));
