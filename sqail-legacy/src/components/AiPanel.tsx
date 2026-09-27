import { useState, useEffect } from "react";
import {
  X,
  Settings,
  Sparkles,
  Plus,
  Pencil,
  Trash2,
  Check,
  ChevronDown,
  ChevronRight,
  History,
} from "lucide-react";
import { cn } from "../lib/utils";
import { useAiStore } from "../stores/aiStore";
import type { AiProviderConfig, AiHistoryEntry } from "../types/ai";
import { AI_FLOW_LABELS, AI_PROVIDER_LABELS } from "../types/ai";
import AiProviderForm from "./AiProviderForm";

export default function AiPanel() {
  const {
    providers,
    history,
    loadProviders,
    loadHistory,
    setPanel,
    getDefaultProvider,
    setDefaultProvider,
    openPalette,
  } = useAiStore();

  const [showSettings, setShowSettings] = useState(false);
  const [editingProvider, setEditingProvider] = useState<AiProviderConfig | undefined>();
  const [showProviderForm, setShowProviderForm] = useState(false);
  const [showHistory, setShowHistory] = useState(true);

  useEffect(() => {
    loadProviders();
    loadHistory();
  }, [loadProviders, loadHistory]);

  const handleDeleteProvider = async (id: string) => {
    await useAiStore.getState().deleteProvider(id);
  };

  const handleSetDefault = async (id: string) => {
    await useAiStore.getState().setDefaultProvider(id);
  };

  const handleHistoryClick = (entry: AiHistoryEntry) => {
    useAiStore.setState({
      currentResponse: entry.response,
      currentFlow: entry.flow,
      error: null,
    });
    openPalette({ flow: entry.flow, sql: entry.prompt });
  };

  return (
    <>
      <aside className="flex w-72 flex-col border-l border-border bg-muted/50">
        {/* Header */}
        <div className="flex h-10 items-center justify-between border-b border-border px-3">
          <div className="flex items-center gap-1.5">
            <Sparkles size={14} className="text-primary" />
            <span className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
              AI Settings
            </span>
          </div>
          <div className="flex items-center gap-0.5">
            <button
              onClick={() => setShowSettings(!showSettings)}
              className={cn(
                "rounded p-1 text-muted-foreground hover:bg-accent hover:text-accent-foreground",
                showSettings && "bg-accent text-accent-foreground",
              )}
              title="Manage Providers"
            >
              <Settings size={14} />
            </button>
            <button
              onClick={() => setPanel(false)}
              className="rounded p-1 text-muted-foreground hover:bg-accent hover:text-accent-foreground"
              title="Close"
            >
              <X size={14} />
            </button>
          </div>
        </div>

        {/* Settings panel */}
        {showSettings && (
          <div className="border-b border-border p-3">
            <div className="mb-2 flex items-center justify-between">
              <span className="text-xs font-medium text-muted-foreground">Providers</span>
              <button
                onClick={() => {
                  setEditingProvider(undefined);
                  setShowProviderForm(true);
                }}
                className="rounded p-1 text-muted-foreground hover:bg-accent hover:text-accent-foreground"
                title="Add provider"
              >
                <Plus size={12} />
              </button>
            </div>
            {providers.length === 0 ? (
              <div className="py-2 text-center text-xs text-muted-foreground">
                No providers configured
              </div>
            ) : (
              <div className="space-y-1">
                {providers.map((p) => (
                  <div
                    key={p.id}
                    className="group flex items-center gap-2 rounded-md px-2 py-1 text-xs"
                  >
                    <div className="min-w-0 flex-1">
                      <span className={cn("block truncate text-xs", p.isDefault && "font-medium text-primary")}>
                        {p.name}
                      </span>
                      <span className="block truncate text-[10px] text-muted-foreground">
                        {AI_PROVIDER_LABELS[p.provider]} — {p.model}
                      </span>
                    </div>
                    <div className="flex gap-0.5 opacity-0 transition-opacity group-hover:opacity-100">
                      {!p.isDefault && (
                        <button
                          onClick={() => handleSetDefault(p.id)}
                          className="rounded p-0.5 hover:bg-background/80"
                          title="Set as default"
                        >
                          <Check size={10} />
                        </button>
                      )}
                      <button
                        onClick={() => {
                          setEditingProvider(p);
                          setShowProviderForm(true);
                        }}
                        className="rounded p-0.5 hover:bg-background/80"
                        title="Edit"
                      >
                        <Pencil size={10} />
                      </button>
                      <button
                        onClick={() => handleDeleteProvider(p.id)}
                        className="rounded p-0.5 text-destructive hover:bg-background/80"
                        title="Delete"
                      >
                        <Trash2 size={10} />
                      </button>
                    </div>
                  </div>
                ))}
              </div>
            )}
          </div>
        )}

        {/* Provider selector */}
        {providers.length > 0 && !showSettings && (
          <div className="border-b border-border px-3 py-1.5">
            <select
              value={getDefaultProvider()?.id ?? ""}
              onChange={(e) => setDefaultProvider(e.target.value)}
              className="w-full rounded-md border border-border bg-background px-2 py-1 text-xs text-foreground"
            >
              {providers.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.name} ({AI_PROVIDER_LABELS[p.provider]} — {p.model})
                </option>
              ))}
            </select>
          </div>
        )}

        {/* Tip: open palette */}
        <div className="border-b border-border px-3 py-2 text-center text-[10px] text-muted-foreground">
          Press <kbd className="rounded bg-muted px-1 py-0.5 text-[10px] font-medium">Ctrl+K</kbd> to open the AI command palette
        </div>

        {/* History section */}
        <div className="flex-1 overflow-hidden">
          <button
            onClick={() => setShowHistory(!showHistory)}
            className="flex w-full items-center gap-1.5 border-b border-border px-3 py-2 text-xs font-medium text-muted-foreground hover:bg-accent"
          >
            {showHistory ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
            <History size={12} />
            History ({history.length})
          </button>
          {showHistory && (
            <div className="flex-1 overflow-y-auto px-3 pb-2">
              {history.length === 0 ? (
                <div className="py-4 text-center text-[10px] text-muted-foreground">
                  No history yet
                </div>
              ) : (
                <div className="space-y-1 pt-1">
                  {[...history].reverse().map((entry) => (
                    <button
                      key={entry.id}
                      onClick={() => handleHistoryClick(entry)}
                      className="w-full rounded-md px-2 py-1 text-left text-[10px] text-muted-foreground hover:bg-accent"
                    >
                      <span className="font-medium">{AI_FLOW_LABELS[entry.flow]}</span>
                      <span className="ml-1 opacity-60">
                        {entry.prompt.slice(0, 40)}
                        {entry.prompt.length > 40 ? "..." : ""}
                      </span>
                    </button>
                  ))}
                </div>
              )}
              {history.length > 0 && (
                <button
                  onClick={() => useAiStore.getState().clearHistory()}
                  className="mt-1 text-[10px] text-destructive hover:underline"
                >
                  Clear history
                </button>
              )}
            </div>
          )}
        </div>
      </aside>

      {/* Provider form modal */}
      {showProviderForm && (
        <AiProviderForm
          initial={editingProvider}
          onClose={() => {
            setShowProviderForm(false);
            setEditingProvider(undefined);
          }}
        />
      )}
    </>
  );
}
