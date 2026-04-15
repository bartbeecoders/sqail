import { useCallback } from "react";
import { DiffEditor, type BeforeMount } from "@monaco-editor/react";
import { Check, X } from "lucide-react";
import { useDarkMode } from "../hooks/useDarkMode";
import { useSettingsStore } from "../stores/settingsStore";
import { sqlaiDark, sqlaiLight } from "../lib/monacoThemes";

interface DiffPreviewProps {
  original: string;
  modified: string;
  onAccept: () => void;
  onReject: () => void;
}

export default function DiffPreview({ original, modified, onAccept, onReject }: DiffPreviewProps) {
  const isDark = useDarkMode();
  const fontSize = useSettingsStore((s) => s.editorFontSize);
  const fontFamily = useSettingsStore((s) => s.editorFontFamily);

  const handleBeforeMount: BeforeMount = useCallback((monaco) => {
    monaco.editor.defineTheme("sqlai-dark", sqlaiDark);
    monaco.editor.defineTheme("sqlai-light", sqlaiLight);
  }, []);

  return (
    <div className="flex flex-1 flex-col overflow-hidden">
      {/* Toolbar */}
      <div className="flex items-center justify-between border-b border-border bg-muted/30 px-3 py-1.5">
        <span className="text-xs font-medium text-muted-foreground">
          AI Format Preview
        </span>
        <div className="flex items-center gap-1.5">
          <button
            onClick={onReject}
            className="flex items-center gap-1 rounded-md px-2.5 py-1 text-xs font-medium text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground"
          >
            <X size={12} />
            Reject
          </button>
          <button
            onClick={onAccept}
            className="flex items-center gap-1 rounded-md bg-primary px-2.5 py-1 text-xs font-medium text-primary-foreground transition-colors hover:bg-primary/90"
          >
            <Check size={12} />
            Accept
          </button>
        </div>
      </div>

      {/* Diff editor */}
      <div className="flex-1 overflow-hidden">
        <DiffEditor
          original={original}
          modified={modified}
          language="sql"
          theme={isDark ? "sqlai-dark" : "sqlai-light"}
          beforeMount={handleBeforeMount}
          options={{
            fontSize,
            fontFamily,
            readOnly: true,
            renderSideBySide: true,
            scrollBeyondLastLine: false,
            automaticLayout: true,
            minimap: { enabled: false },
            lineNumbers: "on",
            padding: { top: 8, bottom: 8 },
            smoothScrolling: true,
            renderIndicators: true,
            originalEditable: false,
          }}
        />
      </div>
    </div>
  );
}
