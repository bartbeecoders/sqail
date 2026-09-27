import { create } from "zustand";
import type { editor as monacoEditor } from "monaco-editor";
import type { EditorTab } from "../types/editor";
import type { SqailPromptEntry } from "../types/sqailFile";
import { defaultDiagramState, type DiagramState } from "../types/diagram";
import { consumeHandoffTab, isDetachedWindow, tabStorageKey } from "../lib/detach";

const PROMPT_RECENCY_LIMIT = 10;

const STORAGE_KEY = tabStorageKey();
const DETACHED = isDetachedWindow();

// Global ref to the active Monaco editor instance — used by the F5 shortcut
// handler to read selected text without threading refs through the component tree.
let _activeEditor: monacoEditor.IStandaloneCodeEditor | null = null;

export function setActiveEditorInstance(ed: monacoEditor.IStandaloneCodeEditor | null) {
  _activeEditor = ed;
}

export function getActiveEditorInstance(): monacoEditor.IStandaloneCodeEditor | null {
  return _activeEditor;
}

function generateId(): string {
  return crypto.randomUUID();
}

function createTab(index: number): EditorTab {
  return { id: generateId(), title: `Query ${index}`, content: "" };
}

function loadTabs(): { tabs: EditorTab[]; activeTabId: string } {
  // Detached windows boot from a one-shot handoff payload and are otherwise
  // in-memory only — closing the window destroys the tab.
  if (DETACHED) {
    const handoff = consumeHandoffTab();
    if (handoff) return { tabs: [handoff], activeTabId: handoff.id };
    const tab = createTab(1);
    return { tabs: [tab], activeTabId: tab.id };
  }
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw) {
      const data = JSON.parse(raw) as { tabs: EditorTab[]; activeTabId: string };
      if (data.tabs.length > 0) return data;
    }
  } catch {
    // ignore
  }
  const tab = createTab(1);
  return { tabs: [tab], activeTabId: tab.id };
}

interface EditorState {
  tabs: EditorTab[];
  activeTabId: string;

  addTab: () => void;
  addTabWithContent: (title: string, content: string) => void;
  addDiagramTab: (title: string, schemaName: string, connectionId?: string) => void;
  /** Add a fully-specified tab (used when restoring from a `.sqail` file). */
  addRestoredTab: (descriptor: Omit<EditorTab, "id">) => EditorTab;
  updateDiagram: (id: string, updater: (d: DiagramState) => DiagramState) => void;
  reorderTabs: (fromId: string, toId: string, side: "before" | "after") => void;
  closeTab: (id: string) => void;
  closeOtherTabs: (id: string) => void;
  closeTabsToRight: (id: string) => void;
  closeAllTabs: () => void;
  setActiveTab: (id: string) => void;
  renameTab: (id: string, title: string) => void;
  setContent: (id: string, content: string) => void;
  setFilePath: (id: string, filePath: string) => void;
  setConnectionId: (id: string, connectionId: string | undefined) => void;
  setSavedQueryId: (id: string, savedQueryId: string) => void;
  togglePin: (id: string) => void;
  /** Append to the tab's palette recency (dedupes consecutive; caps at 10). */
  addPromptToTab: (id: string, prompt: string) => void;
  /** Append a rich AI exchange to the tab's per-file history. */
  appendAiHistoryEntry: (id: string, entry: SqailPromptEntry) => void;
  findTabBySavedQueryId: (savedQueryId: string) => EditorTab | undefined;
  getActiveTab: () => EditorTab | undefined;
  clearActiveTab: () => void;
}

