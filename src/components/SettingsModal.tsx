import { useState, useEffect, useRef, useCallback } from "react";
import { X, RotateCcw, Settings, Keyboard, Info, Code2, Plus, Trash2, Pencil, Sparkles, Check } from "lucide-react";
import { cn } from "../lib/utils";
import { useShortcutStore } from "../stores/shortcutStore";
import { useSettingsStore } from "../stores/settingsStore";
import { useSnippetStore, type SqlSnippet } from "../stores/snippetStore";
import { useAiStore } from "../stores/aiStore";
import { AI_PROVIDER_LABELS } from "../types/ai";
import type { AiProviderConfig } from "../types/ai";
import AiProviderForm from "./AiProviderForm";
import {
  SHORTCUT_ACTIONS,
  CATEGORY_LABELS,
  eventToShortcut,
} from "../types/shortcuts";

interface SettingsModalProps {
  onClose: () => void;
  initialTab?: SettingsTab;
}

type SettingsTab = "general" | "ai" | "shortcuts" | "snippets" | "about";

export default function SettingsModal({ onClose, initialTab = "general" }: SettingsModalProps) {
  const [activeTab, setActiveTab] = useState<SettingsTab>(initialTab);

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50">
      <div className="flex h-[560px] w-full max-w-2xl flex-col rounded-lg border border-border bg-background shadow-xl">
        {/* Header */}
        <div className="flex items-center justify-between border-b border-border px-5 py-3">
          <h2 className="text-sm font-semibold">Settings</h2>
          <button
            onClick={onClose}
            className="rounded p-1 text-muted-foreground hover:bg-accent hover:text-accent-foreground"
          >
            <X size={16} />
          </button>
        </div>

        <div className="flex flex-1 overflow-hidden">
          {/* Sidebar tabs */}
          <nav className="w-44 shrink-0 border-r border-border bg-muted/30 p-2">
            <TabButton
              active={activeTab === "general"}
              icon={<Settings size={14} />}
              label="General"
              onClick={() => setActiveTab("general")}
            />
            <TabButton
              active={activeTab === "ai"}
              icon={<Sparkles size={14} />}
              label="AI Providers"
              onClick={() => setActiveTab("ai")}
            />
            <TabButton
              active={activeTab === "shortcuts"}
              icon={<Keyboard size={14} />}
              label="Keyboard Shortcuts"
              onClick={() => setActiveTab("shortcuts")}
            />
            <TabButton
              active={activeTab === "snippets"}
              icon={<Code2 size={14} />}
              label="Query Snippets"
              onClick={() => setActiveTab("snippets")}
            />
            <TabButton
              active={activeTab === "about"}
              icon={<Info size={14} />}
              label="About / Releases"
              onClick={() => setActiveTab("about")}
            />
          </nav>

          {/* Content */}
          <div className="flex-1 overflow-y-auto p-5">
            {activeTab === "general" && <GeneralTab />}
            {activeTab === "ai" && <AiProvidersTab />}
            {activeTab === "shortcuts" && <ShortcutsTab />}
            {activeTab === "snippets" && <SnippetsTab />}
            {activeTab === "about" && <AboutTab />}
          </div>
        </div>
      </div>
    </div>
  );
}

function TabButton({
  active,
  icon,
  label,
  onClick,
}: {
  active: boolean;
  icon: React.ReactNode;
  label: string;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className={cn(
        "flex w-full items-center gap-2 rounded-md px-3 py-2 text-xs font-medium transition-colors",
        active
          ? "bg-primary text-primary-foreground"
          : "text-muted-foreground hover:bg-accent hover:text-accent-foreground",
      )}
    >
      {icon}
      {label}
    </button>
  );
}

// ── General Tab ────────────────────────────────────────────

