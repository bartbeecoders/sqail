export type AiProviderType =
  | "claude"
  | "openAi"
  | "openRouter"
  | "openAiCompatible"
  | "minimax"
  | "zai"
  | "claudeCodeCli"
  | "lmStudio"
  | "inlineLocal";

/** Sentinel provider id used for the synthesized "Local inline AI"
 *  dropdown entry — the backend (`commands.rs`) recognises this id and
 *  routes the request to the running llama-server sidecar. */
export const INLINE_LOCAL_PROVIDER_ID = "inline-local";

export interface AiProviderConfig {
  id: string;
  name: string;
  provider: AiProviderType;
  apiKey: string;
  model: string;
  baseUrl?: string;
  isDefault?: boolean;
  acceptInvalidCerts?: boolean;
}

export type AiFlow = "generate_sql" | "explain" | "optimize" | "document" | "format_sql" | "comment_sql" | "fix_query";

export interface AiHistoryEntry {
  id: string;
  timestamp: string;
  flow: AiFlow;
  prompt: string;
  response: string;
  connectionId?: string;
}

export interface AiStreamChunk {
  requestId: string;
  chunk: string;
  done: boolean;
}

export interface AiStreamError {
  requestId: string;
  error: string;
}

export const AI_PROVIDER_LABELS: Record<AiProviderType, string> = {
  claude: "Claude",
  openAi: "OpenAI",
  openRouter: "OpenRouter",
  openAiCompatible: "OpenAI Compatible",
  minimax: "Minimax",
  zai: "Z.ai",
  claudeCodeCli: "Claude Code CLI",
  lmStudio: "LM Studio",
  inlineLocal: "Local (Inline AI)",
};

export const AI_FLOW_LABELS: Record<AiFlow, string> = {
  generate_sql: "Generate SQL",
  explain: "Explain Query",
  optimize: "Optimize Query",
  document: "Generate Docs",
  format_sql: "Format SQL",
  comment_sql: "Add Comments",
  fix_query: "Fix Query",
};

/** Whether a provider type requires an API key */
export function providerNeedsApiKey(provider: AiProviderType): boolean {
  return provider !== "claudeCodeCli" && provider !== "lmStudio";
}

/** Whether a provider type has a configurable base URL */
export function providerHasBaseUrl(provider: AiProviderType): boolean {
  return provider === "openAiCompatible" || provider === "minimax" || provider === "zai" || provider === "lmStudio";
}

const DEFAULT_MODELS: Record<AiProviderType, string> = {
  claude: "claude-sonnet-4-20250514",
  openAi: "gpt-4o",
  openRouter: "anthropic/claude-sonnet-4",
  openAiCompatible: "model-name",
  minimax: "MiniMax-Text-01",
  zai: "GLM-4.7",
  claudeCodeCli: "claude-sonnet-4-20250514",
  lmStudio: "local-model",
  inlineLocal: "",
};

const DEFAULT_BASE_URLS: Partial<Record<AiProviderType, string>> = {
  minimax: "https://api.minimax.io/v1",
  zai: "https://api.z.ai/api/paas/v4",
  lmStudio: "https://llm.hideterms.com/v1",
};

export const ZAI_ENDPOINTS = [
  { url: "https://api.z.ai/api/coding/paas/v4", label: "Coding" },
  { url: "https://api.z.ai/api/paas/v4", label: "General" },
] as const;

export const ZAI_MODELS = [
  "GLM-5.1",
  "GLM-5V-Turbo",
  "GLM-5",
  "GLM-5-Turbo",
  "GLM-4.7",
  "GLM-4.6",
  "GLM-4.5",
] as const;

export function defaultProvider(provider: AiProviderType = "claude"): AiProviderConfig {
  return {
    id: "",
    name: "",
    provider,
    apiKey: "",
    model: DEFAULT_MODELS[provider],
    baseUrl: DEFAULT_BASE_URLS[provider],
    isDefault: false,
    acceptInvalidCerts: false,
  };
}

export function getDefaultModel(provider: AiProviderType): string {
  return DEFAULT_MODELS[provider];
}

export function getDefaultBaseUrl(provider: AiProviderType): string | undefined {
  return DEFAULT_BASE_URLS[provider];
}

export interface OpenRouterModel {
  id: string;
  name: string;
}
