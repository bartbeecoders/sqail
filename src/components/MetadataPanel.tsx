import { useEffect, useState, useRef } from "react";
import {
  BookOpen,
  Loader2,
  Sparkles,
  Trash2,
  Table2,
  Eye,
  Cog,
  ChevronRight,
  ChevronDown,
  AlertCircle,
} from "lucide-react";
import { useMetadataStore } from "../stores/metadataStore";
import { useConnectionStore } from "../stores/connectionStore";
import { useAiStore } from "../stores/aiStore";
import type { ObjectMetadata } from "../types/metadata";
import MetadataDetailModal from "./MetadataDetailModal";

export default function MetadataPanel() {
  const activeConnectionId = useConnectionStore((s) => s.activeConnectionId);
  const { entries, generating, progress, error, loadMetadata, generateAll, deleteAll } =
    useMetadataStore();
  const hasProvider = useAiStore((s) => s.providers.length > 0);
  const [selectedEntry, setSelectedEntry] = useState<ObjectMetadata | null>(null);
  const [expandedTypes, setExpandedTypes] = useState<Set<string>>(new Set(["table", "view"]));
  const prevConnectionId = useRef<string | null>(null);

  useEffect(() => {
    if (activeConnectionId && activeConnectionId !== prevConnectionId.current) {
      prevConnectionId.current = activeConnectionId;
      loadMetadata(activeConnectionId);
    }
  }, [activeConnectionId, loadMetadata]);

  const toggleType = (type: string) => {
    setExpandedTypes((prev) => {
      const next = new Set(prev);
      if (next.has(type)) next.delete(type);
      else next.add(type);
      return next;
    });
  };

  const handleGenerateAll = () => {
    if (!activeConnectionId) return;
    generateAll(activeConnectionId);
  };

  const handleDeleteAll = () => {
    if (!activeConnectionId) return;
    deleteAll(activeConnectionId);
  };

  if (!activeConnectionId) {
    return (
      <div className="px-2 py-4 text-center text-[11px] text-muted-foreground opacity-60">
        Connect to a database to generate metadata
      </div>
    );
  }

  // Group entries by object type
  const grouped: Record<string, ObjectMetadata[]> = {};
  for (const entry of entries) {
    const type = entry.objectType;
    if (!grouped[type]) grouped[type] = [];
    grouped[type].push(entry);
  }

  const typeConfig: Record<string, { label: string; icon: typeof Table2 }> = {
    table: { label: "Tables", icon: Table2 },
    view: { label: "Views", icon: Eye },
    function: { label: "Functions", icon: Cog },
    procedure: { label: "Procedures", icon: Cog },
  };

  const progressPercent =
    progress && progress.total > 0
      ? Math.round((progress.current / progress.total) * 100)
      : 0;

  return (
    <div className="flex flex-col text-xs">
      {/* Header */}
      <div className="flex items-center justify-between px-2 pb-1 pt-2">
        <span className="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
          Metadata
        </span>
        <div className="flex gap-1">
          {entries.length > 0 && (
            <button
              onClick={handleDeleteAll}
              disabled={generating}
              className="rounded p-0.5 text-muted-foreground hover:bg-accent hover:text-destructive disabled:opacity-40"
              title="Delete all metadata"
            >
              <Trash2 size={12} />
            </button>
          )}
        </div>
      </div>

      {/* Error display */}
      {error && (
        <div className="mx-2 mb-1 flex items-start gap-1.5 rounded border border-destructive/30 bg-destructive/10 px-2 py-1.5 text-[11px] text-destructive">
          <AlertCircle size={12} className="mt-0.5 shrink-0" />
          <span className="break-all">{error}</span>
        </div>
      )}

      {/* Provider warning */}
      {!hasProvider && (
        <div className="mx-2 mb-1 rounded border border-amber-500/30 bg-amber-500/10 px-2 py-1.5 text-[11px] text-amber-600 dark:text-amber-400">
          Configure an AI provider in settings to generate metadata
        </div>
      )}

      {/* Generate button */}
      <div className="px-2 pb-2">
        <button
          onClick={handleGenerateAll}
          disabled={generating || !hasProvider}
          className="flex w-full items-center justify-center gap-1.5 rounded-md bg-primary px-3 py-1.5 text-[11px] font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
        >
          {generating ? (
            <Loader2 size={12} className="animate-spin" />
          ) : (
            <Sparkles size={12} />
          )}
          {generating ? "Generating..." : "Generate All Metadata"}
        </button>
      </div>

      {/* Progress bar */}
      {generating && progress && (
        <div className="mx-2 mb-2">
          <div className="mb-1 flex items-center justify-between text-[10px] text-muted-foreground">
            <span className="truncate">{progress.objectName}</span>
            <span>
              {progress.current}/{progress.total}
            </span>
          </div>
          <div className="h-1.5 w-full overflow-hidden rounded-full bg-muted">
            <div
              className="h-full rounded-full bg-primary transition-all duration-300"
              style={{ width: `${progressPercent}%` }}
            />
          </div>
        </div>
      )}

      {/* Object list */}
      {entries.length === 0 && !generating ? (
        <div className="px-2 py-8 text-center text-[11px] text-muted-foreground">
          <BookOpen size={24} className="mx-auto mb-2 opacity-30" />
          No metadata generated yet
        </div>
      ) : (
        <div className="overflow-y-auto px-1">
          {Object.entries(typeConfig).map(([type, { label, icon: Icon }]) => {
            const items = grouped[type];
            if (!items || items.length === 0) return null;
            const isExpanded = expandedTypes.has(type);

            return (
              <div key={type}>
                <button
                  onClick={() => toggleType(type)}
                  className="flex w-full items-center gap-1 rounded px-1 py-0.5 font-medium text-muted-foreground hover:bg-accent hover:text-accent-foreground"
                >
                  {isExpanded ? (
                    <ChevronDown size={12} />
                  ) : (
                    <ChevronRight size={12} />
                  )}
                  <Icon size={12} className="shrink-0 opacity-60" />
                  <span>{label}</span>
                  <span className="ml-auto text-[9px] font-normal opacity-40">
                    {items.length}
                  </span>
                </button>
                {isExpanded && (
                  <div className="ml-3">
                    {items
                      .sort((a, b) => a.objectName.localeCompare(b.objectName))
                      .map((entry) => (
                        <button
                          key={entry.id}
                          onClick={() => setSelectedEntry(entry)}
                          className="flex w-full items-start gap-1.5 rounded px-1 py-1 text-left text-muted-foreground hover:bg-accent hover:text-accent-foreground"
                        >
                          <Icon size={11} className="mt-0.5 shrink-0 opacity-60" />
                          <div className="min-w-0 flex-1">
                            <div className="truncate text-[11px] font-medium">
                              {entry.objectName}
                            </div>
                            <div className="truncate text-[10px] opacity-50">
                              {entry.metadata.description.slice(0, 80)}
                              {entry.metadata.description.length > 80 ? "..." : ""}
                            </div>
                          </div>
                        </button>
                      ))}
                  </div>
                )}
              </div>
            );
          })}
        </div>
      )}

      {/* Detail modal */}
      {selectedEntry && (
        <MetadataDetailModal
          entry={selectedEntry}
          onClose={() => setSelectedEntry(null)}
          connectionId={activeConnectionId}
        />
      )}
    </div>
  );
}
