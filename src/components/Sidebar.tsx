import { useEffect, useState, useCallback, useRef } from "react";
import {
  Database,
  ChevronLeft,
  ChevronRight,
  ChevronDown,
  Plus,
  Plug,
  Unplug,
  Pencil,
  Trash2,
  Copy,
  Loader2,
  TableProperties,
  Clock,
  Bookmark,
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
type BottomTab = "schema" | "history" | "saved";

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

  // Resizable width
  const COLLAPSED_W = 48;
  const MIN_W = 180;
  const MAX_W = 500;
  const DEFAULT_W = 240;
  const [sidebarWidth, setSidebarWidth] = useState<number>(() => {
    try {
      const saved = localStorage.getItem("sqail_sidebar_width");
      if (saved) return Math.max(MIN_W, Math.min(Number(saved), MAX_W));
    } catch { /* ignore */ }
    return DEFAULT_W;
  });
  const dragging = useRef(false);

  useEffect(() => {
    if (!collapsed) {
      localStorage.setItem("sqail_sidebar_width", String(sidebarWidth));
    }
  }, [sidebarWidth, collapsed]);

  const onResizeStart = useCallback((e: React.MouseEvent) => {
    e.preventDefault();
    dragging.current = true;
    const onMove = (ev: MouseEvent) => {
      if (!dragging.current) return;
      const w = Math.max(MIN_W, Math.min(ev.clientX, MAX_W));
      setSidebarWidth(w);
    };
    const onUp = () => {
      dragging.current = false;
      document.removeEventListener("mousemove", onMove);
      document.removeEventListener("mouseup", onUp);
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
    };
    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";
    document.addEventListener("mousemove", onMove);
    document.addEventListener("mouseup", onUp);
  }, []);

  const activeConn = connections.find((c) => c.id === activeConnectionId);

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

  const handleDuplicate = (conn: ConnectionConfig) => {
    const copy = {
      ...conn,
      id: "",
      name: `${conn.name} (copy)`,
    };
    setEditingConn(undefined);
    // Open the form pre-filled with the copy
    setDuplicateConn(copy);
    setFormOpen(true);
  };

  const [duplicateConn, setDuplicateConn] = useState<ConnectionConfig | undefined>();
  const [connDropdownOpen, setConnDropdownOpen] = useState(false);
  const connDropdownRef = useRef<HTMLDivElement>(null);

  // Close dropdown on outside click
  useEffect(() => {
    if (!connDropdownOpen) return;
    const onMouseDown = (e: MouseEvent) => {
      if (connDropdownRef.current && !connDropdownRef.current.contains(e.target as Node)) {
        setConnDropdownOpen(false);
      }
    };
    document.addEventListener("mousedown", onMouseDown);
    return () => document.removeEventListener("mousedown", onMouseDown);
  }, [connDropdownOpen]);

  const openNew = () => {
    setEditingConn(undefined);
    setDuplicateConn(undefined);
    setFormOpen(true);
  };

  const openEdit = (conn: ConnectionConfig) => {
    setEditingConn(conn);
    setDuplicateConn(undefined);
    setFormOpen(true);
  };

  return (
    <>
      <aside
        className="relative flex flex-col border-r border-border bg-muted/50"
        style={{ width: collapsed ? COLLAPSED_W : sidebarWidth }}
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

        {/* Connection selector */}
        {collapsed ? (
          <div className="flex flex-col items-center gap-2 p-2 pt-3 shrink-0">
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
          <div ref={connDropdownRef} className="relative shrink-0 border-b border-border px-2 py-2">
            {/* Dropdown trigger */}
            <button
              onClick={() => setConnDropdownOpen((v) => !v)}
              className="flex w-full items-center gap-2 rounded-md border border-border bg-background px-2 py-1.5 text-left hover:bg-accent/50 transition-colors"
            >
              {activeConn ? (
                <>
                  <span
                    className={cn("h-2 w-2 shrink-0 rounded-full", connectedIds.has(activeConn.id) ? "bg-success" : "bg-muted-foreground/30")}
                    style={activeConn.color ? { backgroundColor: activeConn.color } : undefined}
                  />
                  <div className="min-w-0 flex-1">
                    <div className="truncate text-xs font-medium">{activeConn.name}</div>
                    <div className="truncate text-[9px] text-muted-foreground">
                      {DRIVER_LABELS[activeConn.driver]}
                      {activeConn.driver !== "sqlite" && activeConn.host ? ` — ${activeConn.host}` : ""}
                    </div>
                  </div>
                </>
              ) : (
                <>
                  <Database size={12} className="shrink-0 text-muted-foreground" />
                  <span className="flex-1 text-xs text-muted-foreground">
                    {connections.length === 0 ? "No connections" : "Select connection"}
                  </span>
                </>
              )}
              <ChevronDown size={12} className={cn("shrink-0 text-muted-foreground transition-transform", connDropdownOpen && "rotate-180")} />
            </button>

            {/* Dropdown menu */}
            {connDropdownOpen && (
              <div className="absolute left-2 right-2 top-full z-30 mt-1 max-h-72 overflow-y-auto rounded-md border border-border bg-background py-1 shadow-lg">
                {loading && (
                  <div className="flex items-center justify-center py-3">
                    <Loader2 size={14} className="animate-spin text-muted-foreground" />
                  </div>
                )}

                {connections.map((conn) => {
                  const isActive = activeConnectionId === conn.id;
                  const isConnectedConn = connectedIds.has(conn.id);
                  const isConnecting = connectingId === conn.id;

                  return (
                    <div
                      key={conn.id}
                      onClick={() => {
                        if (isConnectedConn) {
                          handleSetActive(conn.id);
                        } else {
                          handleConnect(conn.id);
                        }
                        setConnDropdownOpen(false);
                      }}
                      className={cn(
                        "group flex items-center gap-2 px-2 py-1.5 cursor-pointer transition-colors",
                        isActive
                          ? "bg-primary/10 text-foreground"
                          : "text-foreground hover:bg-accent",
                      )}
                    >
                      <span
                        className={cn("h-2 w-2 shrink-0 rounded-full", isConnectedConn ? "bg-success" : "bg-muted-foreground/30")}
                        style={conn.color ? { backgroundColor: conn.color } : undefined}
                      />
                      <div className="min-w-0 flex-1">
                        <div className="truncate text-xs font-medium">{conn.name}</div>
                        <div className="truncate text-[9px] text-muted-foreground">
                          {DRIVER_LABELS[conn.driver]}
                          {conn.driver !== "sqlite" && conn.host ? ` — ${conn.host}` : ""}
                        </div>
                      </div>
                      {/* Actions */}
                      <div className="flex shrink-0 gap-0.5 opacity-0 group-hover:opacity-100 transition-opacity">
                        <button
                          onClick={(e) => {
                            e.stopPropagation();
                            handleConnect(conn.id);
                          }}
                          className="rounded p-1 hover:bg-background/80"
                          title={isConnectedConn ? "Disconnect" : "Connect"}
                        >
                          {isConnecting ? (
                            <Loader2 size={11} className="animate-spin" />
                          ) : isConnectedConn ? (
                            <Unplug size={11} />
                          ) : (
                            <Plug size={11} />
                          )}
                        </button>
                        <button
                          onClick={(e) => {
                            e.stopPropagation();
                            openEdit(conn);
                            setConnDropdownOpen(false);
                          }}
                          className="rounded p-1 hover:bg-background/80"
                          title="Edit"
                        >
                          <Pencil size={11} />
                        </button>
                        <button
                          onClick={(e) => {
                            e.stopPropagation();
                            handleDuplicate(conn);
                            setConnDropdownOpen(false);
                          }}
                          className="rounded p-1 hover:bg-background/80"
                          title="Duplicate"
                        >
                          <Copy size={11} />
                        </button>
                        <button
                          onClick={(e) => {
                            e.stopPropagation();
                            handleDelete(conn.id);
                          }}
                          className="rounded p-1 text-destructive hover:bg-background/80"
                          title="Delete"
                        >
                          <Trash2 size={11} />
                        </button>
                      </div>
                    </div>
                  );
                })}

                {/* New connection button */}
                <div className="border-t border-border mt-1 pt-1">
                  <button
                    onClick={() => {
                      openNew();
                      setConnDropdownOpen(false);
                    }}
                    className="flex w-full items-center gap-2 px-2 py-1.5 text-xs text-muted-foreground hover:bg-accent hover:text-foreground transition-colors"
                  >
                    <Plus size={12} />
                    New Connection
                  </button>
                </div>
              </div>
            )}
          </div>
        )}

        {/* Bottom panel: Schema / History / Saved */}
        {!collapsed && (
          <div className="flex flex-1 flex-col overflow-hidden border-t border-border">
            {/* Tab bar */}
            <div className="flex shrink-0 border-b border-border">
              {([
                { id: "schema" as const, icon: TableProperties, label: "Schema" },
                { id: "history" as const, icon: Clock, label: "History" },
                { id: "saved" as const, icon: Bookmark, label: "Saved" },
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
            </div>
          </div>
        )}
        {/* Resize handle */}
        {!collapsed && (
          <div
            onMouseDown={onResizeStart}
            className="absolute right-0 top-0 h-full w-1 cursor-col-resize hover:bg-primary/30 active:bg-primary/50 z-10"
            style={{ marginRight: -1 }}
          />
        )}
      </aside>

      {/* Connection form modal */}
      {formOpen && (
        <ConnectionForm
          initial={editingConn ?? duplicateConn}
          onClose={() => {
            setFormOpen(false);
            setDuplicateConn(undefined);
          }}
        />
      )}
    </>
  );
}
