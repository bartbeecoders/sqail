import { save, open } from "@tauri-apps/plugin-dialog";
import { readTextFile, writeTextFile } from "@tauri-apps/plugin-fs";
import { invoke } from "@tauri-apps/api/core";
import { useEditorStore } from "../stores/editorStore";
import { useConnectionStore } from "../stores/connectionStore";
import { useProjectStore } from "../stores/projectStore";
import type { ConnectionConfig } from "../types/connection";
import type { EditorTab } from "../types/editor";
import type {
  SqailDiagramPayload,
  SqailPlainPayload,
  SqailPromptEntry,
  SqailSqlPayload,
} from "../types/sqailFile";
import {
  decryptPayloadSecrets,
  encodeSqailFile,
  parseSqailFile,
  serializeSqailFile,
} from "./sqail/codec";

const SQL_FILTER = { name: "SQL Files", extensions: ["sql"] };
const SQAIL_FILTER = { name: "sqail Bundles", extensions: ["sqail"] };

function isSqailPath(path: string): boolean {
  return path.toLowerCase().endsWith(".sqail");
}

function fileNameFromPath(path: string): string {
  return path.split(/[/\\]/).pop() ?? path;
}

function lookupConnection(id: string | undefined): ConnectionConfig | undefined {
  if (!id) return undefined;
  return useConnectionStore.getState().connections.find((c) => c.id === id);
}

function buildPayloadFromTab(tab: EditorTab): SqailPlainPayload {
  const connection = lookupConnection(tab.connectionId);
  const promptHistory: SqailPromptEntry[] = tab.aiHistory ?? [];
  if (tab.kind === "diagram" && tab.diagram) {
    const data: SqailDiagramPayload<ConnectionConfig> = {
      title: tab.title,
      diagram: tab.diagram,
      connection,
      promptHistory,
    };
    return { kind: "diagram", data };
  }
  const data: SqailSqlPayload<ConnectionConfig> = {
    title: tab.title,
    sql: tab.content,
    connection,
    promptHistory,
  };
  return { kind: "sql", data };
}

async function writeSqail(tab: EditorTab, path: string): Promise<void> {
  const payload = buildPayloadFromTab(tab);
  const file = await encodeSqailFile(payload);
  await writeTextFile(path, serializeSqailFile(file));
}

async function writePlainSql(tab: EditorTab, path: string): Promise<void> {
  await writeTextFile(path, tab.content);
}

// ---------------------------------------------------------------------------
// Save

export async function saveQuery(): Promise<void> {
  const tab = useEditorStore.getState().getActiveTab();
  if (!tab) return;

  if (tab.filePath) {
    if (isSqailPath(tab.filePath)) {
      await writeSqail(tab, tab.filePath);
    } else {
      if (!tab.content) return;
      await writePlainSql(tab, tab.filePath);
    }
    return;
  }
  await saveQueryAs();
}

export async function saveQueryAs(): Promise<void> {
  const tab = useEditorStore.getState().getActiveTab();
  if (!tab) return;

  const isDiagram = tab.kind === "diagram";
  // Diagrams have no plain-text representation — always .sqail.
  // SQL tabs: default to .sqail when there's context worth bundling
  // (connection or AI history); otherwise default to plain .sql.
  const hasContext =
    !!tab.connectionId || (tab.aiHistory && tab.aiHistory.length > 0);
  const filters = isDiagram
    ? [SQAIL_FILTER]
    : hasContext
      ? [SQAIL_FILTER, SQL_FILTER]
      : [SQL_FILTER, SQAIL_FILTER];

  const defaultExt = filters[0].extensions[0];
  const path = await save({
    filters,
    defaultPath: `${tab.title}.${defaultExt}`,
  });
  if (!path) return;

  if (isSqailPath(path)) {
    await writeSqail(tab, path);
  } else {
    if (isDiagram) {
      // Dialog returned a non-sqail path despite the filter — shouldn't happen,
      // but bail cleanly rather than silently losing the diagram.
      throw new Error("Diagrams can only be saved as .sqail files");
    }
    if (!tab.content) return;
    await writePlainSql(tab, path);
  }

  const store = useEditorStore.getState();
  store.setFilePath(tab.id, path);
  store.renameTab(tab.id, fileNameFromPath(path));
}

// ---------------------------------------------------------------------------
// Open