function GeneralTab() {
  const settings = useSettingsStore();
  const update = settings.updateSetting;

  return (
    <div className="space-y-6">
      {/* Editor section */}
      <section>
        <h3 className="mb-3 text-xs font-semibold uppercase tracking-wider text-muted-foreground">
          Editor
        </h3>
        <div className="space-y-3">
          <SettingRow label="Font Size">
            <input
              type="number"
              min={8}
              max={32}
              value={settings.editorFontSize}
              onChange={(e) => update("editorFontSize", Number(e.target.value))}
              className="input w-20 text-center"
            />
          </SettingRow>
          <SettingRow label="Font Family">
            <input
              value={settings.editorFontFamily}
              onChange={(e) => update("editorFontFamily", e.target.value)}
              className="input w-64"
            />
          </SettingRow>
          <SettingRow label="Tab Size">
            <select
              value={settings.editorTabSize}
              onChange={(e) => update("editorTabSize", Number(e.target.value))}
              className="input w-20"
            >
              <option value={2}>2</option>
              <option value={4}>4</option>
              <option value={8}>8</option>
            </select>
          </SettingRow>
          <SettingRow label="Minimap">
            <ToggleSwitch
              checked={settings.editorMinimap}
              onChange={(v) => update("editorMinimap", v)}
            />
          </SettingRow>
          <SettingRow label="Word Wrap">
            <ToggleSwitch
              checked={settings.editorWordWrap}
              onChange={(v) => update("editorWordWrap", v)}
            />
          </SettingRow>
          <SettingRow label="Line Numbers">
            <ToggleSwitch
              checked={settings.editorLineNumbers}
              onChange={(v) => update("editorLineNumbers", v)}
            />
          </SettingRow>
        </div>
      </section>

      {/* Schema Tree section */}
      <section>
        <h3 className="mb-3 text-xs font-semibold uppercase tracking-wider text-muted-foreground">
          Schema Tree
        </h3>
        <div className="space-y-3">
          <SettingRow label="Font Size">
            <input
              type="number"
              min={8}
              max={24}
              value={settings.treeFontSize}
              onChange={(e) => update("treeFontSize", Number(e.target.value))}
              className="input w-20 text-center"
            />
          </SettingRow>
        </div>
      </section>

      {/* Appearance section */}
      <section>
        <h3 className="mb-3 text-xs font-semibold uppercase tracking-wider text-muted-foreground">
          Appearance
        </h3>
        <div className="space-y-3">
          <SettingRow label="Theme">
            <div className="flex gap-1">
              {(["system", "light", "dark"] as const).map((t) => (
                <button
                  key={t}
                  onClick={() => update("theme", t)}
                  className={cn(
                    "rounded-md px-3 py-1.5 text-xs font-medium transition-colors capitalize",
                    settings.theme === t
                      ? "bg-primary text-primary-foreground"
                      : "bg-muted text-muted-foreground hover:bg-accent",
                  )}
                >
                  {t}
                </button>
              ))}
            </div>
          </SettingRow>
        </div>
      </section>

      {/* Query section */}
      <section>
        <h3 className="mb-3 text-xs font-semibold uppercase tracking-wider text-muted-foreground">
          Query Execution
        </h3>
        <div className="space-y-3">
          <SettingRow label="Default Row Limit">
            <input
              type="number"
              min={1}
              max={100000}
              value={settings.defaultRowLimit}
              onChange={(e) => update("defaultRowLimit", Number(e.target.value))}
              className="input w-24 text-center"
            />
          </SettingRow>
          <SettingRow label="Query Timeout (seconds)">
            <input
              type="number"
              min={1}
              max={600}
              value={settings.queryTimeoutSeconds}
              onChange={(e) => update("queryTimeoutSeconds", Number(e.target.value))}
              className="input w-24 text-center"
            />
          </SettingRow>
        </div>
      </section>

      {/* SQL Formatting section */}
      <section>
        <h3 className="mb-3 text-xs font-semibold uppercase tracking-wider text-muted-foreground">
          SQL Formatting
        </h3>
        <div className="space-y-3">
          <SettingRow label="Indent Size">
            <select
              value={settings.formatIndent}
              onChange={(e) => update("formatIndent", Number(e.target.value))}
              className="input w-20"
            >
              <option value={2}>2</option>
              <option value={4}>4</option>
              <option value={8}>8</option>
            </select>
          </SettingRow>
          <SettingRow label="Uppercase Keywords">
            <ToggleSwitch
              checked={settings.formatUppercaseKeywords}
              onChange={(v) => update("formatUppercaseKeywords", v)}
            />
          </SettingRow>
          <SettingRow label="AND/OR on New Lines">
            <ToggleSwitch
              checked={settings.formatAndOrNewLine}
              onChange={(v) => update("formatAndOrNewLine", v)}
            />
          </SettingRow>
        </div>
      </section>

      {/* Behavior section */}
      <section>
        <h3 className="mb-3 text-xs font-semibold uppercase tracking-wider text-muted-foreground">
          Behavior
        </h3>
        <div className="space-y-3">
          <SettingRow label="Routine Drop Action">
            <div className="flex gap-1">
              {([
                { value: "definition" as const, label: "Open Definition" },
                { value: "exec" as const, label: "Generate Exec" },
              ]).map((opt) => (
                <button
                  key={opt.value}
                  onClick={() => update("routineDropAction", opt.value)}
                  className={cn(
                    "rounded-md px-3 py-1.5 text-xs font-medium transition-colors",
                    settings.routineDropAction === opt.value
                      ? "bg-primary text-primary-foreground"
                      : "bg-muted text-muted-foreground hover:bg-accent",
                  )}
                >
                  {opt.label}
                </button>
              ))}
            </div>
          </SettingRow>
        </div>
      </section>

      <div className="pt-2">
        <button onClick={settings.resetSettings} className="btn-secondary flex items-center gap-1 text-[10px]">
          <RotateCcw size={10} />
          Reset All Settings
        </button>
      </div>
    </div>
  );
}

