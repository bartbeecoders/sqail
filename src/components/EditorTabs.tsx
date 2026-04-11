import { invoke } from "@tauri-apps/api/core";
import { Pin, Plus, X } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { buildSelectStatement } from "../lib/sqlGenerate";
import { cn } from "../lib/utils";
import { useConnectionStore } from "../stores/connectionStore";
import { useEditorStore } from "../stores/editorStore";
import { useSchemaStore } from "../stores/schemaStore";
import type { ColumnInfo } from "../types/schema";
import type { EditorTab } from "../types/editor";

interface TabContextMenu {
  x: number;
  y: number;
  tabId: string;
}

export default function EditorTabs() {
  const {
    tabs,
    activeTabId,
    addTab,
    addTabWithContent,
    closeTab,
    closeOtherTabs,
    closeTabsToRight,
    closeAllTabs,
    setActiveTab,
    renameTab,
    setConnectionId,
    togglePin,
  } = useEditorStore();
  const connections = useConnectionStore((s) => s.connections);
  const globalConnectionId = useConnectionStore((s) => s.activeConnectionId);
  const [editingTabId, setEditingTabId] = useState<string | null>(null);
  const [editValue, setEditValue] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);
  const [contextMenu, setContextMenu] = useState<TabContextMenu | null>(null);
  const contextRef = useRef<HTMLDivElement>(null);
  const [dragOver, setDragOver] = useState(false);

  const commitRename = (id: string) => {
    const trimmed = editValue.trim();
    if (trimmed) renameTab(id, trimmed);
    setEditingTabId(null);
  };

  const closeContextMenu = useCallback(() => setContextMenu(null), []);

  useEffect(() => {
    if (!contextMenu) return;
    const onMouseDown = (e: MouseEvent) => {
      if (contextRef.current && !contextRef.current.contains(e.target as Node)) {
        closeContextMenu();
      }
    };
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") closeContextMenu();
    };
    document.addEventListener("mousedown", onMouseDown);
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("mousedown", onMouseDown);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [contextMenu, closeContextMenu]);

  const handleDragOver = useCallback((e: React.DragEvent) => {
    if (e.dataTransfer.types.includes("application/sqlai-table")) {
      e.preventDefault();
      e.dataTransfer.dropEffect = "copy";
      setDragOver(true);
    }
  }, []);

  const handleDragLeave = useCallback(() => {
    setDragOver(false);
  }, []);

  const handleDrop = useCallback(
    async (e: React.DragEvent) => {
      setDragOver(false);
      const raw = e.dataTransfer.getData("application/sqlai-table");
      if (!raw) return;
      e.preventDefault();

      const { schemaName, tableName } = JSON.parse(raw) as {
        schemaName: string;
        tableName: string;
      };

      const connStore = useConnectionStore.getState();
      const conn = connStore.connections.find(
        (c) => c.id === connStore.activeConnectionId,
      );
      if (!conn) return;

      const schemaStore = useSchemaStore.getState();
      const key = `${schemaName}.${tableName}`;
      let cols = schemaStore.columns[key];
      if (!cols) {
        try {
          cols = await invoke<ColumnInfo[]>("list_columns", {
            connectionId: conn.id,
            schemaName,
            tableName,
          });
          schemaStore.loadColumns(conn.id, schemaName, tableName);
        } catch {
          cols = [];
        }
      }

      const sql = buildSelectStatement(schemaName, tableName, cols, conn.driver);
      addTabWithContent(tableName, sql);
      // Link the new tab to the current connection
      const newTab = useEditorStore.getState().getActiveTab();
      if (newTab && conn.id) {
        setConnectionId(newTab.id, conn.id);
      }
    },
    [addTabWithContent, setConnectionId],
  );

  const tabIdx = contextMenu
    ? tabs.findIndex((t) => t.id === contextMenu.tabId)
    : -1;
  const hasTabsToRight = tabIdx >= 0 && tabIdx < tabs.length - 1;
  const contextTab = tabIdx >= 0 ? tabs[tabIdx] : undefined;

  const pinnedTabs = tabs.filter((t) => t.pinned);
  const unpinnedTabs = tabs.filter((t) => !t.pinned);

  const renderTab = (tab: EditorTab) => {
    const isActive = tab.id === activeTabId;
    const isEditing = editingTabId === tab.id;
    const conn = tab.connectionId
      ? connections.find((c) => c.id === tab.connectionId)
      : undefined;
    return (
      <div
        key={tab.id}
        className={cn(
          "group flex cursor-pointer items-center gap-1 rounded-t-md px-3 py-1 text-xs transition-colors",
          isActive
            ? "bg-background text-foreground border-x border-t border-border"
            : "text-muted-foreground hover:text-foreground hover:bg-muted",
        )}
        onClick={() => setActiveTab(tab.id)}
        onDoubleClick={() => {
          setEditingTabId(tab.id);
          setEditValue(tab.title);
          requestAnimationFrame(() => inputRef.current?.select());
        }}
        onContextMenu={(e) => {
          e.preventDefault();
          setContextMenu({ x: e.clientX, y: e.clientY, tabId: tab.id });
        }}
      >
        {conn && (
          <span
            className="h-2 w-2 shrink-0 rounded-full"
            style={{ backgroundColor: conn.color || "#6366f1" }}
            title={conn.name}
          />
        )}
        {isEditing ? (
          <input
            ref={inputRef}
            value={editValue}
            onChange={(e) => setEditValue(e.target.value)}
            onBlur={() => commitRename(tab.id)}
            onKeyDown={(e) => {
              if (e.key === "Enter") commitRename(tab.id);
              if (e.key === "Escape") setEditingTabId(null);
            }}
            onClick={(e) => e.stopPropagation()}
            onDoubleClick={(e) => e.stopPropagation()}
            className="w-24 bg-transparent outline-none ring-1 ring-border rounded px-1"
            autoFocus
          />
        ) : (
          <span className="max-w-24 truncate">{tab.title}</span>
        )}
        {!isEditing && (
          <button
            onClick={(e) => {
              e.stopPropagation();
              togglePin(tab.id);
            }}
            className={cn(
              "ml-1 rounded p-0.5 transition-opacity hover:bg-muted",
              tab.pinned ? "opacity-100" : "opacity-0 group-hover:opacity-100",
            )}
            title={tab.pinned ? "Unpin tab" : "Pin tab"}
          >
            <Pin
              size={10}
              className={cn("shrink-0", tab.pinned && "fill-current")}
            />
          </button>
        )}
        {tabs.length > 1 && !isEditing && (
          <button
            onClick={(e) => {
              e.stopPropagation();
              closeTab(tab.id);
            }}
            className="rounded p-0.5 opacity-0 transition-opacity group-hover:opacity-100 hover:bg-muted"
            title="Close tab"
          >
            <X size={10} />
          </button>
        )}
      </div>
    );
  };

  return (
    <div
      className={cn(
        "flex flex-col border-b border-border bg-muted/30",
        dragOver && "ring-2 ring-inset ring-primary/40",
      )}
      onDragOver={handleDragOver}
      onDragLeave={handleDragLeave}
      onDrop={handleDrop}
    >
      {pinnedTabs.length > 0 && (
        <div className="flex min-h-8 flex-wrap items-end gap-x-px gap-y-0.5 border-b border-border/60 bg-muted/40 px-1 py-0.5">
          {pinnedTabs.map(renderTab)}
        </div>
      )}
      <div className="flex min-h-8 flex-wrap items-end gap-x-px gap-y-0.5 px-1 py-0.5">
        {unpinnedTabs.map(renderTab)}
        <button
          onClick={() => {
            addTab();
            // Assign current connection to the new tab
            if (globalConnectionId) {
              const newTab = useEditorStore.getState().getActiveTab();
              if (newTab) setConnectionId(newTab.id, globalConnectionId);
            }
          }}
          className="mb-0.5 ml-1 rounded p-1 text-muted-foreground hover:bg-muted hover:text-foreground"
          title="New tab (Ctrl+N)"
        >
          <Plus size={12} />
        </button>
      </div>

      {contextMenu && (
        <div
          ref={contextRef}
          className="fixed z-50 min-w-48 rounded-md border border-border bg-background py-1 shadow-lg text-xs"
          style={{ left: contextMenu.x, top: contextMenu.y }}
        >
          <ContextItem
            label="Close"
            disabled={tabs.length <= 1}
            onClick={() => {
              closeTab(contextMenu.tabId);
              closeContextMenu();
            }}
          />
          <ContextItem
            label="Close other tabs"
            disabled={tabs.length <= 1}
            onClick={() => {
              closeOtherTabs(contextMenu.tabId);
              closeContextMenu();
            }}
          />
          <ContextItem
            label="Close tabs to the right"
            disabled={!hasTabsToRight}
            onClick={() => {
              closeTabsToRight(contextMenu.tabId);
              closeContextMenu();
            }}
          />
          <ContextItem
            label="Close all tabs"
            onClick={() => {
              closeAllTabs();
              closeContextMenu();
            }}
          />
          <div className="my-1 border-t border-border" />
          <ContextItem
            label={contextTab?.pinned ? "Unpin tab" : "Pin tab"}
            onClick={() => {
              togglePin(contextMenu.tabId);
              closeContextMenu();
            }}
          />
          <ContextItem
            label="Rename"
            onClick={() => {
              setEditingTabId(contextMenu.tabId);
              setEditValue(
                tabs.find((t) => t.id === contextMenu.tabId)?.title ?? "",
              );
              closeContextMenu();
              requestAnimationFrame(() => inputRef.current?.select());
            }}
          />
        </div>
      )}
    </div>
  );
}

function ContextItem({
  label,
  onClick,
  disabled,
}: {
  label: string;
  onClick: () => void;
  disabled?: boolean;
}) {
  return (
    <button
      onClick={disabled ? undefined : onClick}
      disabled={disabled}
      className={cn(
        "flex w-full items-center px-3 py-1 text-left text-xs",
        disabled
          ? "text-muted-foreground/40 cursor-default"
          : "hover:bg-accent hover:text-accent-foreground",
      )}
    >
      {label}
    </button>
  );
}
