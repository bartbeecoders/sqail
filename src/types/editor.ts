import type { DiagramState } from "./diagram";

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
}
