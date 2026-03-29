import { useEffect, useState } from "react";
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
} from "lucide-react";
import { cn } from "../lib/utils";
import { useConnectionStore } from "../stores/connectionStore";
import { DRIVER_LABELS } from "../types/connection";
import type { ConnectionConfig } from "../types/connection";
import ConnectionForm from "./ConnectionForm";
import SchemaTree from "./SchemaTree";

interface SidebarProps {
  collapsed: boolean;
  onToggle: () => void;
}

const DRIVER_ICONS: Record<string, string> = {
  postgres: "PG",
  mysql: "My",
  sqlite: "SL",
  mssql: "MS",
};

export default function Sidebar({ collapsed, onToggle }: SidebarProps) {
  const { connections, activeConnectionId, loading, loadConnections, connect, disconnect, deleteConnection } =
    useConnectionStore();
  const [formOpen, setFormOpen] = useState(false);
  const [editingConn, setEditingConn] = useState<ConnectionConfig | undefined>();
  const [connectingId, setConnectingId] = useState<string | null>(null);

  useEffect(() => {
    loadConnections();
  }, [loadConnections]);

  const handleConnect = async (id: string) => {
    setConnectingId(id);
    try {
      if (activeConnectionId === id) {
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
              {connections.map((conn) => (
                <button
                  key={conn.id}
                  onClick={() => handleConnect(conn.id)}
                  className={cn(
                    "rounded p-1.5 text-[10px] font-bold",
                    activeConnectionId === conn.id
                      ? "bg-primary/15 text-primary"
                      : "text-muted-foreground hover:bg-accent hover:text-accent-foreground",
                  )}
                  title={`${conn.name} (${DRIVER_LABELS[conn.driver]})`}
                  style={conn.color ? { borderLeft: `2px solid ${conn.color}` } : undefined}
                >
                  {DRIVER_ICONS[conn.driver] ?? "DB"}
                </button>
              ))}
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
                const isConnecting = connectingId === conn.id;

                return (
                  <div
                    key={conn.id}
                    className={cn(
                      "group flex items-center gap-2 rounded-md px-2 py-1.5 text-sm transition-colors",
                      isActive
                        ? "bg-primary/10 text-foreground"
                        : "text-muted-foreground hover:bg-accent hover:text-accent-foreground",
                    )}
                    style={conn.color ? { borderLeft: `3px solid ${conn.color}` } : undefined}
                  >
                    {/* Status dot */}
                    <span
                      className={cn(
                        "h-2 w-2 shrink-0 rounded-full",
                        isActive ? "bg-success" : "bg-muted-foreground/30",
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
                        onClick={() => handleConnect(conn.id)}
                        className="rounded p-1 hover:bg-background/80"
                        title={isActive ? "Disconnect" : "Connect"}
                      >
                        {isConnecting ? (
                          <Loader2 size={12} className="animate-spin" />
                        ) : isActive ? (
                          <Unplug size={12} />
                        ) : (
                          <Plug size={12} />
                        )}
                      </button>
                      <button
                        onClick={() => openEdit(conn)}
                        className="rounded p-1 hover:bg-background/80"
                        title="Edit"
                      >
                        <Pencil size={12} />
                      </button>
                      <button
                        onClick={() => handleDelete(conn.id)}
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

        {/* Schema tree */}
        {!collapsed && (
          <div className="flex-1 overflow-y-auto border-t border-border">
            <SchemaTree />
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
