import { useState, useRef, useEffect } from "react";
import {
  Trash2,
  ChevronDown,
  ChevronRight,
  CheckCircle2,
  XCircle,
  Clock,
  Zap,
  ScrollText,
} from "lucide-react";
import { cn } from "../lib/utils";
import { useMetadataStore } from "../stores/metadataStore";
import type { MetadataLogEntry } from "../types/metadata";

export default function MetadataLogPanel() {
  const { logEntries, clearLog } = useMetadataStore();
  const bottomRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [logEntries.length]);

  return (
    <div className="flex flex-col text-xs h-full">
      {/* Header */}
      <div className="flex items-center justify-between px-2 pb-1 pt-2 shrink-0">
        <span className="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
          Generation Log
        </span>
        {logEntries.length > 0 && (
          <button
            onClick={clearLog}
            className="rounded p-0.5 text-muted-foreground hover:bg-accent hover:text-destructive"
            title="Clear log"
          >
            <Trash2 size={12} />
          </button>
        )}
      </div>

      {logEntries.length === 0 ? (
        <div className="px-2 py-6 text-center text-[11px] text-muted-foreground">
          <ScrollText size={24} className="mx-auto mb-2 opacity-30" />
          <p>No generation activity yet</p>
          <p className="mt-1 text-[10px] opacity-70">
            LLM calls will appear here when you generate metadata
          </p>
        </div>
      ) : (
        <div className="flex-1 overflow-y-auto px-1">
          {logEntries.map((entry) => (
            <LogEntryRow key={entry.id} entry={entry} />
          ))}
          <div ref={bottomRef} />
        </div>
      )}
    </div>
  );
}

function LogEntryRow({ entry }: { entry: MetadataLogEntry }) {
  const [expanded, setExpanded] = useState(false);
  const isError = entry.status === "error";
  const time = new Date(entry.timestamp).toLocaleTimeString();

  return (
    <div className={cn(
      "mb-1 rounded border text-[11px]",
      isError ? "border-destructive/30 bg-destructive/5" : "border-border",
    )}>
      {/* Summary row */}
      <button
        onClick={() => setExpanded(!expanded)}
        className="flex w-full items-center gap-1.5 px-2 py-1.5 text-left hover:bg-accent/50 rounded"
      >
        {expanded ? <ChevronDown size={10} /> : <ChevronRight size={10} />}
        {isError ? (
          <XCircle size={11} className="shrink-0 text-destructive" />
        ) : (
          <CheckCircle2 size={11} className="shrink-0 text-emerald-500" />
        )}
        <span className="flex-1 min-w-0 truncate font-medium text-foreground">
          {entry.objectNames.join(", ")}
        </span>
        <span className="shrink-0 flex items-center gap-1 text-[9px] text-muted-foreground">
          <Clock size={8} />
          {formatDuration(entry.durationMs)}
        </span>
      </button>

      {/* Expanded details */}
      {expanded && (
        <div className="border-t border-border px-2 py-2 space-y-2">
          {/* Stats row */}
          <div className="flex flex-wrap gap-3 text-[10px] text-muted-foreground">
            <span className="flex items-center gap-1">
              <Clock size={9} />
              {time}
            </span>
            <span className="rounded bg-muted px-1 py-0.5 text-[9px]">
              {entry.flow === "generate_batch_metadata" ? "batch" : "single"}
            </span>
            {entry.totalTokens != null && (
              <span className="flex items-center gap-1">
                <Zap size={9} />
                {entry.totalTokens.toLocaleString()} tokens
              </span>
            )}
            {entry.promptTokens != null && entry.completionTokens != null && (
              <span className="text-[9px] opacity-60">
                ({entry.promptTokens.toLocaleString()} in / {entry.completionTokens.toLocaleString()} out)
              </span>
            )}
          </div>

          {/* Error message */}
          {isError && entry.error && (
            <div className="rounded bg-destructive/10 px-2 py-1.5 text-[10px] text-destructive break-all">
              {entry.error}
            </div>
          )}

          {/* Prompt */}
          <ExpandableSection label="Prompt" content={entry.prompt} />

          {/* Response */}
          {entry.response && (
            <ExpandableSection label="Response" content={entry.response} />
          )}
        </div>
      )}
    </div>
  );
}

function ExpandableSection({ label, content }: { label: string; content: string }) {
  const [open, setOpen] = useState(false);
  const previewLen = 120;
  const isLong = content.length > previewLen;

  return (
    <div>
      <button
        onClick={() => setOpen(!open)}
        className="flex items-center gap-1 text-[10px] font-medium text-muted-foreground hover:text-foreground"
      >
        {open ? <ChevronDown size={9} /> : <ChevronRight size={9} />}
        {label}
        <span className="font-normal opacity-50">({content.length} chars)</span>
      </button>
      {open && (
        <pre className="mt-1 max-h-48 overflow-auto rounded bg-muted/50 px-2 py-1.5 text-[10px] font-mono leading-relaxed text-foreground/80 whitespace-pre-wrap break-all">
          {content}
        </pre>
      )}
      {!open && isLong && (
        <pre className="mt-0.5 rounded bg-muted/30 px-2 py-1 text-[9px] font-mono text-muted-foreground truncate">
          {content.slice(0, previewLen)}...
        </pre>
      )}
      {!open && !isLong && (
        <pre className="mt-0.5 rounded bg-muted/30 px-2 py-1 text-[9px] font-mono text-muted-foreground">
          {content}
        </pre>
      )}
    </div>
  );
}

function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  const s = ms / 1000;
  if (s < 60) return `${s.toFixed(1)}s`;
  const m = Math.floor(s / 60);
  const rem = Math.round(s % 60);
  return `${m}m ${rem}s`;
}
