export type AiProviderType =
  | "claude"
  | "openAi"
  | "openRouter"
  | "openAiCompatible"
  | "minimax"
  | "zai"
  | "claudeCodeCli"
  | "lmStudio";

export interface AiProviderConfig {
  id: string;
  name: string;
  provider: AiProviderType;
  apiKey: string;
  model: string;
  baseUrl?: string;
  isDefault?: boolean;
}

export type AiFlow = "generate_sql" | "explain" | "optimize" | "document" | "format_sql" | "comment_sql";

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
};

export const AI_FLOW_LABELS: Record<AiFlow, string> = {
  generate_sql: "Generate SQL",
  explain: "Explain Query",
  optimize: "Optimize Query",
  document: "Generate Docs",
  format_sql: "Format SQL",
  comment_sql: "Add Comments",
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
  zai: "deepseek-chat",
  claudeCodeCli: "claude-sonnet-4-20250514",
  lmStudio: "local-model",
};

const DEFAULT_BASE_URLS: Partial<Record<AiProviderType, string>> = {
  minimax: "https://api.minimax.io/v1",
  zai: "https://api.z.ai/api/paas/v4",
  lmStudio: "https://llm.hideterms.com/v1",
};

export function defaultProvider(provider: AiProviderType = "claude"): AiProviderConfig {
  return {
    id: "",
    name: "",
    provider,
    apiKey: "",
    model: DEFAULT_MODELS[provider],
    baseUrl: DEFAULT_BASE_URLS[provider],
    isDefault: false,
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
