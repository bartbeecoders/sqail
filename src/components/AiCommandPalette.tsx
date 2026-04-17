import { useState, useEffect, useRef, useCallback, useMemo } from "react";
import {
  Sparkles,
  Loader2,
  Code,
  ClipboardCopy,
  X,
  AlertCircle,
  ChevronDown,
  Columns2,
} from "lucide-react";
import { useAiStore } from "../stores/aiStore";
import { useEditorStore } from "../stores/editorStore";
import { useConnectionStore } from "../stores/connectionStore";
import { useInlineAiStore } from "../stores/inlineAiStore";
import { buildSchemaContext } from "../lib/schemaContext";
import { buildVirtualInlineProvider } from "../lib/inlineProvider";
import {
  AI_FLOW_LABELS,
  AI_PROVIDER_LABELS,
} from "../types/ai";
import type { AiFlow, AiHistoryEntry, AiProviderConfig } from "../types/ai";
import { invoke } from "@tauri-apps/api/core";

const PREFIX_COMMANDS: Record<string, AiFlow> = {
  "/explain": "explain",
  "/optimize": "optimize",
  "/format": "format_sql",
  "/comment": "comment_sql",
  "/docs": "document",
};

/** Flows where the response is SQL code that can be inserted into the editor. */
const INSERTABLE_FLOWS: AiFlow[] = [
  "generate_sql",
  "optimize",
  "format_sql",
  "comment_sql",
  "fix_query",
];

/** Flows that operate on the current editor SQL (no free-text prompt needed). */
const SQL_CONTEXT_FLOWS: AiFlow[] = [
  "explain",
  "optimize",
  "format_sql",
  "comment_sql",
  "fix_query",
];

