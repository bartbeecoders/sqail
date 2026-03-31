import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import type { QueryResponse, QueryResult } from "../types/query";
import { useQueryHistoryStore } from "./queryHistoryStore";
import { useConnectionStore } from "./connectionStore";

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

      // Auto-log to query history
      const conn = useConnectionStore.getState().connections.find((c) => c.id === connectionId);
      const totalRows = response.results.reduce((sum, r) => sum + r.rowCount, 0);
      useQueryHistoryStore.getState().addHistoryEntry({
        id: crypto.randomUUID(),
        timestamp: new Date().toISOString(),
        connectionId,
        connectionName: conn?.name,
        query: sql,
        executionTimeMs: response.totalTimeMs,
        rowCount: totalRows,
        success: !response.error,
        errorMessage: response.error ?? undefined,
      });
    } catch (e) {
      set({
        results: [],
        activeResultIndex: 0,
        totalTimeMs: 0,
        error: String(e),
        loading: false,
      });

      // Log failed queries too
      const conn = useConnectionStore.getState().connections.find((c) => c.id === connectionId);
      useQueryHistoryStore.getState().addHistoryEntry({
        id: crypto.randomUUID(),
        timestamp: new Date().toISOString(),
        connectionId,
        connectionName: conn?.name,
        query: sql,
        executionTimeMs: 0,
        success: false,
        errorMessage: String(e),
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
