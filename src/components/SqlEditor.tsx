import { useRef, useCallback, useEffect, useState } from "react";
import Editor, { type OnMount, type BeforeMount } from "@monaco-editor/react";
import type { editor as monacoEditor } from "monaco-editor";
import { invoke } from "@tauri-apps/api/core";
import { useDarkMode } from "../hooks/useDarkMode";
import { useEditorStore } from "../stores/editorStore";
import { useConnectionStore } from "../stores/connectionStore";
import { useSchemaStore } from "../stores/schemaStore";
import { useAiStore } from "../stores/aiStore";
import { buildSchemaContext } from "../lib/schemaContext";
import { sqlaiDark, sqlaiLight } from "../lib/monacoThemes";
import { createSqlCompletionProvider } from "../lib/sqlCompletions";
import { buildSelectStatement } from "../lib/sqlGenerate";
import type { ColumnInfo } from "../types/schema";

interface SqlEditorProps {
  onExecute?: (sql: string) => void;
  onFormat?: () => void;
}

export default function SqlEditor({ onExecute, onFormat }: SqlEditorProps) {
  const isDark = useDarkMode();
  const editorRef = useRef<monacoEditor.IStandaloneCodeEditor | null>(null);
  const { activeTabId, tabs, setContent } = useEditorStore();
  const [dragOver, setDragOver] = useState(false);
  const activeTab = tabs.find((t) => t.id === activeTabId);

  const handleBeforeMount: BeforeMount = useCallback((monaco) => {
    monaco.editor.defineTheme("sqlai-dark", sqlaiDark);
    monaco.editor.defineTheme("sqlai-light", sqlaiLight);
    monaco.languages.registerCompletionItemProvider("sql", createSqlCompletionProvider());
  }, []);

  const handleMount: OnMount = useCallback(
    (editor, monaco) => {
      editorRef.current = editor;

      // Ctrl+Enter / Cmd+Enter → execute
      editor.addAction({
        id: "execute-query",
        label: "Execute Query",
        keybindings: [monaco.KeyMod.CtrlCmd | monaco.KeyCode.Enter],
        run: (ed) => {
          const selection = ed.getSelection();
          const model = ed.getModel();
          if (!model) return;
          const selectedText =
            selection && !selection.isEmpty() ? model.getValueInRange(selection) : model.getValue();
          onExecute?.(selectedText.trim());
        },
      });

      // Ctrl+Shift+F → format (we handle formatting externally)
      editor.addAction({
        id: "format-sql",
        label: "Format SQL",
        keybindings: [monaco.KeyMod.CtrlCmd | monaco.KeyMod.Shift | monaco.KeyCode.KeyF],
        run: () => {
          onFormat?.();
        },
      });

      // Ctrl+N → new tab
      editor.addAction({
        id: "new-tab",
        label: "New Tab",
        keybindings: [monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyN],
        run: () => {
          useEditorStore.getState().addTab();
        },
      });

      // Ctrl+W → close tab
      editor.addAction({
        id: "close-tab",
        label: "Close Tab",
        keybindings: [monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyW],
        run: () => {
          const state = useEditorStore.getState();
          state.closeTab(state.activeTabId);
        },
      });

      // AI context menu actions
      editor.addAction({
        id: "ai-explain-query",
        label: "AI: Explain Query",
        contextMenuGroupId: "ai",
        contextMenuOrder: 1,
        run: (ed) => {
          const selection = ed.getSelection();
          const model = ed.getModel();
          if (!model) return;
          const sql =
            selection && !selection.isEmpty() ? model.getValueInRange(selection) : model.getValue();
          if (!sql.trim()) return;
          const connStore = useConnectionStore.getState();
          const conn = connStore.connections.find((c) => c.id === connStore.activeConnectionId);
          const driver = conn?.driver ?? "";
          const schemaContext = buildSchemaContext();
          const ai = useAiStore.getState();
          ai.setPanel(true);
          ai.explainQuery(sql.trim(), schemaContext, driver);
        },
      });

      editor.addAction({
        id: "ai-optimize-query",
        label: "AI: Optimize Query",
        contextMenuGroupId: "ai",
        contextMenuOrder: 2,
        run: (ed) => {
          const selection = ed.getSelection();
          const model = ed.getModel();
          if (!model) return;
          const sql =
            selection && !selection.isEmpty() ? model.getValueInRange(selection) : model.getValue();
          if (!sql.trim()) return;
          const connStore = useConnectionStore.getState();
          const conn = connStore.connections.find((c) => c.id === connStore.activeConnectionId);
          const driver = conn?.driver ?? "";
          const schemaContext = buildSchemaContext();
          const ai = useAiStore.getState();
          ai.setPanel(true);
          ai.optimizeQuery(sql.trim(), schemaContext, driver);
        },
      });

      editor.focus();
    },
    [onExecute, onFormat],
  );

  const handleChange = useCallback(
    (value: string | undefined) => {
      if (activeTabId && value !== undefined) {
        setContent(activeTabId, value);
      }
    },
    [activeTabId, setContent],
  );

  // Expose editor ref for external formatting
  useEffect(() => {
    if (editorRef.current && activeTab) {
      const model = editorRef.current.getModel();
      if (model && model.getValue() !== activeTab.content) {
        model.setValue(activeTab.content);
      }
    }
  // eslint-disable-next-line react-hooks/exhaustive-deps -- only re-set model on tab switch, not content changes
  }, [activeTabId]);

  const handleDragOver = useCallback((e: React.DragEvent) => {
    if (e.dataTransfer.types.includes("application/sqlai-table")) {
      e.preventDefault();
      e.dataTransfer.dropEffect = "copy";
      setDragOver(true);
    }
  }, []);

  const handleDragLeave = useCallback(() => {
    setDragOver(false);
  }, []);

  const handleDrop = useCallback(
    async (e: React.DragEvent) => {
      setDragOver(false);
      const raw = e.dataTransfer.getData("application/sqlai-table");
      if (!raw) return;
      e.preventDefault();

      const { schemaName, tableName } = JSON.parse(raw) as {
        schemaName: string;
        tableName: string;
      };

      // Get the active connection's driver
      const connStore = useConnectionStore.getState();
      const conn = connStore.connections.find(
        (c) => c.id === connStore.activeConnectionId,
      );
      if (!conn) return;

      // Get columns — from store if already loaded, otherwise fetch
      const schemaStore = useSchemaStore.getState();
      const key = `${schemaName}.${tableName}`;
      let cols = schemaStore.columns[key];
      if (!cols) {
        try {
          cols = await invoke<ColumnInfo[]>("list_columns", {
            connectionId: conn.id,
            schemaName,
            tableName,
          });
          // Also cache them in the store
          schemaStore.loadColumns(conn.id, schemaName, tableName);
        } catch {
          cols = [];
        }
      }

      const sql = buildSelectStatement(schemaName, tableName, cols, conn.driver);

      // Insert at cursor position if editor is available
      const editor = editorRef.current;
      if (editor) {
        const position = editor.getPosition();
        const model = editor.getModel();
        if (model && position) {
          // If there's existing content, add a newline before
          const currentContent = model.getValue();
          const insertText = currentContent.length > 0 ? `\n${sql}` : sql;
          const endLine = model.getLineCount();
          const endCol = model.getLineMaxColumn(endLine);
          editor.executeEdits("drag-drop", [
            {
              range: {
                startLineNumber: endLine,
                startColumn: endCol,
                endLineNumber: endLine,
                endColumn: endCol,
              },
              text: insertText,
            },
          ]);
          editor.focus();
          return;
        }
      }

      // Fallback: append via store
      const state = useEditorStore.getState();
      const tab = state.getActiveTab();
      if (tab) {
        const newContent = tab.content ? `${tab.content}\n${sql}` : sql;
        state.setContent(tab.id, newContent);
      }
    },
    [],
  );

  return (
    <div
      className={`flex-1 overflow-hidden relative ${dragOver ? "ring-2 ring-inset ring-primary/40" : ""}`}
      onDragOver={handleDragOver}
      onDragLeave={handleDragLeave}
      onDrop={handleDrop}
    >
      {dragOver && (
        <div className="absolute inset-0 z-10 flex items-center justify-center bg-primary/5 pointer-events-none">
          <span className="rounded-md bg-background/90 px-3 py-1.5 text-xs font-medium text-muted-foreground shadow-sm border border-border">
            Drop to generate SELECT statement
          </span>
        </div>
      )}
      <Editor
        language="sql"
        theme={isDark ? "sqlai-dark" : "sqlai-light"}
        value={activeTab?.content ?? ""}
        onChange={handleChange}
        beforeMount={handleBeforeMount}
        onMount={handleMount}
        options={{
          fontSize: 14,
          fontFamily: "'JetBrains Mono', 'Fira Code', 'Cascadia Code', Consolas, monospace",
          lineNumbers: "on",
          minimap: { enabled: false },
          wordWrap: "on",
          scrollBeyondLastLine: false,
          automaticLayout: true,
          tabSize: 2,
          suggestOnTriggerCharacters: true,
          quickSuggestions: true,
          padding: { top: 8, bottom: 8 },
          renderLineHighlight: "line",
          smoothScrolling: true,
          cursorBlinking: "smooth",
          cursorSmoothCaretAnimation: "on",
          bracketPairColorization: { enabled: true },
          autoClosingBrackets: "always",
          autoClosingQuotes: "always",
          matchBrackets: "always",
        }}
      />
    </div>
  );
}
