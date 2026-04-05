import { useEffect, useState, useMemo, useCallback } from "react";
import {
  Search,
  X,
  Trash2,
  Pencil,
  Play,
  Plus,
  Tag,
  Folder,
  FileText,
  Download,
  Upload,
} from "lucide-react";
import { save, open } from "@tauri-apps/plugin-dialog";
import { readTextFile, writeTextFile } from "@tauri-apps/plugin-fs";
import { cn } from "../lib/utils";
import { useQueryHistoryStore } from "../stores/queryHistoryStore";
import { useEditorStore } from "../stores/editorStore";
import { useConnectionStore } from "../stores/connectionStore";
import type { SavedQuery } from "../types/queryHistory";

const SQL_FILTER = { name: "SQL Files", extensions: ["sql"] };

export default function SavedQueriesPanel() {
  const {
    savedQueries,
    loadSavedQueries,
    createSavedQuery,
    updateSavedQuery,
    deleteSavedQuery,
  } = useQueryHistoryStore();

  const [search, setSearch] = useState("");
  const [selectedFolder, setSelectedFolder] = useState<string | null>(null);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [showNewForm, setShowNewForm] = useState(false);
  const [initialSql, setInitialSql] = useState("");

  useEffect(() => {
    loadSavedQueries();
  }, [loadSavedQueries]);

  const folders = useMemo(() => {
    const set = new Set<string>();
    savedQueries.forEach((q) => {
      if (q.folder) set.add(q.folder);
    });
    return Array.from(set).sort();
  }, [savedQueries]);

  const allTags = useMemo(() => {
    const set = new Set<string>();
    savedQueries.forEach((q) => q.tags.forEach((t) => set.add(t)));
    return Array.from(set).sort();
  }, [savedQueries]);

  const filtered = useMemo(() => {
    let items = [...savedQueries];
    if (selectedFolder) items = items.filter((q) => q.folder === selectedFolder);
    if (search) {
      const lower = search.toLowerCase();
      items = items.filter(
        (q) =>
          q.name.toLowerCase().includes(lower) ||
          q.query.toLowerCase().includes(lower) ||
          q.tags.some((t) => t.toLowerCase().includes(lower)) ||
          q.description?.toLowerCase().includes(lower),
      );
    }
    return items.sort((a, b) => b.updatedAt.localeCompare(a.updatedAt));
  }, [savedQueries, search, selectedFolder]);

  const loadIntoEditor = useCallback((query: SavedQuery) => {
    const editorState = useEditorStore.getState();

    // If there's already a tab linked to this saved query, switch to it
    const existing = editorState.findTabBySavedQueryId(query.id);
    if (existing) {
      editorState.setActiveTab(existing.id);
      // Update content in case it was changed externally
      editorState.setContent(existing.id, query.query);
      return;
    }

    // Open a new tab with the query content
    editorState.addTabWithContent(query.name, query.query);
    const newTab = editorState.getActiveTab();
    if (newTab) {
      editorState.setSavedQueryId(newTab.id, query.id);
      // Link the tab to the saved query's connection, or fall back to current
      const connId = query.connectionId ?? useConnectionStore.getState().activeConnectionId ?? undefined;
      if (connId) editorState.setConnectionId(newTab.id, connId);
    }
  }, []);

  const handleExport = useCallback(
    async (query: SavedQuery) => {
      const filePath = await save({
        filters: [SQL_FILTER],
        defaultPath: `${query.name.replace(/[^a-zA-Z0-9_-]/g, "_")}.sql`,
      });
      if (!filePath) return;
      const header = `-- ${query.name}\n${query.description ? `-- ${query.description}\n` : ""}${query.tags.length ? `-- Tags: ${query.tags.join(", ")}\n` : ""}\n`;
      await writeTextFile(filePath, header + query.query);
    },
    [],
  );

  const handleImport = useCallback(async () => {
    const filePath = await open({ filters: [SQL_FILTER], multiple: false });
    if (!filePath) return;
    const content = await readTextFile(filePath as string);
    const name = (filePath as string).split(/[/\\]/).pop()?.replace(/\.sql$/i, "") ?? "Imported Query";
    const now = new Date().toISOString();
    createSavedQuery({
      id: crypto.randomUUID(),
      name,
      query: content,
      tags: ["imported"],
      createdAt: now,
      updatedAt: now,
    });
  }, [createSavedQuery]);

  const handleSave = useCallback(
    (query: SavedQuery, isEdit: boolean) => {
      if (isEdit) {
        updateSavedQuery(query);
      } else {
        // Check for duplicate name
        const duplicate = savedQueries.find(
          (q) => q.name.toLowerCase() === query.name.toLowerCase() && q.id !== query.id,
        );
        if (duplicate) {
          const overwrite = window.confirm(
            `A saved query named "${duplicate.name}" already exists.\n\nDo you want to overwrite it?`,
          );
          if (!overwrite) return;
          // Overwrite: use the existing query's id, keep original createdAt
          query = { ...query, id: duplicate.id, createdAt: duplicate.createdAt };
          updateSavedQuery(query);
        } else {
          createSavedQuery(query);
        }
      }

      // Sync the tab: link it and rename it to the saved query name
      const editorState = useEditorStore.getState();
      const tab = editorState.getActiveTab();
      if (tab && tab.content === query.query) {
        editorState.setSavedQueryId(tab.id, query.id);
        // If the tab still has a default "Query N" name, rename to the saved query name
        if (/^Query \d+$/.test(tab.title)) {
          editorState.renameTab(tab.id, query.name);
        }
      }

      setShowNewForm(false);
      setEditingId(null);
    },
    [savedQueries, createSavedQuery, updateSavedQuery],
  );

  return (
    <div className="flex h-full flex-col">
      {/* Search + actions bar */}
      <div className="flex items-center gap-1 border-b border-border px-2 py-1.5">
        <div className="relative flex-1">
          <Search
            size={10}
            className="absolute left-1.5 top-1/2 -translate-y-1/2 text-muted-foreground"
          />
          <input
            type="text"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder="Search saved queries..."
            className="input h-6 w-full pl-5 pr-5 text-[10px]"
          />
          {search && (
            <button
              onClick={() => setSearch("")}
              className="absolute right-1 top-1/2 -translate-y-1/2 rounded p-0.5 text-muted-foreground hover:text-foreground"
            >
              <X size={8} />
            </button>
          )}
        </div>
        <button
          onClick={() => {
            setInitialSql(useEditorStore.getState().getActiveTab()?.content ?? "");
            setShowNewForm(true);
            setEditingId(null);
          }}
          className="rounded p-1 text-muted-foreground hover:bg-accent hover:text-foreground"
          title="Save current query"
        >
          <Plus size={12} />
        </button>
        <button
          onClick={handleImport}
          className="rounded p-1 text-muted-foreground hover:bg-accent hover:text-foreground"
          title="Import .sql file"
        >
          <Upload size={12} />
        </button>
      </div>

      {/* Folder tabs */}
      {folders.length > 0 && (
        <div className="flex gap-0.5 overflow-x-auto border-b border-border px-2 py-1">
          <button
            onClick={() => setSelectedFolder(null)}
            className={cn(
              "shrink-0 rounded px-1.5 py-0.5 text-[9px] font-medium",
              !selectedFolder
                ? "bg-primary/10 text-primary"
                : "text-muted-foreground hover:bg-accent",
            )}
          >
            All
          </button>
          {folders.map((f) => (
            <button
              key={f}
              onClick={() => setSelectedFolder(selectedFolder === f ? null : f)}
              className={cn(
                "flex shrink-0 items-center gap-0.5 rounded px-1.5 py-0.5 text-[9px] font-medium",
                selectedFolder === f
                  ? "bg-primary/10 text-primary"
                  : "text-muted-foreground hover:bg-accent",
              )}
            >
              <Folder size={8} />
              {f}
            </button>
          ))}
        </div>
      )}

      {/* Save / edit form */}
      {(showNewForm || editingId) && (
        <SaveQueryForm
          initial={editingId ? savedQueries.find((q) => q.id === editingId) : undefined}
          initialSql={!editingId ? initialSql : undefined}
          existingTags={allTags}
          existingFolders={folders}
          onSave={(query) => handleSave(query, !!editingId)}
          onCancel={() => {
            setShowNewForm(false);
            setEditingId(null);
          }}
        />
      )}

      {/* Query list */}
      <div className="flex-1 overflow-y-auto">
        {filtered.length === 0 && !showNewForm && (
          <div className="py-8 text-center text-xs text-muted-foreground">
            <FileText size={20} className="mx-auto mb-2 opacity-30" />
            {savedQueries.length === 0
              ? "No saved queries yet"
              : "No matching queries"}
          </div>
        )}

        {filtered.map((query) => (
          <div
            key={query.id}
            className="group border-b border-border px-2 py-1.5 hover:bg-accent/50"
          >
            <div className="flex items-center gap-1.5">
              <FileText size={10} className="shrink-0 text-primary/60" />
              <span className="flex-1 truncate text-[11px] font-medium text-foreground">
                {query.name}
              </span>
              {query.folder && (
                <span className="shrink-0 rounded bg-muted px-1 py-0.5 text-[8px] text-muted-foreground">
                  {query.folder}
                </span>
              )}
            </div>

            {query.description && (
              <div className="mt-0.5 truncate text-[9px] text-muted-foreground">
                {query.description}
              </div>
            )}

            {/* Query preview */}
            <div
              onClick={() => loadIntoEditor(query)}
              className="mt-0.5 cursor-pointer truncate font-mono text-[10px] leading-tight text-foreground/60 hover:text-foreground"
              title="Click to load into editor"
            >
              {query.query.slice(0, 150)}
            </div>

            {/* Tags */}
            {query.tags.length > 0 && (
              <div className="mt-0.5 flex flex-wrap gap-0.5">
                {query.tags.map((tag) => (
                  <span
                    key={tag}
                    className="flex items-center gap-0.5 rounded bg-primary/8 px-1 py-0.5 text-[8px] text-primary/70"
                  >
                    <Tag size={6} />
                    {tag}
                  </span>
                ))}
              </div>
            )}

            {/* Hover actions */}
            <div className="mt-0.5 flex gap-1 opacity-0 transition-opacity group-hover:opacity-100">
              <button
                onClick={() => loadIntoEditor(query)}
                className="flex items-center gap-0.5 rounded px-1 py-0.5 text-[9px] text-muted-foreground hover:bg-accent hover:text-foreground"
              >
                <Play size={8} />
                Load
              </button>
              <button
                onClick={() => {
                  setEditingId(query.id);
                  setShowNewForm(false);
                }}
                className="flex items-center gap-0.5 rounded px-1 py-0.5 text-[9px] text-muted-foreground hover:bg-accent hover:text-foreground"
              >
                <Pencil size={8} />
                Edit
              </button>
              <button
                onClick={() => handleExport(query)}
                className="flex items-center gap-0.5 rounded px-1 py-0.5 text-[9px] text-muted-foreground hover:bg-accent hover:text-foreground"
              >
                <Download size={8} />
                Export
              </button>
              <button
                onClick={() => deleteSavedQuery(query.id)}
                className="flex items-center gap-0.5 rounded px-1 py-0.5 text-[9px] text-destructive/70 hover:bg-accent hover:text-destructive"
              >
                <Trash2 size={8} />
              </button>
            </div>
          </div>
        ))}
      </div>

      {/* Footer */}
      <div className="flex items-center justify-between border-t border-border px-2 py-1">
        <span className="text-[9px] text-muted-foreground">
          {savedQueries.length} saved
        </span>
      </div>
    </div>
  );
}