function SettingRow({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="grid grid-cols-[1fr_auto] items-center gap-4">
      <span className="text-xs text-foreground">{label}</span>
      <div className="flex justify-end">{children}</div>
    </div>
  );
}

function ToggleSwitch({ checked, onChange }: { checked: boolean; onChange: (v: boolean) => void }) {
  return (
    <button
      onClick={() => onChange(!checked)}
      className={cn(
        "relative h-5 w-9 rounded-full transition-colors",
        checked ? "bg-primary" : "bg-muted-foreground/30",
      )}
    >
      <span
        className={cn(
          "absolute top-0.5 left-0.5 h-4 w-4 rounded-full bg-white transition-transform shadow-sm",
          checked && "translate-x-4",
        )}
      />
    </button>
  );
}

// ── AI Providers Tab ──────────────────────────────────────

function AiProvidersTab() {
  const { providers, loadProviders, deleteProvider, setDefaultProvider } = useAiStore();
  const [editingProvider, setEditingProvider] = useState<AiProviderConfig | undefined>();
  const [showForm, setShowForm] = useState(false);

  useEffect(() => {
    loadProviders();
  }, [loadProviders]);

  return (
    <>
      <div className="space-y-4">
        <div className="flex items-center justify-between">
          <div>
            <h3 className="text-sm font-semibold">AI Providers</h3>
            <p className="text-[11px] text-muted-foreground">
              Configure LLM providers for AI-powered SQL features.
            </p>
          </div>
          <button
            onClick={() => {
              setEditingProvider(undefined);
              setShowForm(true);
            }}
            className="flex items-center gap-1 rounded-md bg-primary px-2 py-1 text-[11px] font-medium text-primary-foreground hover:bg-primary/90"
          >
            <Plus size={12} />
            Add Provider
          </button>
        </div>

        {providers.length === 0 ? (
          <div className="rounded-md border border-dashed border-border py-8 text-center text-xs text-muted-foreground">
            No providers configured. Add one to enable AI features.
          </div>
        ) : (
          <div className="space-y-1">
            {providers.map((p) => (
              <div
                key={p.id}
                className="group flex items-center gap-3 rounded-md border border-border px-3 py-2"
              >
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-2">
                    <span className={cn("text-xs font-medium", p.isDefault && "text-primary")}>
                      {p.name}
                    </span>
                    {p.isDefault && (
                      <span className="rounded bg-primary/10 px-1 py-0.5 text-[9px] font-medium text-primary">
                        default
                      </span>
                    )}
                  </div>
                  <span className="block text-[10px] text-muted-foreground">
                    {AI_PROVIDER_LABELS[p.provider]} — {p.model}
                  </span>
                </div>
                <div className="flex shrink-0 gap-0.5 opacity-0 transition-opacity group-hover:opacity-100">
                  {!p.isDefault && (
                    <button
                      onClick={() => setDefaultProvider(p.id)}
                      className="rounded p-1 text-muted-foreground hover:bg-accent hover:text-foreground"
                      title="Set as default"
                    >
                      <Check size={12} />
                    </button>
                  )}
                  <button
                    onClick={() => {
                      setEditingProvider(p);
                      setShowForm(true);
                    }}
                    className="rounded p-1 text-muted-foreground hover:bg-accent hover:text-foreground"
                    title="Edit"
                  >
                    <Pencil size={12} />
                  </button>
                  <button
                    onClick={() => deleteProvider(p.id)}
                    className="rounded p-1 text-destructive/70 hover:bg-accent hover:text-destructive"
                    title="Delete"
                  >
                    <Trash2 size={12} />
                  </button>
                </div>
              </div>
            ))}
          </div>
        )}
      </div>

      {showForm && (
        <AiProviderForm
          initial={editingProvider}
          onClose={() => {
            setShowForm(false);
            setEditingProvider(undefined);
          }}
        />
      )}
    </>
  );
}