export const useEditorStore = create<EditorState>((set, get) => {
  const initial = loadTabs();

  const persist = () => {
    // Detached windows are intentionally in-memory only — closing the window
    // discards the tab.
    if (DETACHED) return;
    const { tabs, activeTabId } = get();
    localStorage.setItem(STORAGE_KEY, JSON.stringify({ tabs, activeTabId }));
  };

  return {
    ...initial,

    addTab: () => {
      const { tabs } = get();
      const usedNumbers = tabs
        .map((t) => t.title.match(/^Query (\d+)$/))
        .filter(Boolean)
        .map((m) => Number(m![1]));
      const nextNumber = usedNumbers.length > 0 ? Math.max(...usedNumbers) + 1 : 1;
      const tab = createTab(nextNumber);
      set({ tabs: [...tabs, tab], activeTabId: tab.id });
      persist();
    },

    addTabWithContent: (title, content) => {
      const { tabs } = get();
      const tab: EditorTab = { id: generateId(), title, content };
      set({ tabs: [...tabs, tab], activeTabId: tab.id });
      persist();
    },

    addDiagramTab: (title, schemaName, connectionId) => {
      const { tabs } = get();
      const tab: EditorTab = {
        id: generateId(),
        title,
        content: "",
        kind: "diagram",
        diagram: defaultDiagramState(schemaName),
        connectionId,
      };
      set({ tabs: [...tabs, tab], activeTabId: tab.id });
      persist();
    },

    addRestoredTab: (descriptor) => {
      const { tabs } = get();
      const tab: EditorTab = { id: generateId(), ...descriptor };
      set({ tabs: [...tabs, tab], activeTabId: tab.id });
      persist();
      return tab;
    },

    updateDiagram: (id, updater) => {
      set((s) => ({
        tabs: s.tabs.map((t) => {
          if (t.id !== id || t.kind !== "diagram" || !t.diagram) return t;
          return { ...t, diagram: updater(t.diagram) };
        }),
      }));
      persist();
    },

    reorderTabs: (fromId, toId, side) => {
      if (fromId === toId) return;
      const { tabs } = get();
      const fromIdx = tabs.findIndex((t) => t.id === fromId);
      const toIdx = tabs.findIndex((t) => t.id === toId);
      if (fromIdx < 0 || toIdx < 0) return;
      const reordered = [...tabs];
      const [moving] = reordered.splice(fromIdx, 1);
      // Recompute the target index after removal.
      const anchor = reordered.findIndex((t) => t.id === toId);
      if (anchor < 0) return;
      const insertAt = side === "before" ? anchor : anchor + 1;
      reordered.splice(insertAt, 0, moving);
      set({ tabs: reordered });
      persist();
    },

    closeTab: (id) => {
      const { tabs, activeTabId } = get();
      if (tabs.length <= 1) return; // always keep at least one tab
      const filtered = tabs.filter((t) => t.id !== id);
      const newActive =
        activeTabId === id ? filtered[Math.max(0, tabs.findIndex((t) => t.id === id) - 1)].id : activeTabId;
      set({ tabs: filtered, activeTabId: newActive });
      persist();
    },

    closeOtherTabs: (id) => {
      const { tabs } = get();
      // Spare pinned tabs and the target tab itself
      const kept = tabs.filter((t) => t.id === id || t.pinned);
      if (kept.length === 0) return;
      set({ tabs: kept, activeTabId: id });
      persist();
    },

    closeTabsToRight: (id) => {
      const { tabs, activeTabId } = get();
      const idx = tabs.findIndex((t) => t.id === id);
      if (idx === -1) return;
      // Keep everything up to and including the target, plus any pinned tabs to the right
      const kept = tabs.filter((t, i) => i <= idx || t.pinned);
      const newActive = kept.find((t) => t.id === activeTabId) ? activeTabId : id;
      set({ tabs: kept, activeTabId: newActive });
      persist();
    },

    closeAllTabs: () => {
      const { tabs, activeTabId } = get();
      const pinned = tabs.filter((t) => t.pinned);
      if (pinned.length > 0) {
        const newActive = pinned.find((t) => t.id === activeTabId) ? activeTabId : pinned[0].id;
        set({ tabs: pinned, activeTabId: newActive });
      } else {
        const tab = createTab(1);
        set({ tabs: [tab], activeTabId: tab.id });
      }
      persist();
    },

    setActiveTab: (id) => {
      set({ activeTabId: id });
      persist();
    },

    renameTab: (id, title) => {
      set((s) => ({
        tabs: s.tabs.map((t) => (t.id === id ? { ...t, title } : t)),
      }));
      persist();
    },

    setContent: (id, content) => {
      set((s) => ({
        tabs: s.tabs.map((t) => (t.id === id ? { ...t, content } : t)),
      }));
      persist();
    },

    setFilePath: (id, filePath) => {
      set((s) => ({
        tabs: s.tabs.map((t) => (t.id === id ? { ...t, filePath } : t)),
      }));
      persist();
    },

    setConnectionId: (id, connectionId) => {
      set((s) => ({
        tabs: s.tabs.map((t) => (t.id === id ? { ...t, connectionId } : t)),
      }));
      persist();
    },

    setSavedQueryId: (id, savedQueryId) => {
      set((s) => ({
        tabs: s.tabs.map((t) => (t.id === id ? { ...t, savedQueryId } : t)),
      }));
      persist();
    },

    togglePin: (id) => {
      set((s) => ({
        tabs: s.tabs.map((t) => (t.id === id ? { ...t, pinned: !t.pinned } : t)),
      }));
      persist();
    },

    addPromptToTab: (id, prompt) => {
      set((s) => ({
        tabs: s.tabs.map((t) => {
          if (t.id !== id) return t;
          const existing = t.promptHistory ?? [];
          if (existing[existing.length - 1] === prompt) return t;
          const updated = [...existing, prompt].slice(-PROMPT_RECENCY_LIMIT);
          return { ...t, promptHistory: updated };
        }),
      }));
      persist();
    },

    appendAiHistoryEntry: (id, entry) => {
      set((s) => ({
        tabs: s.tabs.map((t) => {
          if (t.id !== id) return t;
          const existing = t.aiHistory ?? [];
          return { ...t, aiHistory: [...existing, entry] };
        }),
      }));
      persist();
    },

    findTabBySavedQueryId: (savedQueryId) => {
      return get().tabs.find((t) => t.savedQueryId === savedQueryId);
    },

    getActiveTab: () => {
      const { tabs, activeTabId } = get();
      return tabs.find((t) => t.id === activeTabId);
    },

    clearActiveTab: () => {
      const { activeTabId } = get();
      get().setContent(activeTabId, "");
    },
  };
});
