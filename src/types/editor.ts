import type { DiagramState } from "./diagram";
import type { SqailPromptEntry } from "./sqailFile";

export type EditorTabKind = "sql" | "diagram";

export interface EditorTab {
  id: string;
  title: string;
  content: string;
  filePath?: string;
  savedQueryId?: string;
  connectionId?: string;
  pinned?: boolean;
  kind?: EditorTabKind; // default "sql"
  diagram?: DiagramState; // present when kind === "diagram"
  /** Last ~10 prompts typed into the AI palette while this tab was active.
   *  Powers ArrowUp/ArrowDown navigation in the palette. */
  promptHistory?: string[];
  /** Rich per-file AI exchanges (prompt + response). Bundled into `.sqail`
   *  when the tab is saved in that format. */
  aiHistory?: SqailPromptEntry[];
  /** When set, this tab is the working copy of a file inside the active
   *  project. Project save snapshots the tab's state back into that file. */
  projectFileId?: string;
}
