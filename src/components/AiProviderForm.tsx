import { useState, useEffect, useRef } from "react";
import { X, Loader2, CheckCircle2, XCircle, ChevronDown, Search } from "lucide-react";
import { cn } from "../lib/utils";
import { useAiStore } from "../stores/aiStore";
import {
  type AiProviderConfig,
  type AiProviderType,
  type OpenRouterModel,
  defaultProvider,
  AI_PROVIDER_LABELS,
  providerNeedsApiKey,
  providerHasBaseUrl,
  getDefaultModel,
  getDefaultBaseUrl,
  ZAI_ENDPOINTS,
  ZAI_MODELS,
} from "../types/ai";

interface AiProviderFormProps {
  initial?: AiProviderConfig;
  onClose: () => void;
}

type TestStatus = "idle" | "testing" | "success" | "error";

// Two rows of provider buttons
const PROVIDER_ROW_1: AiProviderType[] = ["claude", "openAi", "openRouter", "minimax"];
const PROVIDER_ROW_2: AiProviderType[] = ["zai", "claudeCodeCli", "lmStudio", "openAiCompatible"];

export default function AiProviderForm({ initial, onClose }: AiProviderFormProps) {
  const [form, setForm] = useState<AiProviderConfig>(initial ?? defaultProvider());
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  const [testStatus, setTestStatus] = useState<TestStatus>("idle");
  const [testMessage, setTestMessage] = useState("");

  const [orModels, setOrModels] = useState<OpenRouterModel[]>([]);
  const [orLoading, setOrLoading] = useState(false);
  const [orSearch, setOrSearch] = useState("");
  const [orOpen, setOrOpen] = useState(false);
  const orDropdownRef = useRef<HTMLDivElement>(null);

  const { createProvider, updateProvider, testProvider, fetchOpenRouterModels } = useAiStore();
  const isEdit = !!initial;

  // Fetch OpenRouter models when provider is openRouter and API key is set
  useEffect(() => {
    if (form.provider !== "openRouter" || !form.apiKey) {
      setOrModels([]);
      return;
    }
    let cancelled = false;
    setOrLoading(true);
    fetchOpenRouterModels(form.apiKey, form.acceptInvalidCerts ?? false)
      .then((models) => {
        if (!cancelled) setOrModels(models);
      })
      .catch(() => {
        if (!cancelled) setOrModels([]);
      })
      .finally(() => {
        if (!cancelled) setOrLoading(false);
      });
    return () => { cancelled = true; };
  }, [form.provider, form.apiKey, form.acceptInvalidCerts, fetchOpenRouterModels]);

  // Close dropdown on outside click
  useEffect(() => {
    const handler = (e: MouseEvent) => {
      if (orDropdownRef.current && !orDropdownRef.current.contains(e.target as Node)) {
        setOrOpen(false);
      }
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, []);

  const set = <K extends keyof AiProviderConfig>(key: K, value: AiProviderConfig[K]) =>
    setForm((prev) => ({ ...prev, [key]: value }));

  const handleProviderChange = (provider: AiProviderType) => {
    setForm((prev) => ({
      ...prev,
      provider,
      model: getDefaultModel(provider),
      baseUrl: getDefaultBaseUrl(provider) ?? (providerHasBaseUrl(provider) ? prev.baseUrl : undefined),
      apiKey: providerNeedsApiKey(provider) ? prev.apiKey : "",
    }));
    setTestStatus("idle");
    setTestMessage("");
  };

  const handleTest = async () => {
    setTestStatus("testing");
    setTestMessage("");
    try {
      const msg = await testProvider(form);
      setTestStatus("success");
      setTestMessage(msg);
    } catch (e) {
      setTestStatus("error");
      setTestMessage(String(e));
    }
  };

  const handleSave = async () => {
    setSaving(true);
    setError("");
    try {
      if (isEdit) {
        await updateProvider(form);
      } else {
        await createProvider(form);
      }
      onClose();
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  };

  const needsApiKey = providerNeedsApiKey(form.provider);
  const hasBaseUrl = providerHasBaseUrl(form.provider);
  const needsModel = form.provider !== "claudeCodeCli";

  // For claudeCodeCli, we only need name + default checkbox
  const canSave = form.name && (needsApiKey ? form.apiKey : true);

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50">
      <div className="w-full max-w-lg rounded-lg border border-border bg-background p-5 shadow-xl">
        <div className="mb-4 flex items-center justify-between">
          <h2 className="text-sm font-semibold">
            {isEdit ? "Edit AI Provider" : "New AI Provider"}
          </h2>
          <button
            onClick={onClose}
            className="rounded p-1 text-muted-foreground hover:bg-accent hover:text-accent-foreground"
          >
            <X size={16} />
          </button>
        </div>

        <div className="space-y-3">
          {/* Name */}
          <Field label="Name">
            <input
              value={form.name}
              onChange={(e) => set("name", e.target.value)}
              placeholder="My AI Provider"
              className="input"
            />
          </Field>

          {/* Provider type - two rows */}
          <Field label="Provider">
            <div className="space-y-1">
              <div className="flex gap-1">
                {PROVIDER_ROW_1.map((p) => (
                  <ProviderButton
                    key={p}
                    provider={p}
                    selected={form.provider === p}
                    onClick={() => handleProviderChange(p)}
                  />
                ))}
              </div>
              <div className="flex gap-1">
                {PROVIDER_ROW_2.map((p) => (
                  <ProviderButton
                    key={p}
                    provider={p}
                    selected={form.provider === p}
                    onClick={() => handleProviderChange(p)}
                  />
                ))}
              </div>
            </div>
          </Field>

          {/* API Key */}
          {needsApiKey && (
            <Field label="API Key">
              <input
                type="password"
                value={form.apiKey}
                onChange={(e) => set("apiKey", e.target.value)}
                placeholder="sk-..."
                className="input"
              />
            </Field>
          )}

          {/* API Key optional for LM Studio */}
          {form.provider === "lmStudio" && (
            <Field label="API Key (optional)">
              <input
                type="password"
                value={form.apiKey}
                onChange={(e) => set("apiKey", e.target.value)}
                placeholder="Leave empty if not required"
                className="input"
              />
            </Field>
          )}

          {/* Z.ai Endpoint */}
          {form.provider === "zai" && (
            <Field label="Endpoint">
              <div className="flex gap-1">
                {ZAI_ENDPOINTS.map((ep) => (
                  <button
                    key={ep.url}
                    type="button"
                    onClick={() => set("baseUrl", ep.url)}
                    className={cn(
                      "flex-1 rounded-md px-2 py-1.5 text-[11px] font-medium transition-colors",
                      (form.baseUrl ?? ZAI_ENDPOINTS[1].url) === ep.url
                        ? "bg-primary text-primary-foreground"
                        : "bg-muted text-muted-foreground hover:bg-accent",
                    )}
                  >
                    {ep.label}
                  </button>
                ))}
              </div>
              <p className="mt-1 text-[10px] text-muted-foreground truncate">
                {form.baseUrl ?? ZAI_ENDPOINTS[1].url}
              </p>
            </Field>
          )}

          {/* Model */}
          {needsModel && form.provider === "openRouter" ? (
            <Field label="Model">
              <div className="relative" ref={orDropdownRef}>
                <button
                  type="button"
                  onClick={() => setOrOpen(!orOpen)}
                  className="input flex w-full items-center justify-between gap-2 text-left"
                >
                  <span className={cn("truncate", !form.model && "text-muted-foreground")}>
                    {form.model || "Select a model..."}
                  </span>
                  {orLoading ? (
                    <Loader2 size={14} className="shrink-0 animate-spin text-muted-foreground" />
                  ) : (
                    <ChevronDown size={14} className="shrink-0 text-muted-foreground" />
                  )}
                </button>
                {orOpen && (
                  <div className="absolute z-50 mt-1 max-h-60 w-full overflow-hidden rounded-md border border-border bg-background shadow-lg">
                    <div className="flex items-center gap-1 border-b border-border px-2 py-1.5">
                      <Search size={12} className="text-muted-foreground" />
                      <input
                        value={orSearch}
                        onChange={(e) => setOrSearch(e.target.value)}
                        placeholder="Search models..."
                        className="flex-1 bg-transparent text-xs outline-none"
                        autoFocus
                      />
                    </div>
                    <div className="max-h-48 overflow-y-auto">
                      {orModels.length === 0 && !orLoading && (
                        <div className="px-2 py-3 text-center text-xs text-muted-foreground">
                          {form.apiKey ? "No models found. Check your API key." : "Enter an API key to load models."}
                        </div>
                      )}
                      {orModels
                        .filter(
                          (m) =>
                            !orSearch ||
                            m.id.toLowerCase().includes(orSearch.toLowerCase()) ||
                            m.name.toLowerCase().includes(orSearch.toLowerCase()),
                        )
                        .map((m) => (
                          <button
                            key={m.id}
                            type="button"
                            onClick={() => {
                              set("model", m.id);
                              setOrOpen(false);
                              setOrSearch("");
                            }}
                            className={cn(
                              "flex w-full flex-col px-2 py-1.5 text-left text-xs hover:bg-accent",
                              form.model === m.id && "bg-accent",
                            )}
                          >
                            <span className="font-medium">{m.name}</span>
                            <span className="text-[10px] text-muted-foreground">{m.id}</span>
                          </button>
                        ))}
                    </div>
                  </div>
                )}
              </div>
            </Field>
          ) : needsModel && form.provider === "zai" ? (
            <Field label="Model">
              <select
                value={form.model}
                onChange={(e) => set("model", e.target.value)}
                className="input"
              >
                {ZAI_MODELS.map((m) => (
                  <option key={m} value={m}>{m}</option>
                ))}
              </select>
            </Field>
          ) : needsModel ? (
            <Field label="Model">
              <input
                value={form.model}
                onChange={(e) => set("model", e.target.value)}
                placeholder={getDefaultModel(form.provider)}
                className="input"
              />
            </Field>
          ) : null}

          {/* Base URL (not shown for Z.ai since it has its own endpoint selector) */}
          {hasBaseUrl && form.provider !== "zai" && (
            <Field label="Base URL">
              <input
                value={form.baseUrl ?? ""}
                onChange={(e) => set("baseUrl", e.target.value || undefined)}
                placeholder={getDefaultBaseUrl(form.provider) ?? "https://api.example.com/v1"}
                className="input"
              />
            </Field>
          )}

          {/* Claude Code CLI info */}
          {form.provider === "claudeCodeCli" && (
            <div className="rounded-md bg-muted p-2 text-xs text-muted-foreground">
              Uses the <code className="font-mono text-foreground">claude</code> CLI binary from your PATH.
              No API key needed — it uses your existing Claude Code authentication.
            </div>
          )}

          {/* Accept invalid certs */}
          <Field label="">
            <label className="flex items-center gap-2 text-xs">
              <input
                type="checkbox"
                checked={form.acceptInvalidCerts ?? false}
                onChange={(e) => set("acceptInvalidCerts", e.target.checked)}
                className="rounded border-border"
              />
              Disable SSL certificate verification
            </label>
            {(form.acceptInvalidCerts) && (
              <p className="mt-1 text-[11px] text-amber-500">
                Warning: disabling certificate verification makes the connection insecure. Only use this on trusted corporate networks.
              </p>
            )}
          </Field>

          {/* Default checkbox */}
          <Field label="">
            <label className="flex items-center gap-2 text-xs">
              <input
                type="checkbox"
                checked={form.isDefault ?? false}
                onChange={(e) => set("isDefault", e.target.checked)}
                className="rounded border-border"
              />
              Set as default provider
            </label>
          </Field>

          {/* Test result */}
          {testStatus !== "idle" && (
            <div
              className={cn(
                "flex items-start gap-2 rounded-md p-2 text-xs",
                testStatus === "testing" && "bg-muted text-muted-foreground",
                testStatus === "success" && "bg-success/10 text-success",
                testStatus === "error" && "bg-destructive/10 text-destructive",
              )}
            >
              {testStatus === "testing" && <Loader2 size={14} className="mt-0.5 animate-spin" />}
              {testStatus === "success" && <CheckCircle2 size={14} className="mt-0.5" />}
              {testStatus === "error" && <XCircle size={14} className="mt-0.5 shrink-0" />}
              <span className="break-all">{testStatus === "testing" ? "Testing..." : testMessage}</span>
            </div>
          )}

          {/* Error */}
          {error && (
            <div className="rounded-md bg-destructive/10 p-2 text-xs text-destructive">
              {error}
            </div>
          )}

          {/* Actions */}
          <div className="flex justify-end gap-2 pt-2">
            <button
              onClick={handleTest}
              disabled={testStatus === "testing"}
              className="btn-secondary"
            >
              Test Connection
            </button>
            <button onClick={onClose} className="btn-secondary">
              Cancel
            </button>
            <button
              onClick={handleSave}
              disabled={saving || !canSave}
              className="btn-primary"
            >
              {saving ? "Saving..." : isEdit ? "Update" : "Create"}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

function ProviderButton({
  provider,
  selected,
  onClick,
}: {
  provider: AiProviderType;
  selected: boolean;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className={cn(
        "flex-1 rounded-md px-2 py-1.5 text-[11px] font-medium transition-colors",
        selected
          ? "bg-primary text-primary-foreground"
          : "bg-muted text-muted-foreground hover:bg-accent",
      )}
    >
      {AI_PROVIDER_LABELS[provider]}
    </button>
  );
}

function Field({
  label,
  children,
  className,
}: {
  label: string;
  children: React.ReactNode;
  className?: string;
}) {
  return (
    <label className={cn("block", className)}>
      {label && <span className="mb-1 block text-xs font-medium text-muted-foreground">{label}</span>}
      {children}
    </label>
  );
}
