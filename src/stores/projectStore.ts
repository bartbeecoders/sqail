import { create } from "zustand";
import { save, open } from "@tauri-apps/plugin-dialog";
import { readTextFile, writeTextFile } from "@tauri-apps/plugin-fs";
import { invoke } from "@tauri-apps/api/core";
import type { ConnectionConfig } from "../types/connection";
import type { EditorTab } from "../types/editor";
import type { ProjectGitConfig } from "../types/git";
import type { Project, ProjectFile } from "../types/project";
import type {
  SqailDiagramPayload,
  SqailPlainPayload,
  SqailProjectFile,
  SqailProjectPayload,
  SqailSqlPayload,
} from "../types/sqailFile";
import {
  decryptPayloadSecrets,
  encodeSqailFile,
  parseSqailFile,
  serializeSqailFile,
} from "../lib/sqail/codec";
import { useEditorStore } from "./editorStore";
import { useConnectionStore } from "./connectionStore";

const STORAGE_KEY = "sqail_active_project";

function generateId(): string {
  return crypto.randomUUID();
}

function fileNameFromPath(path: string): string {
  return path.split(/[/\\]/).pop() ?? path;
}

function loadInitial(): Project | null {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as Project;
    if (!parsed.id || !Array.isArray(parsed.files)) return null;
    return parsed;
  } catch {
    return null;
  }
}

function persist(project: Project | null): void {
  if (!project) {
    localStorage.removeItem(STORAGE_KEY);
    return;
  }
  localStorage.setItem(STORAGE_KEY, JSON.stringify(project));
}

interface ProjectState {
  project: Project | null;
  newProject: (name?: string) => void;
  closeProject: () => void;
  /** Add the active editor tab to the project as a new file and link the tab. */
  addActiveTabToProject: () => void;
  /** Open a project file as a tab (focuses an existing linked tab if present). */
  openFile: (fileId: string) => void;
  removeFile: (fileId: string) => void;
  renameFile: (fileId: string, title: string) => void;
  saveProject: () => Promise<void>;
  saveProjectAs: () => Promise<void>;
  openProjectDialog: () => Promise<void>;
  /** Adopt a decoded project payload (used by fileOps when a .sqail of kind
   *  "project" is opened). Replaces any active project. */
  adoptProjectPayload: (
    data: SqailProjectPayload<ConnectionConfig>,
    path: string,
  ) => Promise<void>;
  /** Attach or update git integration config on the active project. */
  setGitConfig: (git: ProjectGitConfig | undefined) => void;
}

