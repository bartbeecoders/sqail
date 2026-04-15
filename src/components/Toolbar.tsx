import { useState, useRef, useEffect, useCallback } from "react";
import { Play, AlignLeft, Trash2, Sparkles, Loader2, Settings, ChevronDown, MessageSquareText, BookOpen, Wand2 } from "lucide-react";
import { cn } from "../lib/utils";
import { useAiStore } from "../stores/aiStore";
import { useEditorStore } from "../stores/editorStore";

interface ToolbarProps {
  onRun?: () => void;
  onFormat?: () => void;
  onFormatWithComments?: () => void;
  onClear?: () => void;
  hasConnection?: boolean;
  loading?: boolean;
  onOpenSettings?: () => void;
  infoPanelOpen?: boolean;
  onToggleInfoPanel?: () => void;
}

interface ToolbarButtonProps {
  icon: React.ReactNode;
  label: string;
  shortcut?: string;
  disabled?: boolean;
  variant?: "default" | "primary";
  onClick?: () => void;
}

function ToolbarButton({
  icon,
  label,
  shortcut,
  disabled = false,
  variant = "default",
  onClick,
}: ToolbarButtonProps) {
  return (
    <button
      disabled={disabled}
      onClick={onClick}
      className={cn(
        "flex items-center gap-1.5 rounded-md px-2.5 py-1.5 text-xs font-medium transition-colors disabled:opacity-40 disabled:cursor-not-allowed",
        variant === "primary"
          ? "bg-primary text-primary-foreground hover:bg-primary/90"
          : "text-foreground hover:bg-accent",
      )}
      title={shortcut ? `${label} (${shortcut})` : label}
    >
      {icon}
      <span>{label}</span>
    </button>
  );
}

export default function Toolbar({ onRun, onFormat, onFormatWithComments, onClear, hasConnection, loading, onOpenSettings, infoPanelOpen, onToggleInfoPanel }: ToolbarProps) {
  const openPalette = useAiStore((s) => s.openPalette);

  return (
    <div className="flex h-10 items-center gap-1 border-b border-border bg-muted/30 px-2">
      <ToolbarButton
        icon={loading ? <Loader2 size={14} className="animate-spin" /> : <Play size={14} />}
        label={loading ? "Running..." : "Run"}
        shortcut="F5 / Ctrl+Enter"
        variant="primary"
        onClick={onRun}
        disabled={!hasConnection || loading}
      />
      <FormatSplitButton onFormat={onFormat} onFormatWithComments={onFormatWithComments} />
      <ToolbarButton
        icon={<Trash2 size={14} />}
        label="Clear"
        onClick={onClear}
      />
      <div className="mx-1 h-4 w-px bg-border" />
      <button
        onClick={() => openPalette()}
        className="flex items-center gap-1.5 rounded-md px-2.5 py-1.5 text-xs font-medium text-foreground transition-colors hover:bg-accent"
        title="AI Command Palette (Ctrl+K)"
      >
        <Sparkles size={14} />
        <span>AI</span>
      </button>

      {/* Spacer to push settings to the right */}
      <div className="flex-1" />

      <button
        onClick={onToggleInfoPanel}
        className={cn(
          "flex items-center gap-1.5 rounded-md px-2 py-1.5 text-xs font-medium transition-colors",
          infoPanelOpen
            ? "bg-accent text-accent-foreground"
            : "text-muted-foreground hover:bg-accent hover:text-accent-foreground",
        )}
        title="Metadata & Log"
      >
        <BookOpen size={14} />
      </button>
      <button
        onClick={onOpenSettings}
        className="flex items-center gap-1.5 rounded-md px-2 py-1.5 text-xs font-medium text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground"
        title="Settings (Ctrl+,)"
      >
        <Settings size={14} />
      </button>
    </div>
  );
}

function FormatSplitButton({
  onFormat,
  onFormatWithComments,
}: {
  onFormat?: () => void;
  onFormatWithComments?: () => void;
}) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  const close = useCallback(() => setOpen(false), []);

  useEffect(() => {
    if (!open) return;
    const onMouseDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) close();
    };
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") close();
    };
    document.addEventListener("mousedown", onMouseDown);
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("mousedown", onMouseDown);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [open, close]);

  return (
    <div ref={ref} className="relative flex items-center">
      <button
        onClick={onFormat}
        className="flex items-center gap-1.5 rounded-l-md px-2.5 py-1.5 text-xs font-medium text-foreground transition-colors hover:bg-accent"
        title="Format (Ctrl+Shift+F)"
      >
        <AlignLeft size={14} />
        <span>Format</span>
      </button>
      <button
        onClick={() => setOpen((v) => !v)}
        className="flex items-center rounded-r-md px-1 py-1.5 text-xs text-foreground transition-colors hover:bg-accent border-l border-border/50"
        title="Format options"
      >
        <ChevronDown size={12} />
      </button>
      {open && (
        <div className="absolute left-0 top-full z-50 mt-1 min-w-48 rounded-md border border-border bg-background py-1 shadow-lg">
          <button
            onClick={() => {
              onFormat?.();
              close();
            }}
            className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-xs hover:bg-accent hover:text-accent-foreground"
          >
            <AlignLeft size={12} />
            Format
          </button>
          <button
            onClick={() => {
              onFormatWithComments?.();
              close();
            }}
            className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-xs hover:bg-accent hover:text-accent-foreground"
          >
            <MessageSquareText size={12} />
            Format with Comments
          </button>
          <div className="my-1 h-px bg-border" />
          <button
            onClick={() => {
              const tab = useEditorStore.getState().getActiveTab();
              const sql = tab?.content?.trim();
              if (sql) {
                useAiStore.getState().openPalette({ flow: "format_sql", sql });
              }
              close();
            }}
            className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-xs hover:bg-accent hover:text-accent-foreground"
          >
            <Wand2 size={12} />
            Format with AI
          </button>
        </div>
      )}
    </div>
  );
}
