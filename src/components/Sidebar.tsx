import { useEffect, useState, useCallback } from "react";
import {
  Database,
  ChevronLeft,
  ChevronRight,
  Plus,
  Plug,
  Unplug,
  Pencil,
  Trash2,
  Loader2,
  TableProperties,
  Clock,
  Bookmark,
  BookOpen,
} from "lucide-react";
import { cn } from "../lib/utils";
import { useConnectionStore } from "../stores/connectionStore";
import { DRIVER_LABELS } from "../types/connection";
import type { ConnectionConfig } from "../types/connection";
import { useEditorStore } from "../stores/editorStore";
import ConnectionForm from "./ConnectionForm";
import SchemaTree from "./SchemaTree";
import QueryHistoryPanel from "./QueryHistoryPanel";
import SavedQueriesPanel from "./SavedQueriesPanel";
import MetadataPanel from "./MetadataPanel";

type BottomTab = "schema" | "history" | "saved" | "metadata";

interface SidebarProps {
  collapsed: boolean;
  onToggle: () => void;
  externalFormOpen?: boolean;
  onExternalFormClose?: () => void;
}

const DRIVER_ICONS: Record<string, string> = {
  postgres: "PG",
  mysql: "My",
  sqlite: "SL",
  mssql: "MS",
};

