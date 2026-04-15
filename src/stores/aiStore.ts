import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import type { AiProviderConfig, AiFlow, AiHistoryEntry, OpenRouterModel } from "../types/ai";
import { useEditorStore } from "./editorStore";

/** Strip <think>...</think> blocks emitted by reasoning models. */
function stripThinkingBlocks(text: string): string {
  // Remove complete <think>...</think> blocks (greedy across newlines)
  let result = text.replace(/<think>[\s\S]*?<\/think>/g, "");
  // If there's an unclosed <think> tag (still streaming), hide everything from it onward
  const openIdx = result.indexOf("<think>");
  if (openIdx !== -1) {
    result = result.slice(0, openIdx);
  }
  return result.trimStart();
}

interface PaletteOptions {
  flow?: AiFlow;
  sql?: string;
  errorMessage?: string;
}

interface AiState {
  providers: AiProviderConfig[];
  history: AiHistoryEntry[];
  panelOpen: boolean;
  streaming: boolean;
  currentRequestId: string | null;
  currentResponse: string;
  currentFlow: AiFlow | null;
  error: string | null;

  // Command palette state
  paletteOpen: boolean;
  paletteFlow: AiFlow | null;
  paletteSql: string;
  paletteError: string;

  // Diff preview state (for AI format preview)
  diffPreview: { original: string; modified: string } | null;
  openDiffPreview: (original: string, modified: string) => void;
  closeDiffPreview: () => void;
  acceptDiffPreview: () => void;

  // Provider selection for the palette (null = use default)
  selectedProviderId: string | null;
  setSelectedProviderId: (id: string | null) => void;

  loadProviders: () => Promise<void>;
  createProvider: (config: AiProviderConfig) => Promise<void>;
  updateProvider: (config: AiProviderConfig) => Promise<void>;
  deleteProvider: (id: string) => Promise<void>;
  setDefaultProvider: (id: string) => Promise<void>;
  getDefaultProvider: () => AiProviderConfig | undefined;

  testProvider: (config: AiProviderConfig) => Promise<string>;
  fetchOpenRouterModels: (apiKey: string, acceptInvalidCerts: boolean) => Promise<OpenRouterModel[]>;

  togglePanel: () => void;
  setPanel: (open: boolean) => void;

  openPalette: (options?: PaletteOptions) => void;
  closePalette: () => void;

  generateSql: (prompt: string, schemaContext: string, driver: string) => Promise<void>;
  explainQuery: (sql: string, schemaContext: string, driver: string) => Promise<void>;
  optimizeQuery: (sql: string, schemaContext: string, driver: string) => Promise<void>;
  generateDocs: (schemaContext: string, driver: string) => Promise<void>;
  formatSql: (sql: string, schemaContext: string, driver: string) => Promise<void>;
  commentSql: (sql: string, schemaContext: string, driver: string) => Promise<void>;
  fixQuery: (sql: string, errorMessage: string, schemaContext: string, driver: string) => Promise<void>;

  appendChunk: (requestId: string, chunk: string) => void;
  finishStream: (requestId: string, fullText: string) => void;
  setStreamError: (requestId: string, error: string) => void;

  loadHistory: () => Promise<void>;
  clearHistory: () => Promise<void>;

  /** Last 10 prompts entered in the palette (newest last). */
  promptHistory: string[];
  addToPromptHistory: (prompt: string) => void;
}

