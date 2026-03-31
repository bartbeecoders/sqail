import { useState, useCallback, useEffect, useMemo } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { formatSqlAligned } from "./lib/sqlFormat";
import { saveQuery, saveQueryAs, openQuery } from "./lib/fileOps";
import Sidebar from "./components/Sidebar";
import Toolbar from "./components/Toolbar";
import EditorArea from "./components/EditorArea";
import ResultsPane from "./components/ResultsPane";
import ResizablePanel from "./components/ResizablePanel";
import AiPanel from "./components/AiPanel";
import SettingsModal from "./components/SettingsModal";
import { useEditorStore } from "./stores/editorStore";
import { useConnectionStore } from "./stores/connectionStore";
import { useQueryStore } from "./stores/queryStore";
import { useAiStore } from "./stores/aiStore";
import { useAiStream } from "./hooks/useAiStream";
import { useMetadataEvents } from "./hooks/useMetadataEvents";
import { useGlobalShortcuts, type ShortcutHandlers } from "./hooks/useGlobalShortcuts";

function loadUiState<T>(key: string, fallback: T): T {
  try {
    const raw = localStorage.getItem(key);
    if (raw !== null) return JSON.parse(raw) as T;
  } catch { /* ignore */ }
  return fallback;
}

export default function App() {
  const [sidebarCollapsed, setSidebarCollapsed] = useState(() => loadUiState("sqail_sidebar_collapsed", false));
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [connectionFormOpen, setConnectionFormOpen] = useState(false);
  const aiPanelOpen = useAiStore((s) => s.panelOpen);

  // Restore AI panel state on mount
  useEffect(() => {
    const saved = loadUiState("sqail_ai_panel_open", false);
    if (saved) useAiStore.getState().setPanel(true);
  }, []);

  useAiStream();
  useMetadataEvents();

  // Persist sidebar collapsed state
  useEffect(() => {
    localStorage.setItem("sqail_sidebar_collapsed", JSON.stringify(sidebarCollapsed));
  }, [sidebarCollapsed]);

  // Persist AI panel open state
  useEffect(() => {
    localStorage.setItem("sqail_ai_panel_open", JSON.stringify(aiPanelOpen));
  }, [aiPanelOpen]);

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
      const formatted = formatSqlAligned(tab.content);
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

  // Global shortcut handlers
  const shortcutHandlers: ShortcutHandlers = useMemo(
    () => ({
      "run-query": handleRunFromToolbar,
      "format-query": handleFormat,
      "new-tab": () => useEditorStore.getState().addTab(),
      "close-tab": () => {
        const state = useEditorStore.getState();
        state.closeTab(state.activeTabId);
      },
      "save-query": () => { saveQuery().catch(console.error); },
      "open-query": () => { openQuery().catch(console.error); },
      "save-query-as": () => { saveQueryAs().catch(console.error); },
      "new-connection": () => setConnectionFormOpen(true),
      "toggle-ai-panel": () => useAiStore.getState().togglePanel(),
      "open-settings": () => setSettingsOpen(true),
    }),
    [handleRunFromToolbar, handleFormat],
  );

  useGlobalShortcuts(shortcutHandlers);

  return (
    <div className="flex h-full">
      <Sidebar
        collapsed={sidebarCollapsed}
        onToggle={() => setSidebarCollapsed(!sidebarCollapsed)}
        externalFormOpen={connectionFormOpen}
        onExternalFormClose={() => setConnectionFormOpen(false)}
      />
      <div className="flex flex-1 flex-col overflow-hidden">
        <Toolbar
          onRun={handleRunFromToolbar}
          onFormat={handleFormat}
          onClear={handleClear}
          hasConnection={!!activeConnectionId}
          loading={loading}
          onOpenSettings={() => setSettingsOpen(true)}
        />
        <ResizablePanel
          top={<EditorArea onExecute={handleExecute} onFormat={handleFormat} />}
          bottom={<ResultsPane />}
          defaultRatio={0.6}
        />
      </div>
      {aiPanelOpen && <AiPanel />}
      {settingsOpen && <SettingsModal onClose={() => setSettingsOpen(false)} />}
    </div>
  );
}
