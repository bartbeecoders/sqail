import { useMemo } from "react";
import { Zap } from "lucide-react";

import { cn } from "../lib/utils";
import { useInlineAiStore } from "../stores/inlineAiStore";

/**
 * Toolbar widget that summarises inline-AI status at a glance. Click
 * opens the Inline AI settings tab.
 *
 * States:
 *   - disabled   → muted
 *   - starting   → amber pulse
 *   - ready      → emerald
 *   - error      → red
 */
export default function InlineAiIndicator({
  onOpen,
}: {
  onOpen: () => void;
}) {
  const enabled = useInlineAiStore((s) => s.enabled);
  const sidecar = useInlineAiStore((s) => s.sidecar);

  const { dotClass, label } = useMemo(() => {
    if (!enabled) return { dotClass: "bg-muted-foreground/40", label: "Inline AI off" };
    switch (sidecar.state) {
      case "ready":
        return { dotClass: "bg-emerald-500", label: `Inline AI ready (${sidecar.modelId})` };
      case "starting":
        return { dotClass: "bg-amber-500 animate-pulse", label: "Inline AI starting…" };
      case "error":
        return { dotClass: "bg-red-500", label: `Inline AI error: ${sidecar.message}` };
      case "stopped":
      default:
        return { dotClass: "bg-muted-foreground/40", label: "Inline AI stopped" };
    }
  }, [enabled, sidecar]);

  return (
    <button
      onClick={onOpen}
      title={label}
      aria-label={label}
      className="flex items-center gap-1.5 rounded-md px-2 py-1.5 text-xs font-medium text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground"
    >
      <span className="relative inline-flex">
        <Zap size={14} />
        <span
          className={cn(
            "absolute -bottom-0.5 -right-0.5 h-2 w-2 rounded-full border border-background",
            dotClass,
          )}
        />
      </span>
    </button>
  );
}