export const useAiStore = create<AiState>((set, get) => ({
  providers: [],
  history: [],
  panelOpen: false,
  streaming: false,
  currentRequestId: null,
  currentResponse: "",
  currentFlow: null,
  error: null,

  paletteOpen: false,
  paletteFlow: null,
  paletteSql: "",
  paletteError: "",
  diffPreview: null,
  selectedProviderId: null,
  promptHistory: [],

  setSelectedProviderId: (id) => set({ selectedProviderId: id }),

  openDiffPreview: (original, modified) => {
    set({ diffPreview: { original, modified } });
  },

  closeDiffPreview: () => {
    set({ diffPreview: null });
  },

  acceptDiffPreview: () => {
    const { diffPreview } = get();
    if (!diffPreview) return;
    const editorState = useEditorStore.getState();
    const tab = editorState.getActiveTab();
    if (tab) {
      editorState.setContent(tab.id, diffPreview.modified);
    }
    set({ diffPreview: null });
  },

  loadProviders: async () => {
    try {
      const providers = await invoke<AiProviderConfig[]>("list_ai_providers");
      set({ providers });
    } catch (e) {
      console.error("Failed to load AI providers:", e);
    }
  },

  createProvider: async (config) => {
    await invoke<AiProviderConfig>("create_ai_provider", { config });
    await get().loadProviders();
  },

  updateProvider: async (config) => {
    await invoke<AiProviderConfig>("update_ai_provider", { config });
    await get().loadProviders();
  },

  deleteProvider: async (id) => {
    await invoke<void>("delete_ai_provider", { id });
    await get().loadProviders();
  },

  setDefaultProvider: async (id) => {
    await invoke<void>("set_default_ai_provider", { id });
    await get().loadProviders();
  },

  getDefaultProvider: () => {
    const { providers } = get();
    return providers.find((p) => p.isDefault) ?? providers[0];
  },

  testProvider: async (config) => {
    return invoke<string>("test_ai_provider", { config });
  },

  fetchOpenRouterModels: async (apiKey, acceptInvalidCerts) => {
    return invoke<OpenRouterModel[]>("list_openrouter_models", { apiKey, acceptInvalidCerts });
  },

  togglePanel: () => set((s) => ({ panelOpen: !s.panelOpen })),
  setPanel: (open) => set({ panelOpen: open }),

  openPalette: (options) =>
    set({
      paletteOpen: true,
      paletteFlow: options?.flow ?? null,
      paletteSql: options?.sql ?? "",
      paletteError: options?.errorMessage ?? "",
      selectedProviderId: null,
      currentResponse: "",
      error: null,
    }),
  closePalette: () =>
    set({ paletteOpen: false, paletteFlow: null, paletteSql: "", paletteError: "" }),

  addToPromptHistory: (prompt) => {
    const { promptHistory } = get();
    // Avoid consecutive duplicates
    if (promptHistory[promptHistory.length - 1] === prompt) return;
    const updated = [...promptHistory, prompt].slice(-10);
    set({ promptHistory: updated });
  },

  generateSql: async (prompt, schemaContext, driver) => {
    const providerId = get().selectedProviderId;
    set({ streaming: true, currentResponse: "", error: null, currentFlow: "generate_sql" });
    try {
      const requestId = await invoke<string>("ai_generate_sql", { prompt, schemaContext, driver, providerId });
      set({ currentRequestId: requestId });
    } catch (e) {
      set({ streaming: false, error: String(e) });
    }
  },

  explainQuery: async (sql, schemaContext, driver) => {
    const providerId = get().selectedProviderId;
    set({ streaming: true, currentResponse: "", error: null, currentFlow: "explain" });
    try {
      const requestId = await invoke<string>("ai_explain_query", { sql, schemaContext, driver, providerId });
      set({ currentRequestId: requestId });
    } catch (e) {
      set({ streaming: false, error: String(e) });
    }
  },

  optimizeQuery: async (sql, schemaContext, driver) => {
    const providerId = get().selectedProviderId;
    set({ streaming: true, currentResponse: "", error: null, currentFlow: "optimize" });
    try {
      const requestId = await invoke<string>("ai_optimize_query", { sql, schemaContext, driver, providerId });
      set({ currentRequestId: requestId });
    } catch (e) {
      set({ streaming: false, error: String(e) });
    }
  },

  generateDocs: async (schemaContext, driver) => {
    const providerId = get().selectedProviderId;
    set({ streaming: true, currentResponse: "", error: null, currentFlow: "document" });
    try {
      const requestId = await invoke<string>("ai_generate_docs", { schemaContext, driver, providerId });
      set({ currentRequestId: requestId });
    } catch (e) {
      set({ streaming: false, error: String(e) });
    }
  },

  formatSql: async (sql, schemaContext, driver) => {
    const providerId = get().selectedProviderId;
    set({ streaming: true, currentResponse: "", error: null, currentFlow: "format_sql" });
    try {
      const requestId = await invoke<string>("ai_format_sql", { sql, schemaContext, driver, providerId });
      set({ currentRequestId: requestId });
    } catch (e) {
      set({ streaming: false, error: String(e) });
    }
  },

  commentSql: async (sql, schemaContext, driver) => {
    const providerId = get().selectedProviderId;
    set({ streaming: true, currentResponse: "", error: null, currentFlow: "comment_sql" });
    try {
      const requestId = await invoke<string>("ai_comment_sql", { sql, schemaContext, driver, providerId });
      set({ currentRequestId: requestId });
    } catch (e) {
      set({ streaming: false, error: String(e) });
    }
  },

  fixQuery: async (sql, errorMessage, schemaContext, driver) => {
    const providerId = get().selectedProviderId;
    set({ streaming: true, currentResponse: "", error: null, currentFlow: "fix_query" });
    try {
      const requestId = await invoke<string>("ai_fix_query", {
        sql,
        errorMessage,
        schemaContext,
        driver,
        providerId,
      });
      set({ currentRequestId: requestId });
    } catch (e) {
      set({ streaming: false, error: String(e) });
    }
  },

  appendChunk: (requestId, chunk) => {
    const state = get();
    if (state.currentRequestId === requestId) {
      const raw = state.currentResponse + chunk;
      set({ currentResponse: stripThinkingBlocks(raw) });
    }
  },

  finishStream: (requestId, fullText) => {
    const state = get();
    if (state.currentRequestId === requestId) {
      set({ streaming: false, currentResponse: stripThinkingBlocks(fullText), currentRequestId: null });
    }
  },

  setStreamError: (requestId, error) => {
    const state = get();
    if (state.currentRequestId === requestId) {
      set({ streaming: false, error, currentRequestId: null });
    }
  },

  loadHistory: async () => {
    try {
      const history = await invoke<AiHistoryEntry[]>("list_ai_history");
      set({ history });
    } catch (e) {
      console.error("Failed to load AI history:", e);
    }
  },

  clearHistory: async () => {
    try {
      await invoke<void>("clear_ai_history");
      set({ history: [] });
    } catch (e) {
      console.error("Failed to clear AI history:", e);
    }
  },
}));