export default function AiCommandPalette() {
  const {
    paletteOpen,
    paletteFlow,
    paletteSql,
    paletteError,
    streaming,
    currentResponse,
    currentFlow,
    error,
    providers,
    selectedProviderId,
    setSelectedProviderId,
    closePalette,
    promptHistory,
    addToPromptHistory,
    generateSql,
    explainQuery,
    optimizeQuery,
    generateDocs,
    formatSql,
    commentSql,
    fixQuery,
    loadProviders,
    getDefaultProvider,
    setPanel,
    openDiffPreview,
  } = useAiStore();

  const [prompt, setPrompt] = useState("");
  const [historyIndex, setHistoryIndex] = useState(-1);
  const draftRef = useRef("");
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const responseRef = useRef<HTMLDivElement>(null);

  const activeConnectionId = useConnectionStore((s) => s.activeConnectionId);
  const connections = useConnectionStore((s) => s.connections);
  const activeConn = connections.find((c) => c.id === activeConnectionId);
  const driver = activeConn?.driver ?? "";

  // Expose the running local inline sidecar as a synthetic provider when
  // inline AI is enabled AND the sidecar is actually Ready. The entry has
  // no persistent config — the backend recognises its sentinel id and
  // routes the request straight at `http://127.0.0.1:<port>/v1`.
  const inlineEnabled = useInlineAiStore((s) => s.enabled);
  const inlineSidecar = useInlineAiStore((s) => s.sidecar);
  const inlineModels = useInlineAiStore((s) => s.models);
  const inlineUseAsDefault = useInlineAiStore((s) => s.useAsDefaultProvider);
  const virtualInlineProvider = useMemo<AiProviderConfig | null>(
    () => buildVirtualInlineProvider(inlineEnabled, inlineSidecar, inlineModels),
    [inlineEnabled, inlineSidecar, inlineModels],
  );

  const displayProviders = useMemo<AiProviderConfig[]>(() => {
    return virtualInlineProvider
      ? [...providers, virtualInlineProvider]
      : providers;
  }, [providers, virtualInlineProvider]);

  const hasProvider = displayProviders.length > 0;
  const defaultProvider =
    inlineUseAsDefault && virtualInlineProvider
      ? virtualInlineProvider
      : getDefaultProvider();

  // Load providers on mount
  useEffect(() => {
    loadProviders();
  }, [loadProviders]);

  // Focus input when palette opens
  useEffect(() => {
    if (paletteOpen) {
      setPrompt("");
      setHistoryIndex(-1);
      draftRef.current = "";
      // Small delay to ensure DOM is ready
      requestAnimationFrame(() => inputRef.current?.focus());
    }
  }, [paletteOpen]);

  // Auto-fire for pre-set flows (from context menu)
  const hasFiredRef = useRef(false);
  useEffect(() => {
    if (paletteOpen && paletteFlow && paletteSql && !hasFiredRef.current) {
      hasFiredRef.current = true;
      handleSubmitFlow(paletteFlow, paletteSql);
    }
    if (!paletteOpen) {
      hasFiredRef.current = false;
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [paletteOpen, paletteFlow, paletteSql]);

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

  const handleSubmitFlow = async (flow: AiFlow, sql?: string) => {
    if (streaming || !hasProvider) return;
    const schemaContext = buildSchemaContext();
    const editorSql = sql || getCurrentSql();

    switch (flow) {
      case "generate_sql":
        // sql here is actually the prompt text
        if (!sql) return;
        await generateSql(sql, schemaContext, driver);
        break;
      case "explain":
        if (!editorSql) return;
        await explainQuery(editorSql, schemaContext, driver);
        break;
      case "optimize":
        if (!editorSql) return;
        await optimizeQuery(editorSql, schemaContext, driver);
        break;
      case "format_sql":
        if (!editorSql) return;
        await formatSql(editorSql, schemaContext, driver);
        break;
      case "comment_sql":
        if (!editorSql) return;
        await commentSql(editorSql, schemaContext, driver);
        break;
      case "fix_query":
        if (!editorSql || !paletteError) return;
        await fixQuery(editorSql, paletteError, schemaContext, driver);
        break;
      case "document":
        if (!schemaContext) return;
        await generateDocs(schemaContext, driver);
        break;
    }
  };

  const handleSubmit = async () => {
    if (streaming) return;
    const text = prompt.trim();
    if (!text) return;

    addToPromptHistory(text);
    setHistoryIndex(-1);
    draftRef.current = "";

    // Check for prefix commands
    const firstWord = text.split(/\s/)[0].toLowerCase();
    const prefixFlow = PREFIX_COMMANDS[firstWord];

    if (prefixFlow) {
      if (SQL_CONTEXT_FLOWS.includes(prefixFlow)) {
        // These flows use editor SQL, not prompt text
        await handleSubmitFlow(prefixFlow);
      } else {
        // document flow
        await handleSubmitFlow(prefixFlow);
      }
    } else {
      // Default: natural language to SQL — include active editor query as context
      const editorSql = getCurrentSql();
      const fullPrompt = editorSql
        ? `Current query in editor:\n${editorSql}\n\nUser request: ${text}`
        : text;
      await handleSubmitFlow("generate_sql", fullPrompt);
    }
  };

  const handleInsertToEditor = () => {
    if (!currentResponse) return;
    const state = useEditorStore.getState();
    const tab = state.getActiveTab();
    if (tab) {
      state.setContent(tab.id, currentResponse);
    }
    closePalette();
  };

  // Save to history when stream finishes
  const prevStreamingRef = useRef(streaming);
  useEffect(() => {
    if (prevStreamingRef.current && !streaming && currentResponse && !error && paletteOpen) {
      const entry: AiHistoryEntry = {
        id: crypto.randomUUID(),
        timestamp: new Date().toISOString(),
        flow: currentFlow!,
        prompt: currentFlow === "generate_sql" ? prompt : getCurrentSql(),
        response: currentResponse,
        connectionId: activeConnectionId ?? undefined,
      };
      invoke("save_ai_history_entry", { entry })
        .then(() => useAiStore.getState().loadHistory())
        .catch((e) => console.error("Failed to save history:", e));
    }
    prevStreamingRef.current = streaming;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [streaming]);

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Escape" && !streaming) {
      e.preventDefault();
      closePalette();
    }
    // Prompt history navigation with Up/Down arrows
    if (e.key === "ArrowUp" && promptHistory.length > 0) {
      e.preventDefault();
      if (historyIndex === -1) {
        draftRef.current = prompt;
      }
      const newIndex = historyIndex === -1
        ? promptHistory.length - 1
        : Math.max(0, historyIndex - 1);
      setHistoryIndex(newIndex);
      setPrompt(promptHistory[newIndex]);
    }
    if (e.key === "ArrowDown" && historyIndex !== -1) {
      e.preventDefault();
      if (historyIndex >= promptHistory.length - 1) {
        setHistoryIndex(-1);
        setPrompt(draftRef.current);
      } else {
        const newIndex = historyIndex + 1;
        setHistoryIndex(newIndex);
        setPrompt(promptHistory[newIndex]);
      }
    }
    if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      handleSubmit();
    }
    // Plain Enter also submits (single-line feel), unless Shift is held for newline
    if (e.key === "Enter" && !e.ctrlKey && !e.metaKey && !e.shiftKey) {
      e.preventDefault();
      handleSubmit();
    }
  };

  const handleBackdropClick = (e: React.MouseEvent) => {
    if (e.target === e.currentTarget && !streaming) {
      closePalette();
    }
  };

  const handleOpenSettings = () => {
    closePalette();
    setPanel(true);
  };

  if (!paletteOpen) return null;

  const isPresetFlow = paletteFlow && SQL_CONTEXT_FLOWS.includes(paletteFlow);
  const canInsert =
    !streaming &&
    currentResponse &&
    currentFlow &&
    INSERTABLE_FLOWS.includes(currentFlow);
  const canCopy = !streaming && currentResponse;

  return (
    <div
      className="fixed inset-0 z-50 flex items-start justify-center bg-black/40 pt-[15vh]"
      onClick={handleBackdropClick}
    >
      <div className="flex w-full max-w-xl flex-col rounded-lg border border-border bg-background shadow-2xl">
        {/* Header */}
        <div className="flex items-center gap-2 border-b border-border px-3 py-2">
          <Sparkles size={14} className="text-primary" />
          <span className="text-xs font-medium text-muted-foreground">
            AI Assistant
          </span>
          {paletteFlow && (
            <span className="rounded bg-primary/10 px-1.5 py-0.5 text-[10px] font-medium text-primary">
              {AI_FLOW_LABELS[paletteFlow]}
            </span>
          )}
          <div className="flex-1" />
          {displayProviders.length > 0 && (
            <ProviderDropdown
              providers={displayProviders}
              selectedId={selectedProviderId}
              defaultProvider={defaultProvider}
              onSelect={setSelectedProviderId}
              disabled={streaming}
            />
          )}
          <button
            onClick={closePalette}
            className="rounded p-0.5 text-muted-foreground hover:bg-accent hover:text-accent-foreground"
          >
            <X size={14} />
          </button>
        </div>

        {/* No provider warning */}
        {!hasProvider && (
          <div className="flex items-start gap-2 border-b border-border bg-amber-500/10 px-3 py-2 text-xs text-amber-600 dark:text-amber-400">
            <AlertCircle size={14} className="mt-0.5 shrink-0" />
            <span>
              No AI provider configured.{" "}
              <button onClick={handleOpenSettings} className="underline">
                Open settings
              </button>{" "}
              to add one.
            </span>
          </div>
        )}

        {/* Input area */}
        {!isPresetFlow && (
          <div className="border-b border-border p-3">
            <textarea
              ref={inputRef}
              value={prompt}
              onChange={(e) => setPrompt(e.target.value)}
              onKeyDown={handleKeyDown}
              placeholder="Ask about your database... (e.g. &quot;show all orders from last month&quot;)&#10;Prefix: /explain /optimize /format /comment"
              className="input min-h-[48px] resize-none text-xs"
              rows={2}
              disabled={streaming}
            />
            <div className="mt-1.5 flex items-center justify-between text-[10px] text-muted-foreground">
              <span>
                Enter to send {"\u00b7"} Shift+Enter for newline
              </span>
              {!activeConnectionId && (
                <span className="text-amber-500">No connection active</span>
              )}
            </div>
          </div>
        )}

        {/* Pre-set flow info */}
        {isPresetFlow && paletteSql && (
          <div className="max-h-32 overflow-y-auto border-b border-border bg-muted/50 px-3 py-2">
            <pre className="text-[10px] leading-relaxed text-muted-foreground">
              {paletteSql.length > 200
                ? paletteSql.slice(0, 200) + "..."
                : paletteSql}
            </pre>
            {paletteError && (
              <pre className="mt-1 whitespace-pre-wrap text-[10px] leading-relaxed text-destructive">
                {paletteError.length > 300
                  ? paletteError.slice(0, 300) + "..."
                  : paletteError}
              </pre>
            )}
          </div>
        )}

        {/* Response area */}
        {(currentResponse || error || streaming) && (
          <div
            ref={responseRef}
            className="max-h-[40vh] overflow-y-auto p-3"
          >
            {error && (
              <div className="rounded-md bg-destructive/10 p-2 text-xs text-destructive">
                {error}
              </div>
            )}
            {currentResponse && (
              <pre className="whitespace-pre-wrap text-xs leading-relaxed text-foreground">
                {currentResponse}
              </pre>
            )}
            {streaming && !currentResponse && (
              <div className="flex items-center gap-2 text-xs text-muted-foreground">
                <Loader2 size={12} className="animate-spin" />
                Generating...
              </div>
            )}
          </div>
        )}

        {/* Action bar */}
        {(canInsert || canCopy) && (
          <div className="flex gap-1.5 border-t border-border px-3 py-2">
            {canInsert && currentFlow === "format_sql" && paletteSql && (
              <button
                onClick={() => {
                  openDiffPreview(paletteSql, currentResponse);
                  closePalette();
                }}
                className="btn-primary flex items-center gap-1 text-[10px]"
              >
                <Columns2 size={10} />
                Preview Changes
              </button>
            )}
            {canInsert && (
              <button
                onClick={handleInsertToEditor}
                className={`flex items-center gap-1 text-[10px] ${currentFlow === "format_sql" && paletteSql ? "btn-secondary" : "btn-primary"}`}
              >
                <Code size={10} />
                Apply Directly
              </button>
            )}
            {canCopy && (
              <button
                onClick={() => navigator.clipboard.writeText(currentResponse)}
                className="btn-secondary flex items-center gap-1 text-[10px]"
              >
                <ClipboardCopy size={10} />
                Copy
              </button>
            )}
          </div>
        )}
      </div>
    </div>
  );
}

