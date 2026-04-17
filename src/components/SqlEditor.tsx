import { useRef, useCallback, useEffect, useState } from "react";
import Editor, { type OnMount, type BeforeMount } from "@monaco-editor/react";
import type { editor as monacoEditor } from "monaco-editor";
import { invoke } from "@tauri-apps/api/core";
import { useDarkMode } from "../hooks/useDarkMode";
import { useEditorStore, setActiveEditorInstance } from "../stores/editorStore";
import { useConnectionStore } from "../stores/connectionStore";
import { useSchemaStore } from "../stores/schemaStore";
import { useAiStore } from "../stores/aiStore";
import { useSettingsStore } from "../stores/settingsStore";
import { sqlaiDark, sqlaiLight } from "../lib/monacoThemes";
import { createSqlCompletionProvider } from "../lib/sqlCompletions";
import { createInlineAiProvider } from "../lib/inlineAi";
import { buildSelectStatement, buildRoutineCallStatement } from "../lib/sqlGenerate";
import { validateSql, toMonacoMarkers } from "../lib/sqlValidator";
import type { ColumnInfo } from "../types/schema";
import type { Driver } from "../types/connection";

/** Map database driver to Monaco language ID */
function driverToLanguage(driver: Driver | ""): string {
  switch (driver) {
    case "postgres": return "pgsql";
    case "mysql": return "mysql";
    default: return "sql"; // mssql, sqlite, no connection
  }
}

interface SqlEditorProps {
  onExecute?: (sql: string) => void;
  onFormat?: () => void;
  overrideTabId?: string;
  /** When true, expose the editor ref so a split pane can share its model. */
  editorRefOut?: React.MutableRefObject<monacoEditor.IStandaloneCodeEditor | null>;
}

