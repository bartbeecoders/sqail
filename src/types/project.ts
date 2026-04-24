import type { ConnectionConfig } from "./connection";
import type { DiagramState } from "./diagram";
import type { SqailPromptEntry } from "./sqailFile";

export type ProjectFileKind = "sql" | "diagram";

/**
 * A single file inside a project. Holds the last-snapshotted state; while a
 * tab is open and linked to this file (via `EditorTab.projectFileId`), the
 * tab is authoritative until the project is saved (which snapshots the tabs
 * back into their files).
 */
export interface ProjectFile {
  id: string;
  kind: ProjectFileKind;
  title: string;
  sql?: string;
  diagram?: DiagramState;
  connection?: ConnectionConfig;
  promptHistory?: SqailPromptEntry[];
}

export interface Project {
  id: string;
  name: string;
  files: ProjectFile[];
  /** Filesystem path the project was last saved to / opened from. */
  filePath?: string;
}
