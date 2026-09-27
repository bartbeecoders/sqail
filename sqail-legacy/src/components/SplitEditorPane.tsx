import { useRef, useEffect, useCallback } from "react";
import type { editor as monacoEditor } from "monaco-editor";
import { loader } from "@monaco-editor/react";
import { useDarkMode } from "../hooks/useDarkMode";
import { useSettingsStore } from "../stores/settingsStore";

interface SplitEditorPaneProps {
  /** The primary editor whose model this pane mirrors. */
  primaryEditorRef: React.RefObject<monacoEditor.IStandaloneCodeEditor | null>;
}

/**
 * A lightweight secondary Monaco editor that shares the primary editor's text model.
 * Both panes show the same file content; edits in either pane are instantly reflected
 * in the other. Each pane has its own scroll position and cursor.
 */
export default function SplitEditorPane({ primaryEditorRef }: SplitEditorPaneProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const editorRef = useRef<monacoEditor.IStandaloneCodeEditor | null>(null);
  const isDark = useDarkMode();

  const fontSize = useSettingsStore((s) => s.editorFontSize);
  const fontFamily = useSettingsStore((s) => s.editorFontFamily);
  const tabSize = useSettingsStore((s) => s.editorTabSize);
  const minimap = useSettingsStore((s) => s.editorMinimap);
  const wordWrap = useSettingsStore((s) => s.editorWordWrap);
  const lineNumbers = useSettingsStore((s) => s.editorLineNumbers);

  const createEditor = useCallback(async () => {
    if (!containerRef.current) return;
    const primary = primaryEditorRef.current;
    if (!primary) return;

    const monaco = await loader.init();
    const model = primary.getModel();
    if (!model) return;

    // Don't recreate if already set to this model
    if (editorRef.current) {
      if (editorRef.current.getModel() === model) return;
      editorRef.current.dispose();
    }

    const ed = monaco.editor.create(containerRef.current, {
      model,
      theme: isDark ? "sqlai-dark" : "sqlai-light",
      fontSize,
      fontFamily,
      lineNumbers: lineNumbers ? "on" : "off",
      minimap: { enabled: minimap },
      wordWrap: wordWrap ? "on" : "off",
      scrollBeyondLastLine: false,
      automaticLayout: true,
      tabSize,
      readOnly: false,
      padding: { top: 8, bottom: 8 },
      renderLineHighlight: "line",
      smoothScrolling: true,
      cursorBlinking: "smooth",
      cursorSmoothCaretAnimation: "on",
      bracketPairColorization: { enabled: true },
      autoClosingBrackets: "always",
      autoClosingQuotes: "always",
      matchBrackets: "always",
    });

    editorRef.current = ed;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [primaryEditorRef]);

  // Create the editor once the primary editor and container are available
  useEffect(() => {
    // Small delay to ensure the primary editor is fully mounted
    const timer = setTimeout(() => createEditor(), 50);
    return () => clearTimeout(timer);
  }, [createEditor]);

  // Re-attach model when the primary editor's model changes (tab switch)
  useEffect(() => {
    const primary = primaryEditorRef.current;
    const secondary = editorRef.current;
    if (!primary || !secondary) return;
    const model = primary.getModel();
    if (model && secondary.getModel() !== model) {
      secondary.setModel(model);
    }
  });

  // Keep theme in sync
  useEffect(() => {
    loader.init().then((monaco) => {
      monaco.editor.setTheme(isDark ? "sqlai-dark" : "sqlai-light");
    });
  }, [isDark]);

  // Keep editor options in sync
  useEffect(() => {
    editorRef.current?.updateOptions({
      fontSize,
      fontFamily,
      tabSize,
      lineNumbers: lineNumbers ? "on" : "off",
      minimap: { enabled: minimap },
      wordWrap: wordWrap ? "on" : "off",
    });
  }, [fontSize, fontFamily, tabSize, lineNumbers, minimap, wordWrap]);

  // Cleanup
  useEffect(() => {
    return () => {
      editorRef.current?.dispose();
      editorRef.current = null;
    };
  }, []);

  return <div ref={containerRef} className="h-full w-full" />;
}
