export interface EditorTab {
  id: string;
  title: string;
  content: string;
  filePath?: string;
  savedQueryId?: string;
  connectionId?: string;
}
