import { useState, useRef, useCallback } from "react";
import { Columns2, X } from "lucide-react";
import { cn } from "../lib/utils";
import { useEditorStore } from "../stores/editorStore";
import EditorTabs from "./EditorTabs";
import SqlEditor from "./SqlEditor";

interface EditorAreaProps {
  onExecute?: (sql: string) => void;
  onFormat?: () => void;
}

export default function EditorArea({ onExecute, onFormat }: EditorAreaProps) {
  const [split, setSplit] = useState(false);
  const [splitTabId, setSplitTabId] = useState<string | null>(null);
  const [splitRatio, setSplitRatio] = useState(0.5);
  const containerRef = useRef<HTMLDivElement>(null);
  const dragging = useRef(false);
  const tabs = useEditorStore((s) => s.tabs);

  // When enabling split, default to a different tab if possible
  const toggleSplit = useCallback(() => {
    if (split) {
      setSplit(false);
      setSplitTabId(null);
    } else {
      const state = useEditorStore.getState();
      const other = state.tabs.find((t) => t.id !== state.activeTabId);
      setSplitTabId(other?.id ?? state.activeTabId);
      setSplit(true);
    }
  }, [split]);

  const onResizeStart = useCallback((e: React.MouseEvent) => {
    e.preventDefault();
    dragging.current = true;

    const onMove = (ev: MouseEvent) => {
      if (!dragging.current || !containerRef.current) return;
      const rect = containerRef.current.getBoundingClientRect();
      const x = ev.clientX - rect.left;
      const clamped = Math.max(0.2, Math.min(x / rect.width, 0.8));
      setSplitRatio(clamped);
    };

    const onUp = () => {
      dragging.current = false;
      document.removeEventListener("mousemove", onMove);
      document.removeEventListener("mouseup", onUp);
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
    };

    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";
    document.addEventListener("mousemove", onMove);
    document.addEventListener("mouseup", onUp);
  }, []);

  return (
    <div className="flex flex-1 flex-col overflow-hidden">
      <div className="flex items-center">
        <div className="flex-1">
          <EditorTabs />
        </div>
        <button
          onClick={toggleSplit}
          className={cn(
            "mr-1 rounded p-1 transition-colors",
            split
              ? "bg-primary/10 text-primary"
              : "text-muted-foreground hover:bg-accent hover:text-foreground",
          )}
          title={split ? "Close split view" : "Split editor"}
        >
          <Columns2 size={14} />
        </button>
      </div>

      {split ? (
        <div ref={containerRef} className="flex flex-1 overflow-hidden">
          <div style={{ flex: `${splitRatio} 1 0%` }} className="min-w-0 overflow-hidden">
            <SqlEditor onExecute={onExecute} onFormat={onFormat} />
          </div>
          <div
            onMouseDown={onResizeStart}
            className="flex w-1.5 shrink-0 cursor-col-resize items-center justify-center bg-border/50 hover:bg-primary/30 transition-colors"
          >
            <div className="h-8 w-0.5 rounded-full bg-muted-foreground/30" />
          </div>
          <div style={{ flex: `${1 - splitRatio} 1 0%` }} className="flex min-w-0 flex-col overflow-hidden">
            {/* Split pane tab selector */}
            <div className="flex h-7 shrink-0 items-center gap-px overflow-x-auto border-b border-border bg-muted/30 px-1">
              {tabs.map((tab) => (
                <button
                  key={tab.id}
                  onClick={() => setSplitTabId(tab.id)}
                  className={cn(
                    "rounded px-2 py-0.5 text-[10px] transition-colors",
                    tab.id === splitTabId
                      ? "bg-background text-foreground border border-border"
                      : "text-muted-foreground hover:text-foreground hover:bg-muted",
                  )}
                >
                  {tab.title}
                </button>
              ))}
              <button
                onClick={() => setSplit(false)}
                className="ml-auto rounded p-0.5 text-muted-foreground hover:bg-muted hover:text-foreground"
                title="Close split"
              >
                <X size={10} />
              </button>
            </div>
            <SqlEditor
              onExecute={onExecute}
              onFormat={onFormat}
              overrideTabId={splitTabId ?? undefined}
            />
          </div>
        </div>
      ) : (
        <SqlEditor onExecute={onExecute} onFormat={onFormat} />
      )}
    </div>
  );
}
