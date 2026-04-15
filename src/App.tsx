import { useState, useCallback, useEffect, useMemo } from "react";
import { formatSqlAligned, formatSqlWithComments } from "./lib/sqlFormat";
import type { FormatOptions } from "./lib/sqlFormat";
import { useMetadataStore } from "./stores/metadataStore";
import { useSettingsStore } from "./stores/settingsStore";
import { saveQuery, saveQueryAs, openQuery } from "./lib/fileOps";
import TitleBar from "./components/TitleBar";
import ResizeHandles from "./components/ResizeHandles";
import SplashScreen from "./components/SplashScreen";
import Sidebar from "./components/Sidebar";
import Toolbar from "./components/Toolbar";
import EditorArea from "./components/EditorArea";
import ResultsPane from "./components/ResultsPane";
import ResizablePanel from "./components/ResizablePanel";
import AiPanel from "./components/AiPanel";
import AiCommandPalette from "./components/AiCommandPalette";
import InfoPanel from "./components/InfoPanel";
import SettingsModal from "./components/SettingsModal";
import UpdateChecker from "./components/UpdateChecker";
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
  const [splashDone, setSplashDone] = useState(false);
  const [sidebarCollapsed, setSidebarCollapsed] = useState(() => loadUiState("sqail_sidebar_collapsed", false));
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [connectionFormOpen, setConnectionFormOpen] = useState(false);
  const [infoPanelOpen, setInfoPanelOpen] = useState(() => loadUiState("sqail_info_panel_open", false));
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

  // Persist info panel open state
  useEffect(() => {
    localStorage.setItem("sqail_info_panel_open", JSON.stringify(infoPanelOpen));
  }, [infoPanelOpen]);

  const activeConnectionId = useConnectionStore((s) => s.activeConnectionId);
  const activeTabId = useEditorStore((s) => s.activeTabId);
  const loading = useQueryStore((s) => s.loading);
  const executeQuery = useQueryStore((s) => s.executeQuery);

  // When the active tab changes, sync the global active connection to match the tab's connection
  useEffect(() => {
    const tab = useEditorStore.getState().getActiveTab();
    if (!tab?.connectionId) return;
    const connStore = useConnectionStore.getState();
    if (tab.connectionId === connStore.activeConnectionId) return;

    if (connStore.connectedIds.has(tab.connectionId)) {
      // Already connected — just switch the active pointer (no backend call)
      connStore.setActive(tab.connectionId);
    } else {
      // Not connected yet — establish the connection
      connStore.connect(tab.connectionId).catch(() => {
        // Connection config may have been deleted — ignore
      });
    }
  }, [activeTabId]);

  // When the user connects to a new connection, assign it to the active tab if it has none
  useEffect(() => {
    if (!activeConnectionId) return;
    const tab = useEditorStore.getState().getActiveTab();
    if (tab && !tab.connectionId) {
      useEditorStore.getState().setConnectionId(tab.id, activeConnectionId);
    }
  }, [activeConnectionId]);

  const handleExecute = useCallback(
    (sql: string) => {
      // Use the active tab's connectionId for execution
      const tab = useEditorStore.getState().getActiveTab();
      const connId = tab?.connectionId ?? activeConnectionId;
      if (!connId || !sql.trim()) return;
      executeQuery(connId, sql);
    },
    [activeConnectionId, executeQuery],
  );

  const handleFormat = useCallback(() => {
    const state = useEditorStore.getState();
    const tab = state.getActiveTab();
    if (!tab || !tab.content.trim()) return;
    try {
      const settings = useSettingsStore.getState();
      const fmtOpts: FormatOptions = {
        indent: settings.formatIndent,
        uppercaseKeywords: settings.formatUppercaseKeywords,
        andOrNewLine: settings.formatAndOrNewLine,
      };
      const formatted = formatSqlAligned(tab.content, fmtOpts);
      state.setContent(tab.id, formatted);
    } catch {
      // If formatting fails, leave content unchanged
    }
  }, []);

  const handleFormatWithComments = useCallback(() => {
    const state = useEditorStore.getState();
    const tab = state.getActiveTab();
    if (!tab || !tab.content.trim()) return;
    try {
      const metaStore = useMetadataStore.getState();
      const settings = useSettingsStore.getState();
      const fmtOpts: FormatOptions = {
        indent: settings.formatIndent,
        uppercaseKeywords: settings.formatUppercaseKeywords,
        andOrNewLine: settings.formatAndOrNewLine,
      };
      const formatted = formatSqlWithComments(tab.content, metaStore, fmtOpts);
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
      "new-tab": () => {
        const connId = useConnectionStore.getState().activeConnectionId;
        useEditorStore.getState().addTab();
        if (connId) {
          const tab = useEditorStore.getState().getActiveTab();
          if (tab) useEditorStore.getState().setConnectionId(tab.id, connId);
        }
      },
      "close-tab": () => {
        const state = useEditorStore.getState();
        state.closeTab(state.activeTabId);
      },
      "save-query": () => { saveQuery().catch(console.error); },
      "open-query": () => { openQuery().catch(console.error); },
      "save-query-as": () => { saveQueryAs().catch(console.error); },
      "new-connection": () => setConnectionFormOpen(true),
      "open-ai-palette": () => useAiStore.getState().openPalette(),
      "toggle-ai-panel": () => useAiStore.getState().togglePanel(),
      "open-settings": () => setSettingsOpen(true),
    }),
    [handleRunFromToolbar, handleFormat],
  );

  useGlobalShortcuts(shortcutHandlers);

  return (
    <div className="flex h-full flex-col">
      <TitleBar />
      <UpdateChecker />
      <ResizeHandles />
      {!splashDone && <SplashScreen onComplete={() => setSplashDone(true)} />}
      <div className="flex flex-1 overflow-hidden">
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
          onFormatWithComments={handleFormatWithComments}
          onClear={handleClear}
          hasConnection={!!(useEditorStore.getState().getActiveTab()?.connectionId ?? activeConnectionId)}
          loading={loading}
          onOpenSettings={() => setSettingsOpen(true)}
          infoPanelOpen={infoPanelOpen}
          onToggleInfoPanel={() => setInfoPanelOpen(!infoPanelOpen)}
        />
        <ResizablePanel
          top={<EditorArea onExecute={handleExecute} onFormat={handleFormat} />}
          bottom={<ResultsPane />}
          defaultRatio={0.6}
        />
      </div>
      {infoPanelOpen && <InfoPanel onClose={() => setInfoPanelOpen(false)} />}
      {aiPanelOpen && <AiPanel />}
      <AiCommandPalette />
      {settingsOpen && <SettingsModal onClose={() => setSettingsOpen(false)} />}
      </div>
    </div>
  );
}
