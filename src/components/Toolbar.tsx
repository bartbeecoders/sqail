import { Play, AlignLeft, Trash2, Sparkles, Loader2, Settings } from "lucide-react";
import { cn } from "../lib/utils";
import { useAiStore } from "../stores/aiStore";

interface ToolbarProps {
  onRun?: () => void;
  onFormat?: () => void;
  onClear?: () => void;
  hasConnection?: boolean;
  loading?: boolean;
  onOpenSettings?: () => void;
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

export default function Toolbar({ onRun, onFormat, onClear, hasConnection, loading, onOpenSettings }: ToolbarProps) {
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
      <ToolbarButton
        icon={<AlignLeft size={14} />}
        label="Format"
        shortcut="Ctrl+Shift+F"
        onClick={onFormat}
      />
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
        onClick={onOpenSettings}
        className="flex items-center gap-1.5 rounded-md px-2 py-1.5 text-xs font-medium text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground"
        title="Settings (Ctrl+,)"
      >
        <Settings size={14} />
      </button>
    </div>
  );
}
