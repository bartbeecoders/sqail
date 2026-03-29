import type { languages } from "monaco-editor";
import { useSchemaStore } from "../stores/schemaStore";

const SQL_KEYWORDS = [
  "SELECT", "FROM", "WHERE", "AND", "OR", "NOT", "IN", "IS", "NULL",
  "LIKE", "BETWEEN", "EXISTS", "CASE", "WHEN", "THEN", "ELSE", "END",
  "AS", "ON", "JOIN", "INNER", "LEFT", "RIGHT", "FULL", "OUTER", "CROSS",
  "INSERT", "INTO", "VALUES", "UPDATE", "SET", "DELETE",
  "CREATE", "ALTER", "DROP", "TABLE", "INDEX", "VIEW", "DATABASE", "SCHEMA",
  "PRIMARY", "KEY", "FOREIGN", "REFERENCES", "UNIQUE", "CHECK", "DEFAULT",
  "CONSTRAINT", "CASCADE", "RESTRICT",
  "GROUP", "BY", "ORDER", "ASC", "DESC", "HAVING", "LIMIT", "OFFSET",
  "UNION", "ALL", "INTERSECT", "EXCEPT", "DISTINCT",
  "COUNT", "SUM", "AVG", "MIN", "MAX", "COALESCE", "NULLIF",
  "CAST", "CONVERT", "EXTRACT", "SUBSTRING", "TRIM", "UPPER", "LOWER",
  "NOW", "CURRENT_TIMESTAMP", "CURRENT_DATE", "CURRENT_TIME",
  "TRUE", "FALSE", "WITH", "RECURSIVE", "RETURNING",
  "BEGIN", "COMMIT", "ROLLBACK", "SAVEPOINT", "TRANSACTION",
  "GRANT", "REVOKE", "TRUNCATE", "EXPLAIN", "ANALYZE", "VACUUM",
  "IF", "REPLACE", "TEMPORARY", "TEMP", "SERIAL", "BIGSERIAL",
  "INTEGER", "INT", "BIGINT", "SMALLINT", "NUMERIC", "DECIMAL", "REAL",
  "DOUBLE", "PRECISION", "FLOAT", "BOOLEAN", "BOOL",
  "VARCHAR", "CHAR", "TEXT", "BYTEA", "BLOB",
  "DATE", "TIME", "TIMESTAMP", "INTERVAL", "TIMESTAMPTZ",
  "JSON", "JSONB", "UUID", "ARRAY",
];

export function createSqlCompletionProvider(): languages.CompletionItemProvider {
  return {
    provideCompletionItems: (model, position) => {
      const word = model.getWordUntilPosition(position);
      const range = {
        startLineNumber: position.lineNumber,
        endLineNumber: position.lineNumber,
        startColumn: word.startColumn,
        endColumn: word.endColumn,
      };

      const suggestions: languages.CompletionItem[] = [];

      // SQL keywords
      for (const kw of SQL_KEYWORDS) {
        suggestions.push({
          label: kw,
          kind: 17, // Keyword
          insertText: kw,
          range,
          sortText: `2_${kw}`,
        });
      }

      // Schema-aware: table names
      const tableNames = useSchemaStore.getState().getAllTableNames();
      for (const name of tableNames) {
        suggestions.push({
          label: name,
          kind: 1, // Class (table icon)
          insertText: name,
          detail: "table",
          range,
          sortText: `0_${name}`,
        });
      }

      // Schema-aware: column names
      const columnNames = useSchemaStore.getState().getAllColumnNames();
      for (const name of columnNames) {
        suggestions.push({
          label: name,
          kind: 4, // Field (column icon)
          insertText: name,
          detail: "column",
          range,
          sortText: `1_${name}`,
        });
      }

      return { suggestions };
    },
  };
}
