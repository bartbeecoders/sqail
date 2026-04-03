import { create } from "zustand";
import type { EditorTab } from "../types/editor";

const STORAGE_KEY = "sqail_tabs";

function generateId(): string {
  return crypto.randomUUID();
}

function createTab(index: number): EditorTab {
  return { id: generateId(), title: `Query ${index}`, content: "" };
}

function loadTabs(): { tabs: EditorTab[]; activeTabId: string } {
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
  closeTab: (id: string) => void;
  setActiveTab: (id: string) => void;
  renameTab: (id: string, title: string) => void;
  setContent: (id: string, content: string) => void;
  setFilePath: (id: string, filePath: string) => void;
  getActiveTab: () => EditorTab | undefined;
  clearActiveTab: () => void;
}

export const useEditorStore = create<EditorState>((set, get) => {
  const initial = loadTabs();

  const persist = () => {
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

    closeTab: (id) => {
      const { tabs, activeTabId } = get();
      if (tabs.length <= 1) return; // always keep at least one tab
      const filtered = tabs.filter((t) => t.id !== id);
      const newActive =
        activeTabId === id ? filtered[Math.max(0, tabs.findIndex((t) => t.id === id) - 1)].id : activeTabId;
      set({ tabs: filtered, activeTabId: newActive });
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
