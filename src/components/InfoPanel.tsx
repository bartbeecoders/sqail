import { useState, useCallback, useEffect, useRef } from "react";
import { X, BookOpen, ScrollText } from "lucide-react";
import { cn } from "../lib/utils";
import MetadataPanel from "./MetadataPanel";
import MetadataLogPanel from "./MetadataLogPanel";

type InfoTab = "metadata" | "log";

const MIN_W = 220;
const MAX_W = 600;
const DEFAULT_W = 288;

interface InfoPanelProps {
  onClose: () => void;
}

export default function InfoPanel({ onClose }: InfoPanelProps) {
  const [activeTab, setActiveTab] = useState<InfoTab>("metadata");
  const dragging = useRef(false);

  const [width, setWidth] = useState<number>(() => {
    try {
      const saved = localStorage.getItem("sqail_info_panel_width");
      if (saved) return Math.max(MIN_W, Math.min(Number(saved), MAX_W));
    } catch { /* ignore */ }
    return DEFAULT_W;
  });

  useEffect(() => {
    localStorage.setItem("sqail_info_panel_width", String(width));
  }, [width]);

  const onResizeStart = useCallback((e: React.MouseEvent) => {
    e.preventDefault();
    dragging.current = true;
    const startX = e.clientX;
    const startW = width;
    const onMove = (ev: MouseEvent) => {
      if (!dragging.current) return;
      const delta = startX - ev.clientX;
      setWidth(Math.max(MIN_W, Math.min(startW + delta, MAX_W)));
    };
    const onUp = () => {
      dragging.current = false;
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
      document.removeEventListener("mousemove", onMove);
      document.removeEventListener("mouseup", onUp);
    };
    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";
    document.addEventListener("mousemove", onMove);
    document.addEventListener("mouseup", onUp);
  }, [width]);

  return (
    <aside
      className="relative flex flex-col border-l border-border bg-muted/50"
      style={{ width }}
    >
      {/* Resize handle */}
      <div
        onMouseDown={onResizeStart}
        className="absolute left-0 top-0 bottom-0 w-1 cursor-col-resize hover:bg-primary/30 active:bg-primary/50 z-10"
      />

      {/* Header */}
      <div className="flex items-center justify-between border-b border-border px-3 py-2">
        <div className="flex gap-1">
          {([
            { id: "metadata" as const, icon: BookOpen, label: "Metadata" },
            { id: "log" as const, icon: ScrollText, label: "Log" },
          ]).map(({ id, icon: Icon, label }) => (
            <button
              key={id}
              onClick={() => setActiveTab(id)}
              className={cn(
                "flex items-center gap-1 rounded-md px-2 py-1 text-[10px] font-medium transition-colors",
                activeTab === id
                  ? "bg-primary text-primary-foreground"
                  : "text-muted-foreground hover:bg-accent hover:text-accent-foreground",
              )}
            >
              <Icon size={11} />
              {label}
            </button>
          ))}
        </div>
        <button
          onClick={onClose}
          className="rounded p-1 text-muted-foreground hover:bg-accent hover:text-accent-foreground"
          title="Close panel"
        >
          <X size={14} />
        </button>
      </div>

      {/* Content */}
      <div className="flex-1 overflow-y-auto">
        {activeTab === "metadata" && <MetadataPanel />}
        {activeTab === "log" && <MetadataLogPanel />}
      </div>
    </aside>
  );
}
