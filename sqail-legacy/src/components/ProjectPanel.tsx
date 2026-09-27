import { useState } from "react";
import {
  FolderOpen,
  FolderPlus,
  FileCode2,
  Network,
  Save,
  Trash2,
  Pencil,
  Plus,
  X,
} from "lucide-react";
import { cn } from "../lib/utils";
import { useProjectStore } from "../stores/projectStore";
import { useEditorStore } from "../stores/editorStore";
import GitPanel from "./GitPanel";

export default function ProjectPanel() {
  const project = useProjectStore((s) => s.project);
  const newProject = useProjectStore((s) => s.newProject);
  const closeProject = useProjectStore((s) => s.closeProject);
  const openProjectDialog = useProjectStore((s) => s.openProjectDialog);
  const saveProject = useProjectStore((s) => s.saveProject);
  const saveProjectAs = useProjectStore((s) => s.saveProjectAs);
  const addActiveTabToProject = useProjectStore((s) => s.addActiveTabToProject);
  const openFile = useProjectStore((s) => s.openFile);
  const removeFile = useProjectStore((s) => s.removeFile);
  const renameFile = useProjectStore((s) => s.renameFile);

  const [renamingId, setRenamingId] = useState<string | null>(null);
  const [renameDraft, setRenameDraft] = useState("");

  const tabs = useEditorStore((s) => s.tabs);
  const activeTabId = useEditorStore((s) => s.activeTabId);
  const activeTab = tabs.find((t) => t.id === activeTabId);
  const activeAlreadyLinked = !!activeTab?.projectFileId;

  if (!project) {
    return (
      <div className="flex h-full flex-col items-stretch justify-start gap-2 p-3">
        <p className="text-xs text-muted-foreground">
          No project is open. Projects bundle related SQL scripts and diagrams (plus their
          connection + AI history) into a single <code className="rounded bg-muted px-1">.sqail</code>{" "}
          file.
        </p>
        <button
          onClick={() => newProject()}
          className="flex items-center gap-2 rounded-md border border-border px-2 py-1.5 text-xs hover:bg-accent"
        >
          <FolderPlus size={12} />
          New project
        </button>
        <button
          onClick={() => openProjectDialog().catch(console.error)}
          className="flex items-center gap-2 rounded-md border border-border px-2 py-1.5 text-xs hover:bg-accent"
        >
          <FolderOpen size={12} />
          Open project…
        </button>
      </div>
    );
  }

  const handleRename = (id: string, title: string) => {
    setRenamingId(id);
    setRenameDraft(title);
  };

  const commitRename = () => {
    if (renamingId && renameDraft.trim()) {
      renameFile(renamingId, renameDraft.trim());
    }
    setRenamingId(null);
    setRenameDraft("");
  };

  return (
    <div className="flex h-full flex-col overflow-hidden">
      {/* Header with project name and actions */}
      <div className="flex shrink-0 items-center gap-1 border-b border-border px-2 py-1.5">
        <ProjectNameEditor
          name={project.name}
          onRename={(name) => {
            // Reuse newProject's persist path by mutating through the store.
            useProjectStore.setState((s) =>
              s.project ? { project: { ...s.project, name } } : s,
            );
            // Persist via saveProject is not appropriate here (renames should be
            // reflected in-memory immediately; next save picks them up).
            const proj = useProjectStore.getState().project;
            if (proj) localStorage.setItem("sqail_active_project", JSON.stringify(proj));
          }}
        />
        <button
          onClick={() => saveProject().catch(console.error)}
          className="rounded p-1 text-muted-foreground hover:bg-accent hover:text-accent-foreground"
          title="Save project"
        >
          <Save size={12} />
        </button>
        <button
          onClick={() => saveProjectAs().catch(console.error)}
          className="rounded px-1 text-[9px] font-medium text-muted-foreground hover:bg-accent hover:text-accent-foreground"
          title="Save project as…"
        >
          As…
        </button>
        <button
          onClick={() => closeProject()}
          className="rounded p-1 text-muted-foreground hover:bg-accent hover:text-accent-foreground"
          title="Close project (unsaved changes are kept in open tabs)"
        >
          <X size={12} />
        </button>
      </div>

      {/* Add-tab button */}
      <div className="shrink-0 border-b border-border px-2 py-1.5">
        <button
          onClick={() => addActiveTabToProject()}
          disabled={!activeTab || activeAlreadyLinked}
          className={cn(
            "flex w-full items-center gap-2 rounded-md border border-border px-2 py-1 text-[11px] transition-colors",
            activeTab && !activeAlreadyLinked
              ? "hover:bg-accent"
              : "cursor-not-allowed opacity-50",
          )}
          title={
            !activeTab
              ? "No active tab"
              : activeAlreadyLinked
                ? "Active tab is already part of a project file"
                : "Add the active tab to this project"
          }
        >
          <Plus size={11} />
          Add current tab
        </button>
      </div>

      {/* File list + git panel (scrollable) */}
      <div className="flex-1 overflow-y-auto">
        {project.files.length === 0 ? (
          <p className="p-3 text-[11px] text-muted-foreground">
            This project has no files yet. Open a SQL tab or diagram and click{" "}
            <em>Add current tab</em> above.
          </p>
        ) : (
          <ul className="py-1">
            {project.files.map((f) => {
              const linked = tabs.find((t) => t.projectFileId === f.id);
              const Icon = f.kind === "diagram" ? Network : FileCode2;
              return (
                <li
                  key={f.id}
                  className={cn(
                    "group flex items-center gap-1.5 px-2 py-1 text-[11px] hover:bg-accent/50",
                    linked && linked.id === activeTabId && "bg-accent",
                  )}
                >
                  <Icon size={11} className="shrink-0 text-muted-foreground" />
                  {renamingId === f.id ? (
                    <input
                      autoFocus
                      value={renameDraft}
                      onChange={(e) => setRenameDraft(e.target.value)}
                      onBlur={commitRename}
                      onKeyDown={(e) => {
                        if (e.key === "Enter") commitRename();
                        if (e.key === "Escape") {
                          setRenamingId(null);
                          setRenameDraft("");
                        }
                      }}
                      className="flex-1 rounded border border-border bg-background px-1 py-0.5 text-[11px] outline-none"
                    />
                  ) : (
                    <button
                      onClick={() => openFile(f.id)}
                      onDoubleClick={() => handleRename(f.id, f.title)}
                      className="flex-1 truncate text-left"
                      title={f.title}
                    >
                      {f.title}
                    </button>
                  )}
                  <button
                    onClick={() => handleRename(f.id, f.title)}
                    className="opacity-0 transition-opacity group-hover:opacity-100"
                    title="Rename"
                  >
                    <Pencil size={10} className="text-muted-foreground hover:text-foreground" />
                  </button>
                  <button
                    onClick={() => {
                      if (window.confirm(`Remove "${f.title}" from this project?`)) {
                        removeFile(f.id);
                      }
                    }}
                    className="opacity-0 transition-opacity group-hover:opacity-100"
                    title="Remove from project"
                  >
                    <Trash2
                      size={10}
                      className="text-muted-foreground hover:text-destructive"
                    />
                  </button>
                </li>
              );
            })}
          </ul>
        )}
        <GitPanel />
      </div>
    </div>
  );
}

function ProjectNameEditor({
  name,
  onRename,
}: {
  name: string;
  onRename: (name: string) => void;
}) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(name);

  if (editing) {
    return (
      <input
        autoFocus
        value={draft}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={() => {
          if (draft.trim()) onRename(draft.trim());
          setEditing(false);
        }}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            if (draft.trim()) onRename(draft.trim());
            setEditing(false);
          }
          if (e.key === "Escape") {
            setDraft(name);
            setEditing(false);
          }
        }}
        className="flex-1 rounded border border-border bg-background px-1.5 py-0.5 text-xs outline-none"
      />
    );
  }

  return (
    <button
      onClick={() => {
        setDraft(name);
        setEditing(true);
      }}
      className="flex-1 truncate text-left text-xs font-medium hover:text-primary"
      title="Rename project"
    >
      {name}
    </button>
  );
}