export default function Sidebar({ collapsed, onToggle, externalFormOpen, onExternalFormClose }: SidebarProps) {
  const { connections, activeConnectionId, connectedIds, loading, loadConnections, connect, disconnect, deleteConnection } =
    useConnectionStore();
  const [formOpen, setFormOpen] = useState(false);
  const [editingConn, setEditingConn] = useState<ConnectionConfig | undefined>();
  const [connectingId, setConnectingId] = useState<string | null>(null);
  const [bottomTab, setBottomTab] = useState<BottomTab>("schema");

  const handleSaveFromHistory = useCallback((sql: string) => {
    // Load the query into the editor, then switch to saved tab so user can click "+"
    const state = useEditorStore.getState();
    const tab = state.getActiveTab();
    if (tab) state.setContent(tab.id, sql);
    setBottomTab("saved");
  }, []);

  // Open form from external trigger (e.g. keyboard shortcut)
  useEffect(() => {
    if (externalFormOpen) {
      setEditingConn(undefined);
      setFormOpen(true);
      onExternalFormClose?.();
    }
  }, [externalFormOpen, onExternalFormClose]);

  useEffect(() => {
    loadConnections();
  }, [loadConnections]);

  const handleConnect = async (id: string) => {
    setConnectingId(id);
    try {
      if (connectedIds.has(id)) {
        await disconnect(id);
      } else {
        await connect(id);
      }
    } catch (e) {
      console.error("Connection error:", e);
    } finally {
      setConnectingId(null);
    }
  };

  // Click connection name/row to set it as active (without connecting/disconnecting)
  const handleSetActive = (id: string) => {
    if (connectedIds.has(id)) {
      connect(id); // Already connected, just switches active pointer
    }
  };

  const handleDelete = async (id: string) => {
    try {
      await deleteConnection(id);
    } catch (e) {
      console.error("Delete error:", e);
    }
  };

  const openNew = () => {
    setEditingConn(undefined);
    setFormOpen(true);
  };

  const openEdit = (conn: ConnectionConfig) => {
    setEditingConn(conn);
    setFormOpen(true);
  };

  return (
    <>
      <aside
        className={cn(
          "flex flex-col border-r border-border bg-muted/50 transition-all duration-200",
          collapsed ? "w-12" : "w-60",
        )}
      >
        {/* Header */}
        <div className="flex h-10 items-center justify-between border-b border-border px-2">
          {!collapsed && (
            <span className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
              Connections
            </span>
          )}
          <button
            onClick={onToggle}
            className="rounded p-1 text-muted-foreground hover:bg-accent hover:text-accent-foreground"
            title={collapsed ? "Expand sidebar" : "Collapse sidebar"}
          >
            {collapsed ? <ChevronRight size={16} /> : <ChevronLeft size={16} />}
          </button>
        </div>

        {/* Connection list */}
        <div className="shrink-0 overflow-y-auto p-2" style={{ maxHeight: "40%" }}>
          {collapsed ? (
            <div className="flex flex-col items-center gap-2 pt-2">
              <button
                onClick={openNew}
                className="rounded p-1.5 text-muted-foreground hover:bg-accent hover:text-accent-foreground"
                title="Add connection"
              >
                <Plus size={16} />
              </button>
              {connections.map((conn) => {
                const isConn = connectedIds.has(conn.id);
                return (
                  <button
                    key={conn.id}
                    onClick={() => isConn ? handleSetActive(conn.id) : handleConnect(conn.id)}
                    className={cn(
                      "rounded p-1.5 text-[10px] font-bold",
                      activeConnectionId === conn.id
                        ? "bg-primary/15 text-primary"
                        : isConn
                          ? "bg-success/10 text-success"
                          : "text-muted-foreground hover:bg-accent hover:text-accent-foreground",
                    )}
                    title={`${conn.name} (${DRIVER_LABELS[conn.driver]})${isConn ? " — connected" : ""}`}
                    style={conn.color ? { borderLeft: `2px solid ${conn.color}` } : undefined}
                  >
                    {DRIVER_ICONS[conn.driver] ?? "DB"}
                  </button>
                );
              })}
            </div>
          ) : (
            <div className="space-y-1">
              <button
                onClick={openNew}
                className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-sm text-muted-foreground hover:bg-accent hover:text-accent-foreground"
              >
                <Plus size={14} />
                <span>New Connection</span>
              </button>

              {loading && (
                <div className="flex items-center justify-center py-4">
                  <Loader2 size={16} className="animate-spin text-muted-foreground" />
                </div>
              )}

              {!loading && connections.length === 0 && (
                <div className="py-8 text-center text-xs text-muted-foreground">
                  <Database size={24} className="mx-auto mb-2 opacity-30" />
                  No connections yet
                </div>
              )}

              {connections.map((conn) => {
                const isActive = activeConnectionId === conn.id;
                const isConnectedConn = connectedIds.has(conn.id);
                const isConnecting = connectingId === conn.id;

                return (
                  <div
                    key={conn.id}
                    onClick={() => handleSetActive(conn.id)}
                    className={cn(
                      "group flex items-center gap-2 rounded-md px-2 py-1.5 text-sm transition-colors cursor-pointer",
                      isActive
                        ? "bg-primary/10 text-foreground"
                        : isConnectedConn
                          ? "bg-success/5 text-foreground hover:bg-success/10"
                          : "text-muted-foreground hover:bg-accent hover:text-accent-foreground",
                    )}
                    style={conn.color ? { borderLeft: `3px solid ${conn.color}` } : undefined}
                  >
                    {/* Status dot */}
                    <span
                      className={cn(
                        "h-2 w-2 shrink-0 rounded-full",
                        isConnectedConn ? "bg-success" : "bg-muted-foreground/30",
                      )}
                    />

                    {/* Name + driver */}
                    <div className="min-w-0 flex-1">
                      <div className="truncate text-xs font-medium">{conn.name}</div>
                      <div className="truncate text-[10px] opacity-60">
                        {DRIVER_LABELS[conn.driver]}
                        {conn.driver !== "sqlite" && conn.host ? ` — ${conn.host}` : ""}
                      </div>
                    </div>

                    {/* Actions (show on hover) */}
                    <div className="flex shrink-0 gap-0.5 opacity-0 transition-opacity group-hover:opacity-100">
                      <button
                        onClick={(e) => {
                          e.stopPropagation();
                          handleConnect(conn.id);
                        }}
                        className="rounded p-1 hover:bg-background/80"
                        title={isConnectedConn ? "Disconnect" : "Connect"}
                      >
                        {isConnecting ? (
                          <Loader2 size={12} className="animate-spin" />
                        ) : isConnectedConn ? (
                          <Unplug size={12} />
                        ) : (
                          <Plug size={12} />
                        )}
                      </button>
                      <button
                        onClick={(e) => {
                          e.stopPropagation();
                          openEdit(conn);
                        }}
                        className="rounded p-1 hover:bg-background/80"
                        title="Edit"
                      >
                        <Pencil size={12} />
                      </button>
                      <button
                        onClick={(e) => {
                          e.stopPropagation();
                          handleDelete(conn.id);
                        }}
                        className="rounded p-1 text-destructive hover:bg-background/80"
                        title="Delete"
                      >
                        <Trash2 size={12} />
                      </button>
                    </div>
                  </div>
                );
              })}
            </div>
          )}
        </div>

        {/* Bottom panel: Schema / History / Saved */}
        {!collapsed && (
          <div className="flex flex-1 flex-col overflow-hidden border-t border-border">
            {/* Tab bar */}
            <div className="flex shrink-0 border-b border-border">
              {([
                { id: "schema" as const, icon: TableProperties, label: "Schema" },
                { id: "history" as const, icon: Clock, label: "History" },
                { id: "saved" as const, icon: Bookmark, label: "Saved" },
                { id: "metadata" as const, icon: BookOpen, label: "Metadata" },
              ]).map(({ id, icon: Icon, label }) => (
                <button
                  key={id}
                  onClick={() => setBottomTab(id)}
                  className={cn(
                    "flex flex-1 items-center justify-center gap-1 py-1.5 text-[10px] font-medium transition-colors",
                    bottomTab === id
                      ? "border-b-2 border-primary text-primary"
                      : "text-muted-foreground hover:text-foreground",
                  )}
                >
                  <Icon size={11} />
                  {label}
                </button>
              ))}
            </div>
            <div className="flex-1 overflow-hidden">
              {bottomTab === "schema" && <SchemaTree />}
              {bottomTab === "history" && (
                <QueryHistoryPanel onSaveQuery={handleSaveFromHistory} />
              )}
              {bottomTab === "saved" && <SavedQueriesPanel />}
              {bottomTab === "metadata" && <MetadataPanel />}
            </div>
          </div>
        )}
      </aside>

      {/* Connection form modal */}
      {formOpen && (
        <ConnectionForm
          initial={editingConn}
          onClose={() => setFormOpen(false)}
        />
      )}
    </>
  );
}
