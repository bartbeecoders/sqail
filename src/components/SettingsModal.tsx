import { useState, useEffect, useRef, useCallback } from "react";
import { X, RotateCcw, Settings, Keyboard, Info } from "lucide-react";
import { cn } from "../lib/utils";
import { useShortcutStore } from "../stores/shortcutStore";
import {
  SHORTCUT_ACTIONS,
  CATEGORY_LABELS,
  eventToShortcut,
} from "../types/shortcuts";

interface SettingsModalProps {
  onClose: () => void;
  initialTab?: SettingsTab;
}

type SettingsTab = "general" | "shortcuts" | "about";

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
              active={activeTab === "shortcuts"}
              icon={<Keyboard size={14} />}
              label="Keyboard Shortcuts"
              onClick={() => setActiveTab("shortcuts")}
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
            {activeTab === "shortcuts" && <ShortcutsTab />}
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
  return (
    <div className="space-y-6">
      <section>
        <h3 className="mb-2 text-xs font-semibold uppercase tracking-wider text-muted-foreground">
          Editor
        </h3>
        <p className="text-xs text-muted-foreground">
          Editor preferences (font size, font family, tab size, word wrap) will be configurable here in a future update.
        </p>
      </section>
      <section>
        <h3 className="mb-2 text-xs font-semibold uppercase tracking-wider text-muted-foreground">
          Appearance
        </h3>
        <p className="text-xs text-muted-foreground">
          Theme and accent color settings will be configurable here in a future update. Currently follows system dark/light preference.
        </p>
      </section>
      <section>
        <h3 className="mb-2 text-xs font-semibold uppercase tracking-wider text-muted-foreground">
          Query Execution
        </h3>
        <p className="text-xs text-muted-foreground">
          Default row limit, query timeout, and startup behavior will be configurable here in a future update.
        </p>
      </section>
    </div>
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

// ── About / Releases Tab ───────────────────────────────────

interface ReleaseSection {
  title: string;
  items: string[];
}

interface Release {
  version: string;
  sections: ReleaseSection[];
}

const RELEASES: Release[] = [
  {
    version: "0.2.0",
    sections: [
      {
        title: "AI Integration",
        items: [
          "AI assistant sidebar panel with 4 flows: Generate SQL, Explain Query, Optimize Query, Generate Documentation",
          "7 AI provider types: Claude, OpenAI, Minimax, Z.ai, Claude Code CLI, LM Studio, OpenAI Compatible",
          "Streaming responses via SSE with real-time output in the sidebar",
          "Schema context injection — automatically includes active database schema in AI prompts",
          "Prompt history with persistence (capped at 100 entries)",
          "AI provider settings: add, edit, delete, set default, test connection",
          "Insert AI-generated SQL directly into the editor",
          "Monaco editor context menu: AI Explain Query and AI Optimize Query",
        ],
      },
      {
        title: "Keyboard Shortcuts",
        items: [
          "Global keyboard shortcut system with configurable bindings",
          "Default shortcuts: F5 (Run), Ctrl+S (Save), Ctrl+O (Open), Ctrl+Shift+S (Save As), Ctrl+Shift+N (New Connection), Ctrl+Shift+A (Toggle AI), Ctrl+N (New Tab), Ctrl+W (Close Tab), Ctrl+Shift+F (Format)",
          "Shortcuts persisted in localStorage, survive app restarts",
        ],
      },
      {
        title: "File Operations",
        items: [
          "Save query to .sql file (Ctrl+S) with native file dialog",
          "Save As (Ctrl+Shift+S) always prompts for file location",
          "Open .sql file (Ctrl+O) into current or new editor tab",
          "Tab title updates to reflect the saved filename",
        ],
      },
      {
        title: "Settings Page",
        items: [
          "Settings modal accessible via toolbar gear icon or Ctrl+,",
          "General tab for future editor/appearance/execution preferences",
          "Keyboard Shortcuts tab with full shortcut configuration",
          "About / Releases tab with version history",
        ],
      },
    ],
  },
  {
    version: "0.1.0",
    sections: [
      {
        title: "Project Foundation",
        items: [
          "Tauri v2 desktop app with React + TypeScript + Vite frontend",
          "Tailwind CSS + shadcn/ui design system with dark/light theme (system preference)",
          "Collapsible sidebar, resizable editor/results split panel layout",
        ],
      },
      {
        title: "Database Connections",
        items: [
          "Support for PostgreSQL, MySQL, SQLite, and SQL Server (MSSQL)",
          "Create, edit, delete, and test database connections",
          "Multiple simultaneous connections with independent pools",
          "Color-coded connection indicators, active state tracking",
        ],
      },
      {
        title: "SQL Editor",
        items: [
          "Monaco editor with SQL language mode and custom dark/light themes",
          "SQL keyword autocompletion (90+ keywords, functions, types)",
          "Schema-aware completions: table and column names from connected database",
          "Multi-tab editor with persistence in localStorage",
          "SQL formatting via sql-formatter",
        ],
      },
      {
        title: "Query Execution & Results",
        items: [
          "Virtualized data grid (TanStack Table + TanStack Virtual) for large result sets",
          "Sortable columns, row numbers, typed value rendering (NULL, boolean, numbers)",
          "Multiple result sets from semicolon-separated statements",
          "Execution time tracking and error display",
        ],
      },
      {
        title: "Schema Browser",
        items: [
          "Tree view: Schema > Tables/Views > Columns with type, PK, nullable info",
          "Double-click or drag-and-drop tables to generate SELECT statements",
          "Right-click context menu: SELECT *, COUNT(*), DESCRIBE, DROP",
          "Auto-refresh on connection change, search/filter",
        ],
      },
    ],
  },
];

function AboutTab() {
  return (
    <div className="space-y-6">
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