export const useProjectStore = create<ProjectState>((set, get) => ({
  project: loadInitial(),

  newProject: (name) => {
    const project: Project = {
      id: generateId(),
      name: name ?? "Untitled project",
      files: [],
    };
    set({ project });
    persist(project);
  },

  closeProject: () => {
    const { project } = get();
    if (!project) return;
    // Unlink any tabs that were referencing this project's files.
    const tabs = useEditorStore.getState().tabs;
    for (const t of tabs) {
      if (t.projectFileId) {
        useEditorStore.getState().renameTab(t.id, t.title); // no-op but triggers persist
      }
    }
    set({ project: null });
    persist(null);
  },

  addActiveTabToProject: () => {
    const tab = useEditorStore.getState().getActiveTab();
    if (!tab) return;
    const { project } = get();
    if (!project) return;
    if (tab.projectFileId) return; // already linked

    const conn = tab.connectionId
      ? useConnectionStore.getState().connections.find((c) => c.id === tab.connectionId)
      : undefined;

    const file: ProjectFile = {
      id: generateId(),
      kind: tab.kind === "diagram" ? "diagram" : "sql",
      title: tab.title,
      sql: tab.kind === "diagram" ? undefined : tab.content,
      diagram: tab.kind === "diagram" ? tab.diagram : undefined,
      connection: conn,
      promptHistory: tab.aiHistory ?? [],
    };
    const updated = { ...project, files: [...project.files, file] };
    set({ project: updated });
    persist(updated);

    // Link the tab back to this new file.
    useEditorStore.setState((s) => ({
      tabs: s.tabs.map((t) => (t.id === tab.id ? { ...t, projectFileId: file.id } : t)),
    }));
  },

  openFile: (fileId) => {
    const { project } = get();
    if (!project) return;
    const file = project.files.find((f) => f.id === fileId);
    if (!file) return;

    // If an open tab already references this file, just focus it.
    const editor = useEditorStore.getState();
    const existing = editor.tabs.find((t) => t.projectFileId === fileId);
    if (existing) {
      editor.setActiveTab(existing.id);
      return;
    }

    // Resolve the file's bundled connection (if any) to a real connection id
    // in the connection store. We do not silently register — if the user ran
    // the project through `openProjectDialog`, registration was handled there.
    const connectionId = resolveBundledConnection(file.connection);

    const descriptor: Omit<EditorTab, "id"> = {
      title: file.title,
      content: file.kind === "diagram" ? "" : (file.sql ?? ""),
      kind: file.kind,
      diagram: file.diagram,
      connectionId,
      aiHistory: file.promptHistory ?? [],
      promptHistory: lastPrompts(file.promptHistory),
      projectFileId: file.id,
    };
    editor.addRestoredTab(descriptor);
  },

  removeFile: (fileId) => {
    const { project } = get();
    if (!project) return;
    const updated = {
      ...project,
      files: project.files.filter((f) => f.id !== fileId),
    };
    set({ project: updated });
    persist(updated);
    // Unlink any tab pointing at the removed file.
    useEditorStore.setState((s) => ({
      tabs: s.tabs.map((t) =>
        t.projectFileId === fileId ? { ...t, projectFileId: undefined } : t,
      ),
    }));
  },

  renameFile: (fileId, title) => {
    const { project } = get();
    if (!project) return;
    const updated = {
      ...project,
      files: project.files.map((f) => (f.id === fileId ? { ...f, title } : f)),
    };
    set({ project: updated });
    persist(updated);
  },

  saveProject: async () => {
    const { project } = get();
    if (!project) return;
    if (!project.filePath) {
      await get().saveProjectAs();
      return;
    }
    await writeProjectToPath(project, project.filePath, set);
  },

  saveProjectAs: async () => {
    const { project } = get();
    if (!project) return;
    const path = await save({
      filters: [{ name: "sqail Project", extensions: ["sqail"] }],
      defaultPath: `${project.name}.sqail`,
    });
    if (!path) return;
    await writeProjectToPath(project, path, set);
  },

  openProjectDialog: async () => {
    const path = await open({
      filters: [{ name: "sqail Project", extensions: ["sqail"] }],
      multiple: false,
    });
    if (!path) return;
    const raw = await readTextFile(path as string);
    const file = parseSqailFile(raw);

    let passphrase: string | undefined;
    if (file.passphraseProtected) {
      const entered = window.prompt(
        `"${fileNameFromPath(path as string)}" is passphrase-protected. Enter the passphrase to open it:`,
      );
      if (entered == null) return;
      passphrase = entered;
    }

    let plain: SqailPlainPayload;
    try {
      plain = await decryptPayloadSecrets(file.payload, passphrase);
    } catch (e) {
      window.alert(`Failed to open project: ${String(e)}`);
      return;
    }
    if (plain.kind !== "project") {
      window.alert("Selected .sqail file is not a project bundle.");
      return;
    }
    await get().adoptProjectPayload(plain.data, path as string);
  },

  adoptProjectPayload: async (data, path) => {
    // Register any bundled connections the user opts into first, mapping them
    // so project files that reference them get real connection ids later.
    const registeredByKey = new Map<string, string>();
    for (const conn of data.connections ?? []) {
      const id = await maybeRegisterConnection(conn);
      if (id) registeredByKey.set(connectionKey(conn), id);
    }
    // For each file, also offer to register its own connection (de-duped).
    for (const f of data.files) {
      const conn = f.payload.connection;
      if (!conn) continue;
      const key = connectionKey(conn);
      if (registeredByKey.has(key)) continue;
      const id = await maybeRegisterConnection(conn);
      if (id) registeredByKey.set(key, id);
    }

    const files: ProjectFile[] = data.files.map((f): ProjectFile => {
      if (f.kind === "diagram") {
        const p = f.payload as SqailDiagramPayload<ConnectionConfig>;
        return {
          id: generateId(),
          kind: "diagram",
          title: p.title,
          diagram: p.diagram,
          connection: p.connection,
          promptHistory: p.promptHistory ?? [],
        };
      }
      const p = f.payload as SqailSqlPayload<ConnectionConfig>;
      return {
        id: generateId(),
        kind: "sql",
        title: p.title,
        sql: p.sql,
        connection: p.connection,
        promptHistory: p.promptHistory ?? [],
      };
    });

    const project: Project = {
      id: generateId(),
      name: data.name,
      files,
      filePath: path,
      git: data.git,
    };
    set({ project });
    persist(project);
  },

  setGitConfig: (git) => {
    const { project } = get();
    if (!project) return;
    const updated = { ...project, git };
    set({ project: updated });
    persist(updated);
  },
}));

