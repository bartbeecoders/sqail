import { useState, useCallback, useEffect, useMemo } from "react";
import { formatSqlAligned, formatSqlWithComments } from "./lib/sqlFormat";
import type { FormatOptions } from "./lib/sqlFormat";
import { useMetadataStore } from "./stores/metadataStore";
import { useSettingsStore } from "./stores/settingsStore";
import { saveQuery, saveQueryAs, openQuery } from "./lib/fileOps";
import { validateQuery } from "./lib/validateQuery";
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
import SettingsModal, { type SettingsTab } from "./components/SettingsModal";
import UpdateChecker from "./components/UpdateChecker";
import { useEditorStore, getActiveEditorInstance } from "./stores/editorStore";
import { useConnectionStore } from "./stores/connectionStore";
import { useQueryStore } from "./stores/queryStore";
import { useAiStore } from "./stores/aiStore";
import { useAiStream } from "./hooks/useAiStream";
import { useMetadataEvents } from "./hooks/useMetadataEvents";
import { useInlineAiLifecycle } from "./hooks/useInlineAiLifecycle";
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
  const [settingsOpen, setSettingsOpen] = useState<SettingsTab | null>(null);
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
  useInlineAiLifecycle();

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
  const activeTabKind = useEditorStore(
    (s) => s.tabs.find((t) => t.id === s.activeTabId)?.kind,
  );
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

  const [validateStatus, setValidateStatus] = useState<
    | { kind: "idle" }
    | { kind: "validating" }
    | { kind: "ok"; note?: string }
    | { kind: "error"; message: string }
  >({ kind: "idle" });

  const handleValidate = useCallback(async () => {
    // Use selection if there is one, otherwise full tab content.
    const editor = getActiveEditorInstance();
    let sql = "";
    if (editor) {
      const selection = editor.getSelection();
      const model = editor.getModel();
      if (model) {
        sql =
          selection && !selection.isEmpty()
            ? model.getValueInRange(selection)
            : model.getValue();
      }
    }
    if (!sql.trim()) {
      const tab = useEditorStore.getState().getActiveTab();
      sql = tab?.content ?? "";
    }
    if (!sql.trim()) return;

    const tab = useEditorStore.getState().getActiveTab();
    const connId = tab?.connectionId ?? activeConnectionId;
    if (!connId) {
      setValidateStatus({ kind: "error", message: "No active connection" });
      return;
    }
    setValidateStatus({ kind: "validating" });
    try {
      const result = await validateQuery(connId, sql.trim());
      if (result.ok) {
        setValidateStatus({ kind: "ok", note: result.note ?? undefined });
      } else {
        setValidateStatus({ kind: "error", message: result.error ?? "Invalid SQL" });
      }
    } catch (e) {
      setValidateStatus({ kind: "error", message: String(e) });
    }
  }, [activeConnectionId]);

  // Auto-dismiss the validation toast after a few seconds.
  useEffect(() => {
    if (validateStatus.kind === "idle" || validateStatus.kind === "validating") return;
    const t = setTimeout(() => setValidateStatus({ kind: "idle" }), 4500);
    return () => clearTimeout(t);
  }, [validateStatus]);

  const handleRunFromToolbar = useCallback(() => {
    // If the editor has selected text, run only that; otherwise run full content
    const editor = getActiveEditorInstance();
    if (editor) {
      const selection = editor.getSelection();
      const model = editor.getModel();
      if (model) {
        const text =
          selection && !selection.isEmpty()
            ? model.getValueInRange(selection)
            : model.getValue();
        if (text.trim()) {
          handleExecute(text.trim());
          return;
        }
      }
    }
    // Fallback: use stored tab content
    const tab = useEditorStore.getState().getActiveTab();
    if (tab && tab.content.trim()) {
      handleExecute(tab.content.trim());
    }
  }, [handleExecute]);

  // Global shortcut handlers
  const shortcutHandlers: ShortcutHandlers = useMemo(
    () => ({
      "run-query": handleRunFromToolbar,
      "validate-query": handleValidate,
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
      "open-settings": () => setSettingsOpen("general"),
    }),
    [handleRunFromToolbar, handleFormat, handleValidate],
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
          onValidate={handleValidate}
          validating={validateStatus.kind === "validating"}
          onFormat={handleFormat}
          onFormatWithComments={handleFormatWithComments}
          onClear={handleClear}
          hasConnection={!!(useEditorStore.getState().getActiveTab()?.connectionId ?? activeConnectionId)}
          loading={loading}
          onOpenSettings={(tab) => setSettingsOpen(tab ?? "general")}
          infoPanelOpen={infoPanelOpen}
          onToggleInfoPanel={() => setInfoPanelOpen(!infoPanelOpen)}
        />
        {(validateStatus.kind === "ok" || validateStatus.kind === "error") && (
          <div
            className={`fixed right-4 top-14 z-50 max-w-md rounded-md border px-3 py-2 text-xs shadow-lg ${
              validateStatus.kind === "ok"
                ? "border-success/40 bg-success/10 text-success"
                : "border-destructive/40 bg-destructive/10 text-destructive"
            }`}
          >
            {validateStatus.kind === "ok"
              ? validateStatus.note
                ? `SQL validation skipped: ${validateStatus.note}`
                : "SQL is valid."
              : `Invalid SQL: ${validateStatus.message}`}
          </div>
        )}
        {activeTabKind === "diagram" ? (
          <EditorArea onExecute={handleExecute} onFormat={handleFormat} />
        ) : (
          <ResizablePanel
            top={<EditorArea onExecute={handleExecute} onFormat={handleFormat} />}
            bottom={<ResultsPane />}
            defaultRatio={0.6}
          />
        )}
      </div>
      {infoPanelOpen && <InfoPanel onClose={() => setInfoPanelOpen(false)} />}
      {aiPanelOpen && <AiPanel />}
      <AiCommandPalette />
      {settingsOpen && (
        <SettingsModal
          initialTab={settingsOpen}
          onClose={() => setSettingsOpen(null)}
        />
      )}
      </div>
    </div>
  );
}
