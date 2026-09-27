import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import type { ConnectionConfig } from "../types/connection";

interface ConnectionState {
  connections: ConnectionConfig[];
  activeConnectionId: string | null;
  connectedIds: Set<string>;
  loading: boolean;

  loadConnections: () => Promise<void>;
  createConnection: (config: ConnectionConfig) => Promise<ConnectionConfig>;
  updateConnection: (config: ConnectionConfig) => Promise<void>;
  deleteConnection: (id: string) => Promise<void>;
  testConnection: (config: ConnectionConfig) => Promise<string>;
  connect: (id: string) => Promise<void>;
  disconnect: (id: string) => Promise<void>;
  setActive: (id: string) => void;
  isConnected: (id: string) => boolean;
}

export const useConnectionStore = create<ConnectionState>((set, get) => ({
  connections: [],
  activeConnectionId: null,
  connectedIds: new Set(),
  loading: false,

  loadConnections: async () => {
    set({ loading: true });
    try {
      const connections = await invoke<ConnectionConfig[]>("list_connections");
      const activeConnectionId = await invoke<string | null>("get_active_connection");
      // If backend reports an active connection, it's connected
      const connectedIds = new Set(get().connectedIds);
      if (activeConnectionId) connectedIds.add(activeConnectionId);
      set({ connections, activeConnectionId, connectedIds });
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
    const connectedIds = new Set(get().connectedIds);
    connectedIds.delete(id);
    set({ connectedIds });
    await get().loadConnections();
  },

  testConnection: async (config) => {
    return invoke<string>("test_connection", { config });
  },

  connect: async (id) => {
    const { connectedIds } = get();
    if (connectedIds.has(id)) {
      // Already connected — just switch the active pointer
      set({ activeConnectionId: id });
      return;
    }
    await invoke<void>("connect", { id });
    const newConnected = new Set(get().connectedIds);
    newConnected.add(id);
    set({ activeConnectionId: id, connectedIds: newConnected });
  },

  disconnect: async (id) => {
    await invoke<void>("disconnect", { id });
    const newConnected = new Set(get().connectedIds);
    newConnected.delete(id);
    const state = get();
    const newActive = state.activeConnectionId === id ? null : state.activeConnectionId;
    set({ connectedIds: newConnected, activeConnectionId: newActive });
  },

  setActive: (id) => {
    set({ activeConnectionId: id });
  },

  isConnected: (id) => {
    return get().connectedIds.has(id);
  },
}));
