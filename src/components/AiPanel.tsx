import { useState, useEffect, useRef, useCallback } from "react";
import {
  X,
  Settings,
  Sparkles,
  Send,
  Loader2,
  Plus,
  Pencil,
  Trash2,
  Check,
  ChevronDown,
  ChevronRight,
  Code,
  History,
  AlertCircle,
  ClipboardCopy,
} from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { cn } from "../lib/utils";
import { useAiStore } from "../stores/aiStore";
import { useEditorStore } from "../stores/editorStore";
import { useConnectionStore } from "../stores/connectionStore";
import { buildSchemaContext } from "../lib/schemaContext";
import type { AiFlow, AiProviderConfig, AiHistoryEntry } from "../types/ai";
import { AI_FLOW_LABELS, AI_PROVIDER_LABELS } from "../types/ai";
import AiProviderForm from "./AiProviderForm";

const FLOW_OPTIONS: { flow: AiFlow; icon: React.ReactNode }[] = [
  { flow: "generate_sql", icon: <Code size={12} /> },
  { flow: "explain", icon: <Sparkles size={12} /> },
  { flow: "optimize", icon: <Sparkles size={12} /> },
  { flow: "document", icon: <Sparkles size={12} /> },
];

export default function AiPanel() {
  const {
    providers,
    streaming,
    currentResponse,
    currentFlow,
    error,
    history,
    loadProviders,
    loadHistory,
    generateSql,
    explainQuery,
    optimizeQuery,
    generateDocs,
    setPanel,
  } = useAiStore();

  const [selectedFlow, setSelectedFlow] = useState<AiFlow>("generate_sql");
  const [prompt, setPrompt] = useState("");
  const [showSettings, setShowSettings] = useState(false);
  const [editingProvider, setEditingProvider] = useState<AiProviderConfig | undefined>();
  const [showProviderForm, setShowProviderForm] = useState(false);
  const [showHistory, setShowHistory] = useState(false);
  const responseRef = useRef<HTMLDivElement>(null);

  const activeConnectionId = useConnectionStore((s) => s.activeConnectionId);
  const connections = useConnectionStore((s) => s.connections);
  const activeConn = connections.find((c) => c.id === activeConnectionId);
  const driver = activeConn?.driver ?? "";

  useEffect(() => {
    loadProviders();
    loadHistory();
  }, [loadProviders, loadHistory]);

  // Auto-scroll response area
  useEffect(() => {
    if (responseRef.current) {
      responseRef.current.scrollTop = responseRef.current.scrollHeight;
    }
  }, [currentResponse]);

  const getCurrentSql = useCallback(() => {
    const tab = useEditorStore.getState().getActiveTab();
    return tab?.content?.trim() ?? "";
  }, []);

  const handleSubmit = async () => {
    if (streaming) return;
    const schemaContext = buildSchemaContext();

    switch (selectedFlow) {
      case "generate_sql": {
        if (!prompt.trim()) return;
        await generateSql(prompt, schemaContext, driver);
        break;
      }
      case "explain": {
        const sql = getCurrentSql();
        if (!sql) return;
        await explainQuery(sql, schemaContext, driver);
        break;
      }
      case "optimize": {
        const sql = getCurrentSql();
        if (!sql) return;
        await optimizeQuery(sql, schemaContext, driver);
        break;
      }
      case "document": {
        if (!schemaContext) return;
        await generateDocs(schemaContext, driver);
        break;
      }
    }
  };

  const handleInsertToEditor = () => {
    if (!currentResponse) return;
    const state = useEditorStore.getState();
    const tab = state.getActiveTab();
    if (tab) {
      state.setContent(tab.id, currentResponse);
    }
  };

  const handleSaveToHistory = async () => {
    if (!currentResponse || !currentFlow) return;
    const entry: AiHistoryEntry = {
      id: crypto.randomUUID(),
      timestamp: new Date().toISOString(),
      flow: currentFlow,
      prompt: selectedFlow === "generate_sql" ? prompt : getCurrentSql(),
      response: currentResponse,
      connectionId: activeConnectionId ?? undefined,
    };
    try {
      await invoke("save_ai_history_entry", { entry });
      await useAiStore.getState().loadHistory();
    } catch (e) {
      console.error("Failed to save history:", e);
    }
  };

  // Save to history when stream finishes
  const prevStreamingRef = useRef(streaming);
  useEffect(() => {
    if (prevStreamingRef.current && !streaming && currentResponse && !error) {
      handleSaveToHistory();
    }
    prevStreamingRef.current = streaming;
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [streaming]);

  const handleDeleteProvider = async (id: string) => {
    await useAiStore.getState().deleteProvider(id);
  };

  const handleSetDefault = async (id: string) => {
    await useAiStore.getState().setDefaultProvider(id);
  };

  const needsPromptInput = selectedFlow === "generate_sql";
  const needsSqlInEditor = selectedFlow === "explain" || selectedFlow === "optimize";
  const hasProvider = providers.length > 0;

  return (
    <>
      <aside className="flex w-80 flex-col border-l border-border bg-muted/50">
        {/* Header */}
        <div className="flex h-10 items-center justify-between border-b border-border px-3">
          <div className="flex items-center gap-1.5">
            <Sparkles size={14} className="text-primary" />
            <span className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
              AI Assistant
            </span>
          </div>
          <div className="flex items-center gap-0.5">
            <button
              onClick={() => setShowSettings(!showSettings)}
              className={cn(
                "rounded p-1 text-muted-foreground hover:bg-accent hover:text-accent-foreground",
                showSettings && "bg-accent text-accent-foreground",
              )}
              title="AI Settings"
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

        {/* Flow selector */}
        <div className="flex gap-1 border-b border-border p-2">
          {FLOW_OPTIONS.map(({ flow, icon }) => (
            <button
              key={flow}
              onClick={() => setSelectedFlow(flow)}
              className={cn(
                "flex flex-1 items-center justify-center gap-1 rounded-md px-1.5 py-1 text-[10px] font-medium transition-colors",
                selectedFlow === flow
                  ? "bg-primary text-primary-foreground"
                  : "text-muted-foreground hover:bg-accent hover:text-accent-foreground",
              )}
            >
              {icon}
              <span className="hidden xl:inline">{AI_FLOW_LABELS[flow].split(" ")[0]}</span>
            </button>
          ))}
        </div>

        {/* Input area */}
        <div className="border-b border-border p-3">
          {!hasProvider && (
            <div className="mb-2 flex items-start gap-2 rounded-md bg-amber-500/10 p-2 text-xs text-amber-600 dark:text-amber-400">
              <AlertCircle size={14} className="mt-0.5 shrink-0" />
              <span>No AI provider configured. Click the gear icon to add one.</span>
            </div>
          )}

          {needsPromptInput && (
            <textarea
              value={prompt}
              onChange={(e) => setPrompt(e.target.value)}
              placeholder="Describe the query you want..."
              className="input min-h-[60px] resize-y text-xs"
              onKeyDown={(e) => {
                if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
                  e.preventDefault();
                  handleSubmit();
                }
              }}
            />
          )}

          {needsSqlInEditor && (
            <div className="rounded-md bg-muted p-2 text-xs text-muted-foreground">
              Will use the current SQL from the editor
            </div>
          )}

          {selectedFlow === "document" && (
            <div className="rounded-md bg-muted p-2 text-xs text-muted-foreground">
              Will generate docs for the loaded schema
            </div>
          )}

          <button
            onClick={handleSubmit}
            disabled={
              streaming ||
              !hasProvider ||
              (needsPromptInput && !prompt.trim()) ||
              (needsSqlInEditor && !getCurrentSql())
            }
            className="btn-primary mt-2 flex w-full items-center justify-center gap-1.5 text-xs"
          >
            {streaming ? (
              <>
                <Loader2 size={12} className="animate-spin" />
                Generating...
              </>
            ) : (
              <>
                <Send size={12} />
                {AI_FLOW_LABELS[selectedFlow]}
              </>
            )}
          </button>
        </div>

        {/* Response area */}
        <div ref={responseRef} className="flex-1 overflow-y-auto p-3">
          {error && (
            <div className="mb-2 rounded-md bg-destructive/10 p-2 text-xs text-destructive">
              {error}
            </div>
          )}

          {currentResponse && (
            <div className="space-y-2">
              <pre className="whitespace-pre-wrap text-xs leading-relaxed text-foreground">
                {currentResponse}
              </pre>
              {!streaming && (
                <div className="flex gap-1">
                  {currentFlow === "generate_sql" && (
                    <button
                      onClick={handleInsertToEditor}
                      className="btn-secondary flex items-center gap-1 text-[10px]"
                    >
                      <Code size={10} />
                      Insert to Editor
                    </button>
                  )}
                  {(currentFlow === "optimize") && (
                    <button
                      onClick={handleInsertToEditor}
                      className="btn-secondary flex items-center gap-1 text-[10px]"
                    >
                      <Code size={10} />
                      Insert to Editor
                    </button>
                  )}
                  <button
                    onClick={() => navigator.clipboard.writeText(currentResponse)}
                    className="btn-secondary flex items-center gap-1 text-[10px]"
                  >
                    <ClipboardCopy size={10} />
                    Copy
                  </button>
                </div>
              )}
            </div>
          )}

          {!currentResponse && !error && !streaming && (
            <div className="flex h-full items-center justify-center text-xs text-muted-foreground">
              <div className="text-center">
                <Sparkles size={24} className="mx-auto mb-2 opacity-30" />
                <p>Select a flow and submit a prompt</p>
              </div>
            </div>
          )}
        </div>

        {/* History section */}
        <div className="border-t border-border">
          <button
            onClick={() => setShowHistory(!showHistory)}
            className="flex w-full items-center gap-1.5 px-3 py-2 text-xs font-medium text-muted-foreground hover:bg-accent"
          >
            {showHistory ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
            <History size={12} />
            History ({history.length})
          </button>
          {showHistory && (
            <div className="max-h-40 overflow-y-auto px-3 pb-2">
              {history.length === 0 ? (
                <div className="py-2 text-center text-[10px] text-muted-foreground">
                  No history yet
                </div>
              ) : (
                <div className="space-y-1">
                  {[...history].reverse().map((entry) => (
                    <button
                      key={entry.id}
                      onClick={() => {
                        useAiStore.setState({
                          currentResponse: entry.response,
                          currentFlow: entry.flow,
                          error: null,
                        });
                        setSelectedFlow(entry.flow);
                        if (entry.flow === "generate_sql") {
                          setPrompt(entry.prompt);
                        }
                      }}
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
