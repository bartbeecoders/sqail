import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import type { ConnectionConfig } from "../types/connection";

interface ConnectionState {
  connections: ConnectionConfig[];
  activeConnectionId: string | null;
  loading: boolean;

  loadConnections: () => Promise<void>;
  createConnection: (config: ConnectionConfig) => Promise<ConnectionConfig>;
  updateConnection: (config: ConnectionConfig) => Promise<void>;
  deleteConnection: (id: string) => Promise<void>;
  testConnection: (config: ConnectionConfig) => Promise<string>;
  connect: (id: string) => Promise<void>;
  disconnect: (id: string) => Promise<void>;
}

export const useConnectionStore = create<ConnectionState>((set, get) => ({
  connections: [],
  activeConnectionId: null,
  loading: false,

  loadConnections: async () => {
    set({ loading: true });
    try {
      const connections = await invoke<ConnectionConfig[]>("list_connections");
      const activeConnectionId = await invoke<string | null>("get_active_connection");
      set({ connections, activeConnectionId });
    } finally {
      set({ loading: false });
    }
  },

  createConnection: async (config) => {
    const created = await invoke<ConnectionConfig>("create_connection", { config });
    await get().loadConnections();
    return created;
  },

  updateConnection: async (config) => {
    await invoke<ConnectionConfig>("update_connection", { config });
    await get().loadConnections();
  },

  deleteConnection: async (id) => {
    await invoke<void>("delete_connection", { id });
    await get().loadConnections();
  },

  testConnection: async (config) => {
    return invoke<string>("test_connection", { config });
  },

  connect: async (id) => {
    await invoke<void>("connect", { id });
    set({ activeConnectionId: id });
  },

  disconnect: async (id) => {
    await invoke<void>("disconnect", { id });
    const state = get();
    if (state.activeConnectionId === id) {
      set({ activeConnectionId: null });
    }
  },
}));
