import { useState, useRef, useEffect } from "react";
import { Table2, Loader2, AlertCircle, CheckCircle2, Download } from "lucide-react";
import { cn } from "../lib/utils";
import { useQueryStore } from "../stores/queryStore";
import { exportResults, type ExportFormat } from "../lib/exportResults";
import DataGrid from "./DataGrid";

export default function ResultsPane() {
  const { results, activeResultIndex, totalTimeMs, error, loading, setActiveResultIndex } =
    useQueryStore();

  // Loading state
  if (loading) {
    return (
      <div className="flex flex-1 flex-col">
        <div className="flex flex-1 items-center justify-center text-muted-foreground">
          <Loader2 size={20} className="mr-2 animate-spin" />
          <span className="text-sm">Executing query...</span>
        </div>
      </div>
    );
  }

  // Error only (no results)
  if (error && results.length === 0) {
    return (
      <div className="flex flex-1 flex-col">
        <div className="flex flex-1 items-center justify-center px-8">
          <div className="max-w-lg rounded-md bg-destructive/10 p-4 text-sm text-destructive">
            <div className="mb-1 flex items-center gap-2 font-medium">
              <AlertCircle size={14} />
              Query Error
            </div>
            <p className="break-all text-xs opacity-80">{error}</p>
          </div>
        </div>
        <StatusBar totalTimeMs={totalTimeMs} />
      </div>
    );
  }

  // Empty state
  if (results.length === 0) {
    return (
      <div className="flex flex-1 items-center justify-center bg-muted/20 text-muted-foreground">
        <div className="text-center">
          <Table2 size={28} className="mx-auto mb-2 opacity-30" />
          <p className="text-xs">Run a query to see results</p>
        </div>
      </div>
    );
  }

  const activeResult = results[activeResultIndex];
  const canExport = !activeResult.isMutation && activeResult.columns.length > 0 && activeResult.rows.length > 0;

  return (
    <div className="flex flex-1 flex-col overflow-hidden">
      {/* Multi-result tabs */}
      {results.length > 1 && (
        <div className="flex h-7 items-center gap-px border-b border-border bg-muted/30 px-1">
          {results.map((r, i) => (
            <button
              key={i}
              onClick={() => setActiveResultIndex(i)}
              className={cn(
                "rounded px-2 py-0.5 text-[11px] transition-colors",
                i === activeResultIndex
                  ? "bg-background text-foreground font-medium"
                  : "text-muted-foreground hover:text-foreground",
              )}
            >
              {r.isMutation ? `Statement ${r.statementIndex + 1}` : `Result ${i + 1}`}
            </button>
          ))}
        </div>
      )}

      {/* Result content */}
      {activeResult.isMutation ? (
        <div className="flex flex-1 items-center justify-center text-muted-foreground">
          <div className="text-center">
            <CheckCircle2 size={24} className="mx-auto mb-2 text-success" />
            <p className="text-sm font-medium">
              {activeResult.affectedRows ?? 0} row{activeResult.affectedRows !== 1 ? "s" : ""} affected
            </p>
            <p className="mt-1 text-xs opacity-60">{activeResult.executionTimeMs}ms</p>
          </div>
        </div>
      ) : activeResult.columns.length === 0 ? (
        <div className="flex flex-1 items-center justify-center text-muted-foreground">
          <p className="text-xs">Query returned no columns</p>
        </div>
      ) : (
        <DataGrid columns={activeResult.columns} rows={activeResult.rows} />
      )}

      {/* Error banner (partial — some statements succeeded) */}
      {error && (
        <div className="flex items-center gap-2 border-t border-destructive/30 bg-destructive/10 px-3 py-1.5 text-xs text-destructive">
          <AlertCircle size={12} />
          <span className="truncate">{error}</span>
        </div>
      )}

      {/* Status bar */}
      <StatusBar
        totalTimeMs={totalTimeMs}
        rowCount={activeResult.isMutation ? undefined : activeResult.rowCount}
        affectedRows={activeResult.isMutation ? activeResult.affectedRows ?? undefined : undefined}
        canExport={canExport}
        onExport={
          canExport
            ? (format) => exportResults(format, activeResult.columns, activeResult.rows)
            : undefined
        }
      />
    </div>
  );
}

function StatusBar({
  totalTimeMs,
  rowCount,
  affectedRows,
  canExport,
  onExport,
}: {
  totalTimeMs: number;
  rowCount?: number;
  affectedRows?: number;
  canExport?: boolean;
  onExport?: (format: ExportFormat) => void;
}) {
  return (
    <div className="flex h-6 shrink-0 items-center justify-between border-t border-border bg-muted/40 px-3 text-[11px] text-muted-foreground">
      <div className="flex items-center gap-3">
        {rowCount !== undefined && (
          <span>
            {rowCount.toLocaleString()} row{rowCount !== 1 ? "s" : ""}
          </span>
        )}
        {affectedRows !== undefined && (
          <span>
            {affectedRows.toLocaleString()} affected
          </span>
        )}
      </div>
      <div className="flex items-center gap-3">
        {canExport && onExport && <ExportDropdown onExport={onExport} />}
        {totalTimeMs > 0 && <span>{totalTimeMs}ms</span>}
      </div>
    </div>
  );
}

const EXPORT_OPTIONS: { format: ExportFormat; label: string; desc: string }[] = [
  { format: "csv", label: "CSV", desc: "Comma-separated values" },
  { format: "excel", label: "Excel", desc: "Excel XML spreadsheet (.xls)" },
  { format: "json", label: "JSON", desc: "JSON array of objects" },
  { format: "xml", label: "XML", desc: "XML document" },
];

function ExportDropdown({ onExport }: { onExport: (format: ExportFormat) => void }) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const handler = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [open]);

  return (
    <div ref={ref} className="relative">
      <button
        onClick={() => setOpen((v) => !v)}
        className={cn(
          "flex items-center gap-1 rounded px-1.5 py-0.5 transition-colors",
          open
            ? "bg-primary/10 text-primary"
            : "text-muted-foreground hover:bg-accent hover:text-accent-foreground",
        )}
        title="Export results"
      >
        <Download size={11} />
        <span>Export</span>
      </button>
      {open && (
        <div className="absolute bottom-full right-0 mb-1 min-w-44 rounded-md border border-border bg-background py-1 shadow-lg z-50">
          {EXPORT_OPTIONS.map((opt) => (
            <button
              key={opt.format}
              onClick={() => {
                onExport(opt.format);
                setOpen(false);
              }}
              className="flex w-full flex-col px-3 py-1.5 text-left hover:bg-accent hover:text-accent-foreground"
            >
              <span className="text-xs font-medium">{opt.label}</span>
              <span className="text-[10px] text-muted-foreground">{opt.desc}</span>
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
