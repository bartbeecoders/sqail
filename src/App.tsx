import { useState, useCallback, useEffect } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { format as formatSql } from "sql-formatter";
import Sidebar from "./components/Sidebar";
import Toolbar from "./components/Toolbar";
import EditorArea from "./components/EditorArea";
import ResultsPane from "./components/ResultsPane";
import ResizablePanel from "./components/ResizablePanel";
import { useEditorStore } from "./stores/editorStore";
import { useConnectionStore } from "./stores/connectionStore";
import { useQueryStore } from "./stores/queryStore";

export default function App() {
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false);

  useEffect(() => {
    getCurrentWindow().setTitle(
      `SQaiL v${__APP_VERSION__} (${__BUILD_NUMBER__})`,
    );
  }, []);
  const activeConnectionId = useConnectionStore((s) => s.activeConnectionId);
  const loading = useQueryStore((s) => s.loading);
  const executeQuery = useQueryStore((s) => s.executeQuery);

  const handleExecute = useCallback(
    (sql: string) => {
      if (!activeConnectionId || !sql.trim()) return;
      executeQuery(activeConnectionId, sql);
    },
    [activeConnectionId, executeQuery],
  );

  const handleFormat = useCallback(() => {
    const state = useEditorStore.getState();
    const tab = state.getActiveTab();
    if (!tab || !tab.content.trim()) return;
    try {
      const formatted = formatSql(tab.content, {
        language: "sql",
        tabWidth: 2,
        keywordCase: "upper",
      });
      state.setContent(tab.id, formatted);
    } catch {
      // If formatting fails, leave content unchanged
    }
  }, []);

  const handleClear = useCallback(() => {
    useEditorStore.getState().clearActiveTab();
  }, []);

  const handleRunFromToolbar = useCallback(() => {
    const tab = useEditorStore.getState().getActiveTab();
    if (tab && tab.content.trim()) {
      handleExecute(tab.content.trim());
    }
  }, [handleExecute]);

  return (
    <div className="flex h-full">
      <Sidebar
        collapsed={sidebarCollapsed}
        onToggle={() => setSidebarCollapsed(!sidebarCollapsed)}
      />
      <div className="flex flex-1 flex-col overflow-hidden">
        <Toolbar
          onRun={handleRunFromToolbar}
          onFormat={handleFormat}
          onClear={handleClear}
          hasConnection={!!activeConnectionId}
          loading={loading}
        />
        <ResizablePanel
          top={<EditorArea onExecute={handleExecute} onFormat={handleFormat} />}
          bottom={<ResultsPane />}
          defaultRatio={0.6}
        />
      </div>
    </div>
  );
}