function ProviderDropdown({
  providers,
  selectedId,
  defaultProvider,
  onSelect,
  disabled,
}: {
  providers: import("../types/ai").AiProviderConfig[];
  selectedId: string | null;
  defaultProvider: import("../types/ai").AiProviderConfig | undefined;
  onSelect: (id: string | null) => void;
  disabled: boolean;
}) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  const activeProvider = selectedId
    ? providers.find((p) => p.id === selectedId)
    : defaultProvider;

  const close = useCallback(() => setOpen(false), []);

  useEffect(() => {
    if (!open) return;
    const onMouseDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) close();
    };
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        close();
      }
    };
    document.addEventListener("mousedown", onMouseDown);
    document.addEventListener("keydown", onKeyDown, true);
    return () => {
      document.removeEventListener("mousedown", onMouseDown);
      document.removeEventListener("keydown", onKeyDown, true);
    };
  }, [open, close]);

  if (providers.length <= 1) {
    return (
      <span className="text-[10px] text-muted-foreground">
        {activeProvider?.name}
      </span>
    );
  }

  return (
    <div ref={ref} className="relative">
      <button
        onClick={() => !disabled && setOpen((v) => !v)}
        disabled={disabled}
        className="flex items-center gap-1 rounded px-1.5 py-0.5 text-[10px] text-muted-foreground hover:bg-accent hover:text-accent-foreground transition-colors disabled:opacity-40"
        title="Select AI provider"
      >
        <span className="max-w-28 truncate">{activeProvider?.name}</span>
        <ChevronDown size={10} />
      </button>
      {open && (
        <div className="absolute right-0 top-full z-50 mt-1 min-w-44 rounded-md border border-border bg-background py-1 shadow-lg">
          {providers.map((p) => {
            const isActive = selectedId ? p.id === selectedId : p.id === defaultProvider?.id;
            return (
              <button
                key={p.id}
                onClick={() => {
                  onSelect(p.id === defaultProvider?.id ? null : p.id);
                  close();
                }}
                className={`flex w-full items-center gap-2 px-3 py-1.5 text-left text-[11px] transition-colors ${
                  isActive
                    ? "bg-primary/10 text-primary"
                    : "hover:bg-accent hover:text-accent-foreground"
                }`}
              >
                <div className="flex-1 min-w-0">
                  <div className="truncate font-medium">{p.name}</div>
                  <div className="truncate text-[9px] text-muted-foreground">
                    {AI_PROVIDER_LABELS[p.provider]} — {p.model}
                  </div>
                </div>
                {p.isDefault && (
                  <span className="shrink-0 rounded bg-muted px-1 py-0.5 text-[8px] text-muted-foreground">
                    default
                  </span>
                )}
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}
