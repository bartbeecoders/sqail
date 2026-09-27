import { useEffect, useState, useMemo, useCallback } from "react";
import {
  Search,
  X,
  Trash2,
  CheckCircle2,
  XCircle,
  Clock,
  Play,
  Bookmark,
} from "lucide-react";
import { cn } from "../lib/utils";
import { useQueryHistoryStore } from "../stores/queryHistoryStore";
import { useEditorStore } from "../stores/editorStore";
import type { QueryHistoryEntry } from "../types/queryHistory";

interface QueryHistoryPanelProps {
  onSaveQuery?: (sql: string) => void;
}

export default function QueryHistoryPanel({ onSaveQuery }: QueryHistoryPanelProps) {
  const { history, loadHistory, deleteHistoryEntry, clearHistory } =
    useQueryHistoryStore();
  const [search, setSearch] = useState("");
  const [statusFilter, setStatusFilter] = useState<"all" | "success" | "error">("all");

  useEffect(() => {
    loadHistory();
  }, [loadHistory]);

  const filtered = useMemo(() => {
    let items = [...history].reverse(); // newest first
    if (statusFilter === "success") items = items.filter((e) => e.success);
    if (statusFilter === "error") items = items.filter((e) => !e.success);
    if (search) {
      const lower = search.toLowerCase();
      items = items.filter(
        (e) =>
          e.query.toLowerCase().includes(lower) ||
          e.connectionName?.toLowerCase().includes(lower),
      );
    }
    return items;
  }, [history, search, statusFilter]);

  const loadIntoEditor = useCallback((entry: QueryHistoryEntry) => {
    const state = useEditorStore.getState();
    const tab = state.getActiveTab();
    if (tab) {
      state.setContent(tab.id, entry.query);
    }
  }, []);

  const formatTime = (iso: string) => {
    const d = new Date(iso);
    const now = new Date();
    const diffMs = now.getTime() - d.getTime();
    const diffMins = Math.floor(diffMs / 60000);
    if (diffMins < 1) return "just now";
    if (diffMins < 60) return `${diffMins}m ago`;
    const diffHours = Math.floor(diffMins / 60);
    if (diffHours < 24) return `${diffHours}h ago`;
    const diffDays = Math.floor(diffHours / 24);
    if (diffDays < 7) return `${diffDays}d ago`;
    return d.toLocaleDateString();
  };

  return (
    <div className="flex h-full flex-col">
      {/* Search + filter bar */}
      <div className="flex items-center gap-1 border-b border-border px-2 py-1.5">
        <div className="relative flex-1">
          <Search
            size={10}
            className="absolute left-1.5 top-1/2 -translate-y-1/2 text-muted-foreground"
          />
          <input
            type="text"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder="Search history..."
            className="input h-6 w-full pl-5 pr-5 text-[10px]"
          />
          {search && (
            <button
              onClick={() => setSearch("")}
              className="absolute right-1 top-1/2 -translate-y-1/2 rounded p-0.5 text-muted-foreground hover:text-foreground"
            >
              <X size={8} />
            </button>
          )}
        </div>
        <div className="flex gap-0.5">
          {(["all", "success", "error"] as const).map((s) => (
            <button
              key={s}
              onClick={() => setStatusFilter(s)}
              className={cn(
                "rounded px-1.5 py-0.5 text-[9px] font-medium",
                statusFilter === s
                  ? "bg-primary/10 text-primary"
                  : "text-muted-foreground hover:bg-accent",
              )}
            >
              {s === "all" ? "All" : s === "success" ? "OK" : "Err"}
            </button>
          ))}
        </div>
      </div>

      {/* History list */}
      <div className="flex-1 overflow-y-auto">
        {filtered.length === 0 && (
          <div className="py-8 text-center text-xs text-muted-foreground">
            <Clock size={20} className="mx-auto mb-2 opacity-30" />
            {history.length === 0 ? "No queries executed yet" : "No matching queries"}
          </div>
        )}

        {filtered.map((entry) => (
          <div
            key={entry.id}
            className="group border-b border-border px-2 py-1.5 hover:bg-accent/50"
          >
            {/* Top row: status + time + actions */}
            <div className="flex items-center gap-1.5 text-[9px] text-muted-foreground">
              {entry.success ? (
                <CheckCircle2 size={10} className="shrink-0 text-success" />
              ) : (
                <XCircle size={10} className="shrink-0 text-destructive" />
              )}
              <span className="truncate">{entry.connectionName ?? "—"}</span>
              <span className="ml-auto shrink-0">{formatTime(entry.timestamp)}</span>
              <span className="shrink-0 tabular-nums">{entry.executionTimeMs}ms</span>
              {entry.rowCount != null && (
                <span className="shrink-0 tabular-nums">{entry.rowCount} rows</span>
              )}
            </div>

            {/* Query preview */}
            <div
              onClick={() => loadIntoEditor(entry)}
              className="mt-0.5 cursor-pointer truncate font-mono text-[10px] leading-tight text-foreground/80 hover:text-foreground"
              title="Click to load into editor"
            >
              {entry.query.slice(0, 200)}
            </div>

            {/* Error message if any */}
            {entry.errorMessage && (
              <div className="mt-0.5 truncate text-[9px] text-destructive">
                {entry.errorMessage}
              </div>
            )}

            {/* Hover actions */}
            <div className="mt-0.5 flex gap-1 opacity-0 transition-opacity group-hover:opacity-100">
              <button
                onClick={() => loadIntoEditor(entry)}
                className="flex items-center gap-0.5 rounded px-1 py-0.5 text-[9px] text-muted-foreground hover:bg-accent hover:text-foreground"
                title="Load into editor"
              >
                <Play size={8} />
                Load
              </button>
              {onSaveQuery && (
                <button
                  onClick={() => onSaveQuery(entry.query)}
                  className="flex items-center gap-0.5 rounded px-1 py-0.5 text-[9px] text-muted-foreground hover:bg-accent hover:text-foreground"
                  title="Save query"
                >
                  <Bookmark size={8} />
                  Save
                </button>
              )}
              <button
                onClick={() => deleteHistoryEntry(entry.id)}
                className="flex items-center gap-0.5 rounded px-1 py-0.5 text-[9px] text-destructive/70 hover:bg-accent hover:text-destructive"
                title="Delete"
              >
                <Trash2 size={8} />
              </button>
            </div>
          </div>
        ))}
      </div>

      {/* Footer */}
      {history.length > 0 && (
        <div className="flex items-center justify-between border-t border-border px-2 py-1">
          <span className="text-[9px] text-muted-foreground">
            {history.length} {history.length === 1 ? "query" : "queries"}
          </span>
          <button
            onClick={clearHistory}
            className="text-[9px] text-destructive/70 hover:text-destructive"
          >
            Clear all
          </button>
        </div>
      )}
    </div>
  );
}
