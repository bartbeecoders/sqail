export interface QueryHistoryEntry {
  id: string;
  timestamp: string;
  connectionId?: string;
  connectionName?: string;
  query: string;
  executionTimeMs: number;
  rowCount?: number;
  success: boolean;
  errorMessage?: string;
}

export interface SavedQuery {
  id: string;
  name: string;
  description?: string;
  query: string;
  connectionId?: string;
  tags: string[];
  folder?: string;
  createdAt: string;
  updatedAt: string;
}
