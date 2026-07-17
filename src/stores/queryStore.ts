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
  /** SQL of the most recently executed query — used by "Fix with AI" on errors. */
  lastExecutedSql: string;

  executeQuery: (connectionId: string, sql: string) => Promise<void>;
  /** Abort the in-flight query on the backend (no-op if nothing is running). */
  cancelQuery: () => Promise<void>;
  setActiveResultIndex: (index: number) => void;
  clear: () => void;
}

export const useQueryStore = create<QueryState>((set, get) => ({
  results: [],
  activeResultIndex: 0,
  totalTimeMs: 0,
  error: null,
  loading: false,
  lastExecutedSql: "",

  executeQuery: async (connectionId, sql) => {
    set({ loading: true, error: null, lastExecutedSql: sql });
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

  cancelQuery: async () => {
    if (!get().loading) return;
    try {
      await invoke<boolean>("cancel_query");
      // `executeQuery` will finish with a cancelled response and clear loading.
    } catch (e) {
      // If the backend has no active query, still drop the loading spinner so the UI recovers.
      set({ loading: false, error: String(e) });
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