// ---------------------------------------------------------------------------

async function writeProjectToPath(
  project: Project,
  path: string,
  set: (partial: Partial<ProjectState>) => void,
): Promise<void> {
  // Snapshot any open tabs linked to project files back into the files array.
  const tabs = useEditorStore.getState().tabs;
  const connections = useConnectionStore.getState().connections;

  const snapshotted: ProjectFile[] = project.files.map((f) => {
    const linked = tabs.find((t) => t.projectFileId === f.id);
    if (!linked) return f;
    const conn = linked.connectionId
      ? connections.find((c) => c.id === linked.connectionId)
      : f.connection;
    return {
      ...f,
      title: linked.title || f.title,
      sql: linked.kind === "diagram" ? undefined : linked.content,
      diagram: linked.kind === "diagram" ? linked.diagram : undefined,
      connection: conn,
      promptHistory: linked.aiHistory ?? f.promptHistory,
    };
  });

  const payload: SqailPlainPayload = {
    kind: "project",
    data: {
      name: project.name,
      git: project.git,
      files: snapshotted.map((f): SqailProjectFile<ConnectionConfig> => {
        if (f.kind === "diagram") {
          const p: SqailDiagramPayload<ConnectionConfig> = {
            title: f.title,
            diagram: f.diagram ?? ({} as never),
            connection: f.connection,
            promptHistory: f.promptHistory,
          };
          return { kind: "diagram", payload: p };
        }
        const p: SqailSqlPayload<ConnectionConfig> = {
          title: f.title,
          sql: f.sql ?? "",
          connection: f.connection,
          promptHistory: f.promptHistory,
        };
        return { kind: "sql", payload: p };
      }),
    },
  };

  const file = await encodeSqailFile(payload);
  await writeTextFile(path, serializeSqailFile(file));

  const updated: Project = { ...project, files: snapshotted, filePath: path };
  set({ project: updated });
  persist(updated);
}

function connectionKey(c: ConnectionConfig): string {
  return `${c.driver}|${c.host}|${c.port}|${c.database}|${c.user}|${c.filePath}`;
}

function resolveBundledConnection(
  bundled: ConnectionConfig | undefined,
): string | undefined {
  if (!bundled) return undefined;
  const { connections } = useConnectionStore.getState();
  const match = connections.find((c) => connectionKey(c) === connectionKey(bundled));
  return match?.id;
}

async function maybeRegisterConnection(
  bundled: ConnectionConfig,
): Promise<string | undefined> {
  const { connections } = useConnectionStore.getState();
  const existing = connections.find(
    (c) => connectionKey(c) === connectionKey(bundled),
  );
  if (existing) return existing.id;
  const label =
    bundled.name ||
    `${bundled.driver}://${bundled.user ? bundled.user + "@" : ""}${bundled.host}${bundled.port ? ":" + bundled.port : ""}${bundled.database ? "/" + bundled.database : ""}`;
  const ok = window.confirm(
    `This project bundles a database connection:\n\n  ${label}\n\nAdd it to your saved connections? Credentials will be stored locally.`,
  );
  if (!ok) return undefined;
  const toCreate: ConnectionConfig = { ...bundled, id: "" };
  const created = await invoke<ConnectionConfig>("create_connection", { config: toCreate });
  await useConnectionStore.getState().loadConnections();
  return created.id;
}

function lastPrompts(
  entries: { prompt?: string }[] | undefined,
): string[] {
  if (!entries || entries.length === 0) return [];
  const prompts = entries
    .map((e) => e.prompt)
    .filter((p): p is string => typeof p === "string" && p.length > 0);
  return prompts.slice(-10);
}
