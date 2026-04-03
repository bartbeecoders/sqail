import { Plus, X } from "lucide-react";
import { useRef, useState } from "react";
import { cn } from "../lib/utils";
import { useEditorStore } from "../stores/editorStore";

export default function EditorTabs() {
  const { tabs, activeTabId, addTab, closeTab, setActiveTab, renameTab } = useEditorStore();
  const [editingTabId, setEditingTabId] = useState<string | null>(null);
  const [editValue, setEditValue] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);

  const commitRename = (id: string) => {
    const trimmed = editValue.trim();
    if (trimmed) renameTab(id, trimmed);
    setEditingTabId(null);
  };

  return (
    <div className="flex h-8 items-end gap-px overflow-x-auto border-b border-border bg-muted/30 px-1">
      {tabs.map((tab) => {
        const isActive = tab.id === activeTabId;
        const isEditing = editingTabId === tab.id;
        return (
          <div
            key={tab.id}
            className={cn(
              "group flex cursor-pointer items-center gap-1 rounded-t-md px-3 py-1 text-xs transition-colors",
              isActive
                ? "bg-background text-foreground border-x border-t border-border"
                : "text-muted-foreground hover:text-foreground hover:bg-muted",
            )}
            onClick={() => setActiveTab(tab.id)}
            onDoubleClick={() => {
              setEditingTabId(tab.id);
              setEditValue(tab.title);
              requestAnimationFrame(() => inputRef.current?.select());
            }}
          >
            {isEditing ? (
              <input
                ref={inputRef}
                value={editValue}
                onChange={(e) => setEditValue(e.target.value)}
                onBlur={() => commitRename(tab.id)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") commitRename(tab.id);
                  if (e.key === "Escape") setEditingTabId(null);
                }}
                onClick={(e) => e.stopPropagation()}
                onDoubleClick={(e) => e.stopPropagation()}
                className="w-24 bg-transparent outline-none ring-1 ring-border rounded px-1"
                autoFocus
              />
            ) : (
              <span className="max-w-24 truncate">{tab.title}</span>
            )}
            {tabs.length > 1 && !isEditing && (
              <button
                onClick={(e) => {
                  e.stopPropagation();
                  closeTab(tab.id);
                }}
                className="ml-1 rounded p-0.5 opacity-0 transition-opacity group-hover:opacity-100 hover:bg-muted"
              >
                <X size={10} />
              </button>
            )}
          </div>
        );
      })}
      <button
        onClick={addTab}
        className="mb-0.5 ml-1 rounded p-1 text-muted-foreground hover:bg-muted hover:text-foreground"
        title="New tab (Ctrl+N)"
      >
        <Plus size={12} />
      </button>
    </div>
  );
}