export default function SqlEditor({ onExecute, onFormat, overrideTabId, editorRefOut }: SqlEditorProps) {
  const isDark = useDarkMode();
  const editorRef = useRef<monacoEditor.IStandaloneCodeEditor | null>(null);
  const { activeTabId: storeActiveTabId, tabs, setContent } = useEditorStore();
  const activeTabId = overrideTabId ?? storeActiveTabId;
  const fontSize = useSettingsStore((s) => s.editorFontSize);
  const fontFamily = useSettingsStore((s) => s.editorFontFamily);
  const tabSize = useSettingsStore((s) => s.editorTabSize);
  const minimap = useSettingsStore((s) => s.editorMinimap);
  const wordWrap = useSettingsStore((s) => s.editorWordWrap);
  const lineNumbers = useSettingsStore((s) => s.editorLineNumbers);
  const [dragOver, setDragOver] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const monacoRef = useRef<any>(null);
  const validationTimer = useRef<ReturnType<typeof setTimeout>>(undefined);
  const activeTab = tabs.find((t) => t.id === activeTabId);

  const activeConnectionId = useConnectionStore((s) => s.activeConnectionId);
  const connections = useConnectionStore((s) => s.connections);
  const activeConn = connections.find((c) => c.id === activeConnectionId);
  const driver: Driver | "" = activeConn?.driver ?? "";
  const monacoLanguage = driverToLanguage(driver);

  const handleBeforeMount: BeforeMount = useCallback((monaco) => {
    monaco.editor.defineTheme("sqlai-dark", sqlaiDark);
    monaco.editor.defineTheme("sqlai-light", sqlaiLight);
    const completionProvider = createSqlCompletionProvider();
    const inlineProvider = createInlineAiProvider();
    for (const lang of ["sql", "mysql", "pgsql"]) {
      monaco.languages.registerCompletionItemProvider(lang, completionProvider);
      monaco.languages.registerInlineCompletionsProvider(lang, inlineProvider);
    }
  }, []);

  const runValidation = useCallback(() => {
    const editor = editorRef.current;
    const monaco = monacoRef.current;
    if (!editor || !monaco) return;
    const model = editor.getModel();
    if (!model) return;
    const text = model.getValue();
    if (!text.trim()) {
      monaco.editor.setModelMarkers(model, "sql-validator", []);
      return;
    }
    const connStore = useConnectionStore.getState();
    const conn = connStore.connections.find((c) => c.id === connStore.activeConnectionId);
    const currentDriver = conn?.driver ?? "";
    const errors = validateSql(text, currentDriver);
    const markers = toMonacoMarkers(errors);
    monaco.editor.setModelMarkers(model, "sql-validator", markers);
  }, []);

  const handleMount: OnMount = useCallback(
    (editor, monaco) => {
      editorRef.current = editor;
      monacoRef.current = monaco;
      if (editorRefOut) editorRefOut.current = editor;
      setActiveEditorInstance(editor);

      // Clean up the global ref when this editor is disposed
      editor.onDidDispose(() => setActiveEditorInstance(null));

      // Initial validation
      setTimeout(() => runValidation(), 100);

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

      // AI context menu actions — open command palette with pre-set flow
      const getEditorSql = (ed: import("monaco-editor").editor.ICodeEditor) => {
        const selection = ed.getSelection();
        const model = ed.getModel();
        if (!model) return "";
        return (
          selection && !selection.isEmpty()
            ? model.getValueInRange(selection)
            : model.getValue()
        ).trim();
      };

      editor.addAction({
        id: "ai-explain-query",
        label: "AI: Explain Query",
        contextMenuGroupId: "ai",
        contextMenuOrder: 1,
        run: (ed) => {
          const sql = getEditorSql(ed);
          if (!sql) return;
          useAiStore.getState().openPalette({ flow: "explain", sql });
        },
      });

      editor.addAction({
        id: "ai-optimize-query",
        label: "AI: Optimize Query",
        contextMenuGroupId: "ai",
        contextMenuOrder: 2,
        run: (ed) => {
          const sql = getEditorSql(ed);
          if (!sql) return;
          useAiStore.getState().openPalette({ flow: "optimize", sql });
        },
      });

      editor.addAction({
        id: "ai-format-query",
        label: "AI: Format Query",
        contextMenuGroupId: "ai",
        contextMenuOrder: 3,
        run: (ed) => {
          const sql = getEditorSql(ed);
          if (!sql) return;
          useAiStore.getState().openPalette({ flow: "format_sql", sql });
        },
      });

      editor.addAction({
        id: "ai-comment-query",
        label: "AI: Add Comments",
        contextMenuGroupId: "ai",
        contextMenuOrder: 4,
        run: (ed) => {
          const sql = getEditorSql(ed);
          if (!sql) return;
          useAiStore.getState().openPalette({ flow: "comment_sql", sql });
        },
      });

      editor.focus();
    },
    [onExecute, onFormat, editorRefOut, runValidation],
  );

  const handleChange = useCallback(
    (value: string | undefined) => {
      if (activeTabId && value !== undefined) {
        setContent(activeTabId, value);
      }
      // Debounced validation
      if (validationTimer.current) clearTimeout(validationTimer.current);
      validationTimer.current = setTimeout(() => {
        runValidation();
      }, 500);
    },
    [activeTabId, setContent, runValidation],
  );

  // Expose editor ref for external formatting + re-validate on tab switch
  useEffect(() => {
    if (editorRef.current && activeTab) {
      const model = editorRef.current.getModel();
      if (model && model.getValue() !== activeTab.content) {
        model.setValue(activeTab.content);
      }
    }
    // Re-validate when switching tabs
    setTimeout(() => runValidation(), 50);
  // eslint-disable-next-line react-hooks/exhaustive-deps -- only re-set model on tab switch, not content changes
  }, [activeTabId]);

  // Re-validate when the database driver changes (dialect-specific checks)
  useEffect(() => {
    setTimeout(() => runValidation(), 50);
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [driver]);

  // Ctrl + Mouse Wheel → zoom editor font size (issue #10)
  // Must attach to the editor's DOM directly and use capture phase
  // because Monaco intercepts wheel events internally.
  useEffect(() => {
    const editor = editorRef.current;
    if (!editor) return;
    const dom = editor.getDomNode();
    if (!dom) return;
    const handleWheel = (e: WheelEvent) => {
      if (!e.ctrlKey) return;
      e.preventDefault();
      e.stopPropagation();
      const raw = e.deltaY;
      if (raw === 0) return;
      const delta = raw > 0 ? -1 : 1;
      const current = useSettingsStore.getState().editorFontSize;
      const next = Math.min(40, Math.max(8, current + delta));
      if (next !== current) {
        useSettingsStore.getState().updateSetting("editorFontSize", next);
      }
    };
    dom.addEventListener("wheel", handleWheel, { passive: false, capture: true });
    return () => dom.removeEventListener("wheel", handleWheel, { capture: true });
  });

  const handleDragOver = useCallback((e: React.DragEvent) => {
    if (e.dataTransfer.types.includes("application/sqlai-table") || e.dataTransfer.types.includes("application/sqlai-routine")) {
      e.preventDefault();
      e.dataTransfer.dropEffect = "copy";
      setDragOver(true);
    }
  }, []);

  const handleDragLeave = useCallback(() => {
    setDragOver(false);
  }, []);

  const insertSqlInEditor = useCallback((sql: string) => {
    const editor = editorRef.current;
    if (editor) {
      const model = editor.getModel();
      if (model) {
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
  }, []);

  const handleDrop = useCallback(
    async (e: React.DragEvent) => {
      setDragOver(false);

      const connStore = useConnectionStore.getState();
      const conn = connStore.connections.find(
        (c) => c.id === connStore.activeConnectionId,
      );
      if (!conn) return;

      // Handle table drop
      const tableRaw = e.dataTransfer.getData("application/sqlai-table");
      if (tableRaw) {
        e.preventDefault();
        const { schemaName, tableName } = JSON.parse(tableRaw) as {
          schemaName: string;
          tableName: string;
        };

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
            schemaStore.loadColumns(conn.id, schemaName, tableName);
          } catch {
            cols = [];
          }
        }

        insertSqlInEditor(buildSelectStatement(schemaName, tableName, cols, conn.driver));
        return;
      }

      // Handle routine drop
      const routineRaw = e.dataTransfer.getData("application/sqlai-routine");
      if (routineRaw) {
        e.preventDefault();
        const { schemaName, routineName, routineType } = JSON.parse(routineRaw) as {
          schemaName: string;
          routineName: string;
          routineType: "procedure" | "function";
        };

        const dropAction = useSettingsStore.getState().routineDropAction;
        if (dropAction === "definition") {
          try {
            const definition = await invoke<string>("get_routine_definition", {
              connectionId: conn.id,
              schemaName,
              routineName,
              routineType,
            });
            insertSqlInEditor(definition);
          } catch {
            insertSqlInEditor(buildRoutineCallStatement(schemaName, routineName, routineType, conn.driver));
          }
        } else {
          insertSqlInEditor(buildRoutineCallStatement(schemaName, routineName, routineType, conn.driver));
        }
        return;
      }
    },
    [insertSqlInEditor],
  );

  return (
    <div
      ref={containerRef}
      className={`flex-1 overflow-hidden relative ${dragOver ? "ring-2 ring-inset ring-primary/40" : ""}`}
      onDragOver={handleDragOver}
      onDragLeave={handleDragLeave}
      onDrop={handleDrop}
    >
      {dragOver && (
        <div className="absolute inset-0 z-10 flex items-center justify-center bg-primary/5 pointer-events-none">
          <span className="rounded-md bg-background/90 px-3 py-1.5 text-xs font-medium text-muted-foreground shadow-sm border border-border">
            Drop to generate SQL statement
          </span>
        </div>
      )}
      <Editor
        language={monacoLanguage}
        theme={isDark ? "sqlai-dark" : "sqlai-light"}
        value={activeTab?.content ?? ""}
        onChange={handleChange}
        beforeMount={handleBeforeMount}
        onMount={handleMount}
        options={{
          fontSize,
          fontFamily,
          lineNumbers: lineNumbers ? "on" : "off",
          minimap: { enabled: minimap },
          wordWrap: wordWrap ? "on" : "off",
          scrollBeyondLastLine: false,
          automaticLayout: true,
          tabSize,
          suggestOnTriggerCharacters: true,
          quickSuggestions: true,
          inlineSuggest: { enabled: true },
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