export async function openQuery(): Promise<void> {
  const path = await open({
    filters: [
      { name: "SQL or sqail", extensions: ["sql", "sqail"] },
      SQL_FILTER,
      SQAIL_FILTER,
    ],
    multiple: false,
  });
  if (!path) return;
  const pathStr = path as string;
  const content = await readTextFile(pathStr);

  if (isSqailPath(pathStr)) {
    await openSqailFile(content, pathStr);
  } else {
    openPlainSql(content, pathStr);
  }
}

function openPlainSql(content: string, path: string): void {
  const store = useEditorStore.getState();
  const fileName = fileNameFromPath(path);
  const active = store.getActiveTab();
  if (active && !active.content.trim()) {
    store.setContent(active.id, content);
    store.setFilePath(active.id, path);
    store.renameTab(active.id, fileName);
    return;
  }
  store.addRestoredTab({ title: fileName, content, filePath: path });
}

async function openSqailFile(raw: string, path: string): Promise<void> {
  const file = parseSqailFile(raw);

  let passphrase: string | undefined;
  if (file.passphraseProtected) {
    const entered = window.prompt(
      `"${fileNameFromPath(path)}" is passphrase-protected. Enter the passphrase to open it:`,
    );
    if (entered == null) return; // user cancelled
    passphrase = entered;
  }

  let plain: SqailPlainPayload;
  try {
    plain = await decryptPayloadSecrets(file.payload, passphrase);
  } catch (e) {
    window.alert(`Failed to open .sqail file: ${String(e)}`);
    return;
  }

  switch (plain.kind) {
    case "sql":
      await restoreSqlTab(plain.data, path);
      break;
    case "diagram":
      await restoreDiagramTab(plain.data, path);
      break;
    case "project":
      await useProjectStore.getState().adoptProjectPayload(plain.data, path);
      break;
  }
}

async function restoreSqlTab(
  data: SqailSqlPayload<ConnectionConfig>,
  path: string,
): Promise<void> {
  const connectionId = await maybeRegisterConnection(data.connection);
  const fileName = fileNameFromPath(path);
  useEditorStore.getState().addRestoredTab({
    title: fileName,
    content: data.sql ?? "",
    filePath: path,
    connectionId,
    promptHistory: lastPrompts(data.promptHistory),
    aiHistory: data.promptHistory ?? [],
  });
}

async function restoreDiagramTab(
  data: SqailDiagramPayload<ConnectionConfig>,
  path: string,
): Promise<void> {
  const connectionId = await maybeRegisterConnection(data.connection);
  const fileName = fileNameFromPath(path);
  useEditorStore.getState().addRestoredTab({
    title: fileName,
    content: "",
    kind: "diagram",
    diagram: data.diagram,
    filePath: path,
    connectionId,
    promptHistory: lastPrompts(data.promptHistory),
    aiHistory: data.promptHistory ?? [],
  });
}

/** Extract the last ~10 prompt strings so the palette's ArrowUp/Down list is populated. */
function lastPrompts(entries: SqailPromptEntry[] | undefined): string[] {
  if (!entries || entries.length === 0) return [];
  const prompts = entries
    .map((e) => e.prompt)
    .filter((p): p is string => typeof p === "string" && p.length > 0);
  return prompts.slice(-10);
}

/**
 * If the file bundled a connection, ask the user whether to add it to the
 * connection list. Credentials are never added silently — the user must opt in.
 * Returns the connection id to associate with the restored tab (or undefined).
 */
async function maybeRegisterConnection(
  bundled: ConnectionConfig | undefined,
): Promise<string | undefined> {
  if (!bundled) return undefined;

  const { connections } = useConnectionStore.getState();
  // Match by a stable tuple of (driver, host, port, database, user, filePath).
  const existing = connections.find(
    (c) =>
      c.driver === bundled.driver &&
      c.host === bundled.host &&
      c.port === bundled.port &&
      c.database === bundled.database &&
      c.user === bundled.user &&
      c.filePath === bundled.filePath,
  );
  if (existing) return existing.id;

  const label =
    bundled.name ||
    `${bundled.driver}://${bundled.user ? bundled.user + "@" : ""}${bundled.host}${bundled.port ? ":" + bundled.port : ""}${bundled.database ? "/" + bundled.database : ""}`;
  const ok = window.confirm(
    `This file bundles a database connection:\n\n  ${label}\n\nAdd it to your saved connections? Credentials will be stored locally.`,
  );
  if (!ok) return undefined;

  // Strip id so the backend assigns a new one.
  const toCreate: ConnectionConfig = { ...bundled, id: "" };
  const created = await invoke<ConnectionConfig>("create_connection", { config: toCreate });
  await useConnectionStore.getState().loadConnections();
  return created.id;
}
