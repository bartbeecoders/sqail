import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import type { SchemaInfo, TableInfo, ColumnInfo, IndexInfo, RoutineInfo } from "../types/schema";

interface SchemaState {
  schemas: SchemaInfo[];
  tables: Record<string, TableInfo[]>; // keyed by schema name
  columns: Record<string, ColumnInfo[]>; // keyed by "schema.table"
  indexes: Record<string, IndexInfo[]>; // keyed by "schema.table"
  routines: Record<string, RoutineInfo[]>; // keyed by schema name
  loading: boolean;
  error: string | null;
  connectionId: string | null;

  loadSchemas: (connectionId: string) => Promise<void>;
  loadTables: (connectionId: string, schemaName: string) => Promise<void>;
  loadColumns: (connectionId: string, schemaName: string, tableName: string) => Promise<void>;
  loadIndexes: (connectionId: string, schemaName: string, tableName: string) => Promise<void>;
  loadRoutines: (connectionId: string, schemaName: string) => Promise<void>;
  clear: () => void;

  /** Flat list of all loaded table names (for completions) */
  getAllTableNames: () => string[];
  /** Flat list of all loaded column names (for completions) */
  getAllColumnNames: () => string[];
}

export const useSchemaStore = create<SchemaState>((set, get) => ({
  schemas: [],
  tables: {},
  columns: {},
  indexes: {},
  routines: {},
  loading: false,
  error: null,
  connectionId: null,

  loadSchemas: async (connectionId) => {
    set({ loading: true, connectionId, error: null });
    try {
      const schemas = await invoke<SchemaInfo[]>("list_schemas", { connectionId });
      set({ schemas, tables: {}, columns: {}, indexes: {}, routines: {} });
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      console.error("Failed to load schemas:", msg);
      set({ error: msg, schemas: [] });
    } finally {
      set({ loading: false });
    }
  },

  loadTables: async (connectionId, schemaName) => {
    try {
      const tables = await invoke<TableInfo[]>("list_tables", { connectionId, schemaName });
      set((s) => ({ tables: { ...s.tables, [schemaName]: tables } }));
    } catch (e) {
      console.error("Failed to load tables:", e);
    }
  },

  loadColumns: async (connectionId, schemaName, tableName) => {
    const key = `${schemaName}.${tableName}`;
    try {
      const columns = await invoke<ColumnInfo[]>("list_columns", {
        connectionId,
        schemaName,
        tableName,
      });
      set((s) => ({ columns: { ...s.columns, [key]: columns } }));
    } catch (e) {
      console.error("Failed to load columns:", e);
    }
  },

  loadIndexes: async (connectionId, schemaName, tableName) => {
    const key = `${schemaName}.${tableName}`;
    try {
      const indexes = await invoke<IndexInfo[]>("list_indexes", {
        connectionId,
        schemaName,
        tableName,
      });
      set((s) => ({ indexes: { ...s.indexes, [key]: indexes } }));
    } catch (e) {
      console.error("Failed to load indexes:", e);
    }
  },

  loadRoutines: async (connectionId, schemaName) => {
    try {
      const routines = await invoke<RoutineInfo[]>("list_routines", {
        connectionId,
        schemaName,
      });
      set((s) => ({ routines: { ...s.routines, [schemaName]: routines } }));
    } catch (e) {
      console.error("Failed to load routines:", e);
    }
  },

  clear: () =>
    set({
      schemas: [],
      tables: {},
      columns: {},
      indexes: {},
      routines: {},
      error: null,
      connectionId: null,
    }),

  getAllTableNames: () => {
    const { tables } = get();
    const names: string[] = [];
    for (const list of Object.values(tables)) {
      for (const t of list) names.push(t.name);
    }
    return names;
  },

  getAllColumnNames: () => {
    const { columns } = get();
    const names = new Set<string>();
    for (const list of Object.values(columns)) {
      for (const c of list) names.add(c.name);
    }
    return Array.from(names);
  },
}));