// ── Inline save/edit form ──────────────────────────────────

function SaveQueryForm({
  initial,
  initialSql,
  existingTags,
  existingFolders,
  onSave,
  onCancel,
}: {
  initial?: SavedQuery;
  initialSql?: string;
  existingTags: string[];
  existingFolders: string[];
  onSave: (query: SavedQuery) => void;
  onCancel: () => void;
}) {
  const activeTab = useEditorStore.getState().getActiveTab();
  const activeConnectionId = useConnectionStore((s) => s.activeConnectionId);

  // Default the name: use the tab title if it's not a default "Query N" name
  const defaultName = () => {
    if (initial?.name) return initial.name;
    if (activeTab && !/^Query \d+$/.test(activeTab.title)) return activeTab.title;
    return "";
  };

  const [name, setName] = useState(defaultName);
  const [description, setDescription] = useState(initial?.description ?? "");
  const [folder, setFolder] = useState(initial?.folder ?? "");
  const [tagsInput, setTagsInput] = useState(initial?.tags.join(", ") ?? "");
  const sql = initial?.query ?? initialSql ?? activeTab?.content ?? "";

  const handleSubmit = () => {
    if (!name.trim()) return;
    const now = new Date().toISOString();
    const tags = tagsInput
      .split(",")
      .map((t) => t.trim())
      .filter(Boolean);
    onSave({
      id: initial?.id ?? crypto.randomUUID(),
      name: name.trim(),
      description: description.trim() || undefined,
      query: sql,
      connectionId: initial?.connectionId ?? activeConnectionId ?? undefined,
      tags,
      folder: folder.trim() || undefined,
      createdAt: initial?.createdAt ?? now,
      updatedAt: now,
    });
  };

  return (
    <div className="border-b border-border bg-muted/30 px-2 py-2 space-y-1.5">
      <input
        type="text"
        value={name}
        onChange={(e) => setName(e.target.value)}
        placeholder="Query name *"
        className="input h-6 w-full text-[10px]"
        autoFocus
      />
      <input
        type="text"
        value={description}
        onChange={(e) => setDescription(e.target.value)}
        placeholder="Description (optional)"
        className="input h-6 w-full text-[10px]"
      />
      <div className="flex gap-1">
        <input
          type="text"
          value={folder}
          onChange={(e) => setFolder(e.target.value)}
          placeholder="Folder"
          className="input h-6 flex-1 text-[10px]"
          list="folder-suggestions"
        />
        <datalist id="folder-suggestions">
          {existingFolders.map((f) => (
            <option key={f} value={f} />
          ))}
        </datalist>
        <input
          type="text"
          value={tagsInput}
          onChange={(e) => setTagsInput(e.target.value)}
          placeholder="Tags (comma-separated)"
          className="input h-6 flex-1 text-[10px]"
          list="tag-suggestions"
        />
        <datalist id="tag-suggestions">
          {existingTags.map((t) => (
            <option key={t} value={t} />
          ))}
        </datalist>
      </div>
      <div className="truncate font-mono text-[9px] text-muted-foreground">
        {sql.slice(0, 100)}
        {sql.length > 100 ? "..." : ""}
      </div>
      <div className="flex gap-1">
        <button
          onClick={handleSubmit}
          disabled={!name.trim()}
          className="rounded bg-primary px-2 py-0.5 text-[10px] font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-40"
        >
          {initial ? "Update" : "Save"}
        </button>
        <button
          onClick={onCancel}
          className="rounded px-2 py-0.5 text-[10px] text-muted-foreground hover:bg-accent"
        >
          Cancel
        </button>
      </div>
    </div>
  );
}
