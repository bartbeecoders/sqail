import { useState, useRef, useCallback } from "react";
import { Columns2 } from "lucide-react";
import type { editor as monacoEditor } from "monaco-editor";
import { cn } from "../lib/utils";
import { useAiStore } from "../stores/aiStore";
import EditorTabs from "./EditorTabs";
import SqlEditor from "./SqlEditor";
import SplitEditorPane from "./SplitEditorPane";
import DiffPreview from "./DiffPreview";

interface EditorAreaProps {
  onExecute?: (sql: string) => void;
  onFormat?: () => void;
}

export default function EditorArea({ onExecute, onFormat }: EditorAreaProps) {
  const [split, setSplit] = useState(false);
  const [splitRatio, setSplitRatio] = useState(0.5);
  const containerRef = useRef<HTMLDivElement>(null);
  const dragging = useRef(false);
  const primaryEditorRef = useRef<monacoEditor.IStandaloneCodeEditor | null>(null);
  const diffPreview = useAiStore((s) => s.diffPreview);
  const acceptDiff = useAiStore((s) => s.acceptDiffPreview);
  const rejectDiff = useAiStore((s) => s.closeDiffPreview);

  const toggleSplit = useCallback(() => {
    setSplit((prev) => !prev);
  }, []);

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

  // Diff preview mode: replace the editor entirely
  if (diffPreview) {
    return (
      <div className="flex flex-1 flex-col overflow-hidden">
        <DiffPreview
          original={diffPreview.original}
          modified={diffPreview.modified}
          onAccept={acceptDiff}
          onReject={rejectDiff}
        />
      </div>
    );
  }

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
          <div style={{ flex: `${splitRatio} 1 0%` }} className="flex min-w-0 flex-col overflow-hidden">
            <SqlEditor onExecute={onExecute} onFormat={onFormat} editorRefOut={primaryEditorRef} />
          </div>
          <div
            onMouseDown={onResizeStart}
            className="flex w-1.5 shrink-0 cursor-col-resize items-center justify-center bg-border/50 hover:bg-primary/30 transition-colors"
          >
            <div className="h-8 w-0.5 rounded-full bg-muted-foreground/30" />
          </div>
          <div style={{ flex: `${1 - splitRatio} 1 0%` }} className="min-w-0 overflow-hidden">
            <SplitEditorPane primaryEditorRef={primaryEditorRef} />
          </div>
        </div>
      ) : (
        <SqlEditor onExecute={onExecute} onFormat={onFormat} editorRefOut={primaryEditorRef} />
      )}
    </div>
  );
}
