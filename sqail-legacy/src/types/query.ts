export interface QueryColumn {
  name: string;
  typeName: string;
}

export interface QueryResult {
  columns: QueryColumn[];
  rows: (string | number | boolean | null)[][];
  rowCount: number;
  affectedRows: number | null;
  executionTimeMs: number;
  isMutation: boolean;
  statementIndex: number;
}

export interface QueryResponse {
  results: QueryResult[];
  totalTimeMs: number;
  error: string | null;
}