// ── Shortcuts Tab ──────────────────────────────────────────

function ShortcutsTab() {
  const { shortcuts, updateShortcut, resetShortcut, resetDefaults } =
    useShortcutStore();
  const [editingId, setEditingId] = useState<string | null>(null);
  const captureRef = useRef<HTMLDivElement>(null);

  const handleCapture = useCallback(
    (e: KeyboardEvent) => {
      if (!editingId) return;
      e.preventDefault();
      e.stopPropagation();

      // Escape cancels editing
      if (e.key === "Escape") {
        setEditingId(null);
        return;
      }

      const shortcut = eventToShortcut(e);
      if (shortcut) {
        updateShortcut(editingId, shortcut);
        setEditingId(null);
      }
    },
    [editingId, updateShortcut],
  );

  useEffect(() => {
    if (editingId) {
      window.addEventListener("keydown", handleCapture, true);
      return () => window.removeEventListener("keydown", handleCapture, true);
    }
  }, [editingId, handleCapture]);

  // Group actions by category
  const grouped = new Map<string, typeof SHORTCUT_ACTIONS>();
  for (const action of SHORTCUT_ACTIONS) {
    const list = grouped.get(action.category) ?? [];
    list.push(action);
    grouped.set(action.category, list);
  }

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <p className="text-xs text-muted-foreground">
          Click a shortcut to change it. Press Escape to cancel.
        </p>
        <button
          onClick={resetDefaults}
          className="btn-secondary flex items-center gap-1 text-[10px]"
        >
          <RotateCcw size={10} />
          Reset All
        </button>
      </div>

      {Array.from(grouped.entries()).map(([category, actions]) => (
        <div key={category}>
          <h3 className="mb-2 text-xs font-semibold uppercase tracking-wider text-muted-foreground">
            {CATEGORY_LABELS[category] ?? category}
          </h3>
          <div className="rounded-md border border-border">
            {actions.map((action, i) => {
              const currentKey = shortcuts[action.id] ?? action.defaultKey;
              const isEditing = editingId === action.id;
              const isModified = currentKey !== action.defaultKey;

              return (
                <div
                  key={action.id}
                  className={cn(
                    "flex items-center justify-between px-3 py-2 text-xs",
                    i > 0 && "border-t border-border",
                  )}
                >
                  <span className="text-foreground">{action.label}</span>
                  <div className="flex items-center gap-1.5">
                    <div
                      ref={isEditing ? captureRef : undefined}
                      onClick={() => setEditingId(action.id)}
                      className={cn(
                        "min-w-[120px] cursor-pointer rounded-md border px-2 py-1 text-center font-mono text-[11px] transition-colors",
                        isEditing
                          ? "border-primary bg-primary/10 text-primary"
                          : "border-border bg-muted text-muted-foreground hover:border-primary/50",
                        isModified && !isEditing && "text-foreground",
                      )}
                    >
                      {isEditing ? (
                        <span className="animate-pulse">Press keys...</span>
                      ) : (
                        formatShortcut(currentKey)
                      )}
                    </div>
                    {isModified && (
                      <button
                        onClick={() => resetShortcut(action.id)}
                        className="rounded p-0.5 text-muted-foreground hover:text-foreground"
                        title={`Reset to ${action.defaultKey}`}
                      >
                        <RotateCcw size={10} />
                      </button>
                    )}
                  </div>
                </div>
              );
            })}
          </div>
        </div>
      ))}
    </div>
  );
}

// ── Snippets Tab ───────────────────────────────────────────

function SnippetsTab() {
  const { allSnippets, userSnippets, addSnippet, updateSnippet, deleteSnippet, resetToDefaults } =
    useSnippetStore();
  const [editingId, setEditingId] = useState<string | null>(null);
  const [showNew, setShowNew] = useState(false);

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <div>
          <h3 className="text-sm font-semibold">Query Snippets</h3>
          <p className="text-[11px] text-muted-foreground">
            Type a prefix in the editor to expand a snippet. Use <code className="text-[10px] bg-muted px-1 rounded">{"$1"}</code>, <code className="text-[10px] bg-muted px-1 rounded">{"$2"}</code> for tab stops.
          </p>
        </div>
        <div className="flex gap-1">
          <button
            onClick={() => { setShowNew(true); setEditingId(null); }}
            className="flex items-center gap-1 rounded-md bg-primary px-2 py-1 text-[11px] font-medium text-primary-foreground hover:bg-primary/90"
          >
            <Plus size={12} />
            New Snippet
          </button>
          {userSnippets.length > 0 && (
            <button
              onClick={resetToDefaults}
              className="flex items-center gap-1 rounded-md px-2 py-1 text-[11px] text-muted-foreground hover:bg-accent"
              title="Reset to built-in snippets only"
            >
              <RotateCcw size={12} />
            </button>
          )}
        </div>
      </div>

      {(showNew || editingId) && (
        <SnippetForm
          initial={editingId ? allSnippets.find((s) => s.id === editingId) : undefined}
          onSave={(snippet) => {
            if (editingId) updateSnippet(snippet);
            else addSnippet(snippet);
            setShowNew(false);
            setEditingId(null);
          }}
          onCancel={() => { setShowNew(false); setEditingId(null); }}
        />
      )}

      <div className="space-y-1">
        {allSnippets.map((snippet) => {
          const isBuiltin = snippet.id.startsWith("builtin-");
          const isUserOverride = !isBuiltin;
          return (
            <div
              key={snippet.id}
              className="group flex items-start gap-3 rounded-md border border-border px-3 py-2"
            >
              <div className="flex-1 min-w-0">
                <div className="flex items-center gap-2">
                  <code className="rounded bg-muted px-1.5 py-0.5 text-[11px] font-semibold text-primary">
                    {snippet.prefix}
                  </code>
                  <span className="text-xs font-medium">{snippet.name}</span>
                  {isBuiltin && (
                    <span className="rounded bg-muted px-1 py-0.5 text-[9px] text-muted-foreground">built-in</span>
                  )}
                  {isUserOverride && (
                    <span className="rounded bg-primary/10 px-1 py-0.5 text-[9px] text-primary">custom</span>
                  )}
                </div>
                {snippet.description && (
                  <p className="mt-0.5 text-[10px] text-muted-foreground">{snippet.description}</p>
                )}
                <pre className="mt-1 overflow-x-auto rounded bg-muted/50 px-2 py-1 text-[10px] font-mono text-foreground/70 leading-relaxed">
                  {snippet.body}
                </pre>
              </div>
              {isUserOverride && (
                <div className="flex shrink-0 gap-0.5 opacity-0 transition-opacity group-hover:opacity-100">
                  <button
                    onClick={() => { setEditingId(snippet.id); setShowNew(false); }}
                    className="rounded p-1 text-muted-foreground hover:bg-accent hover:text-foreground"
                  >
                    <Pencil size={12} />
                  </button>
                  <button
                    onClick={() => deleteSnippet(snippet.id)}
                    className="rounded p-1 text-destructive/70 hover:bg-accent hover:text-destructive"
                  >
                    <Trash2 size={12} />
                  </button>
                </div>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}

function SnippetForm({
  initial,
  onSave,
  onCancel,
}: {
  initial?: SqlSnippet;
  onSave: (snippet: SqlSnippet) => void;
  onCancel: () => void;
}) {
  const [prefix, setPrefix] = useState(initial?.prefix ?? "");
  const [name, setName] = useState(initial?.name ?? "");
  const [body, setBody] = useState(initial?.body ?? "");
  const [description, setDescription] = useState(initial?.description ?? "");

  return (
    <div className="rounded-md border border-border bg-muted/30 p-3 space-y-2">
      <div className="flex gap-2">
        <div className="w-24">
          <label className="text-[10px] font-medium text-muted-foreground">Prefix *</label>
          <input
            value={prefix}
            onChange={(e) => setPrefix(e.target.value)}
            placeholder="sel"
            className="input mt-0.5 h-7 w-full text-xs"
            autoFocus
          />
        </div>
        <div className="flex-1">
          <label className="text-[10px] font-medium text-muted-foreground">Name *</label>
          <input
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="SELECT statement"
            className="input mt-0.5 h-7 w-full text-xs"
          />
        </div>
      </div>
      <div>
        <label className="text-[10px] font-medium text-muted-foreground">Description</label>
        <input
          value={description}
          onChange={(e) => setDescription(e.target.value)}
          placeholder="Optional description"
          className="input mt-0.5 h-7 w-full text-xs"
        />
      </div>
      <div>
        <label className="text-[10px] font-medium text-muted-foreground">
          Body * <span className="font-normal">(use $1, $2 for tab stops)</span>
        </label>
        <textarea
          value={body}
          onChange={(e) => setBody(e.target.value)}
          placeholder={"SELECT ${1:*}\nFROM ${2:table};"}
          className="input mt-0.5 w-full font-mono text-xs leading-relaxed"
          rows={4}
        />
      </div>
      <div className="flex gap-1">
        <button
          onClick={() => {
            if (!prefix.trim() || !name.trim() || !body.trim()) return;
            onSave({
              id: initial?.id ?? crypto.randomUUID(),
              prefix: prefix.trim(),
              name: name.trim(),
              body,
              description: description.trim() || undefined,
            });
          }}
          disabled={!prefix.trim() || !name.trim() || !body.trim()}
          className="rounded bg-primary px-3 py-1 text-[11px] font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-40"
        >
          {initial ? "Update" : "Add"}
        </button>
        <button
          onClick={onCancel}
          className="rounded px-3 py-1 text-[11px] text-muted-foreground hover:bg-accent"
        >
          Cancel
        </button>
      </div>
    </div>
  );
}

// ── About / Releases Tab ───────────────────────────────────

interface ReleaseSection {
  title: string;
  items: string[];
}

interface Release {
  version: string;
  sections: ReleaseSection[];
}

// Shared release data — also used by the portal site
import RELEASES_JSON from "../../releases.json";
const RELEASES: Release[] = RELEASES_JSON;

function AboutTab() {
  return (
    <div className="space-y-6">
      {/* Hero image */}
      <div className="overflow-hidden rounded-lg">
        <img
          src={new URL("../assets/about-hero.jpg", import.meta.url).href}
          alt="SQaiL – database tables and queries flowing in a spiral"
          className="w-full h-auto object-cover"
        />
      </div>

      {/* App header */}
      <div className="flex items-baseline gap-3">
        <h3 className="text-lg font-bold text-foreground">SQaiL</h3>
        <span className="rounded-full bg-primary/15 px-2 py-0.5 text-xs font-medium text-primary">
          v{__APP_VERSION__}
        </span>
        <span className="text-[10px] text-muted-foreground">
          Build {__BUILD_NUMBER__}
        </span>
      </div>
      <p className="text-xs text-muted-foreground">
        A lightweight, cross-platform desktop SQL database editor with AI integration.
      </p>

      <div className="h-px bg-border" />

      {/* Release history */}
      {RELEASES.map((release) => (
        <div key={release.version}>
          <h4 className="mb-3 flex items-center gap-2 text-sm font-semibold text-foreground">
            v{release.version}
            {release.version === __APP_VERSION__ && (
              <span className="rounded bg-success/15 px-1.5 py-0.5 text-[10px] font-medium text-success">
                current
              </span>
            )}
          </h4>
          <div className="space-y-3 pl-3 border-l-2 border-border">
            {release.sections.map((section) => (
              <div key={section.title}>
                <h5 className="mb-1 text-xs font-semibold text-foreground">
                  {section.title}
                </h5>
                <ul className="space-y-0.5">
                  {section.items.map((item, i) => (
                    <li
                      key={i}
                      className="flex items-start gap-1.5 text-[11px] leading-relaxed text-muted-foreground"
                    >
                      <span className="mt-1.5 h-1 w-1 shrink-0 rounded-full bg-muted-foreground/40" />
                      {item}
                    </li>
                  ))}
                </ul>
              </div>
            ))}
          </div>
        </div>
      ))}
    </div>
  );
}

/** Display-friendly formatting of a shortcut string. */
function formatShortcut(shortcut: string): string {
  return shortcut
    .replace(/Ctrl/g, "Ctrl")
    .replace(/Shift/g, "Shift")
    .replace(/Alt/g, "Alt")
    .replace(/Meta/g, "Cmd");
}
