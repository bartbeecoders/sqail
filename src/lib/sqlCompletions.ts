import type { languages, IRange } from "monaco-editor";
import { useSchemaStore } from "../stores/schemaStore";
import { useConnectionStore } from "../stores/connectionStore";
import { useSnippetStore } from "../stores/snippetStore";
import type { Driver } from "../types/connection";

// ── Context detection ─────────────────────────────────────────

/**
 * What kind of token the cursor position expects next.
 * Used to filter and prioritise completion items.
 */
type CompletionContext =
  | "table"          // after FROM, JOIN, INTO, UPDATE, table-position keywords
  | "column"         // after SELECT, WHERE, SET, ON, GROUP BY, ORDER BY, HAVING
  | "schema"         // after schema-dot (e.g. "public.")
  | "tableColumn"    // after "table." — columns of a specific table/alias
  | "keyword"        // general keyword position (start of statement, after semicolon)
  | "function"       // after SELECT expr position, WHERE expr position
  | "joinCondition"  // after ON in a JOIN
  | "datatype"       // after column name in CREATE TABLE, or after CAST(... AS
  | "general";       // can't determine — show everything

interface CursorContext {
  kind: CompletionContext;
  /** For "schema" context: the schema name before the dot */
  schemaName?: string;
  /** For "tableColumn" context: the table/alias name before the dot */
  tableName?: string;
  /** Tables/aliases referenced in the current statement (for column scoping) */
  referencedTables: ReferencedTable[];
}

export interface ReferencedTable {
  schema?: string;
  table: string;
  alias?: string;
}

/**
 * Analyse the SQL text up to the cursor to determine completion context.
 * fullText is the complete document — used to extract table references
 * from the entire statement (FROM clause may come after the cursor).
 */
function detectContext(textBeforeCursor: string, fullText: string): CursorContext {
  // Strip string literals and comments to avoid false matches
  const cleaned = stripStringsAndComments(textBeforeCursor);

  // Extract referenced tables from the FULL statement text, not just before cursor
  const currentStmtFull = getCurrentStatement(stripStringsAndComments(fullText), textBeforeCursor.length);
  const referencedTables = extractReferencedTables(currentStmtFull);

  // Get the last meaningful tokens before cursor
  const trimmed = cleaned.trimEnd();

  // Check if cursor is right after a dot (qualified identifier)
  if (trimmed.endsWith(".")) {
    const beforeDot = trimmed.slice(0, -1).trimEnd();
    const lastWord = getLastWord(beforeDot);
    if (lastWord) {
      const upperWord = lastWord.toUpperCase();
      // Check if it matches a table alias or table name in the current statement first
      const isTableRef = referencedTables.some(
        (r) => r.alias?.toUpperCase() === upperWord || r.table.toUpperCase() === upperWord,
      );
      if (isTableRef) {
        return { kind: "tableColumn", tableName: lastWord, referencedTables };
      }
      // Check if it's a schema name
      const schemas = useSchemaStore.getState().schemas;
      if (schemas.some((s) => s.name.toUpperCase() === upperWord)) {
        return { kind: "schema", schemaName: lastWord, referencedTables };
      }
      // Otherwise it's a table/alias prefix (unresolved — maybe unloaded table)
      return { kind: "tableColumn", tableName: lastWord, referencedTables };
    }
  }

  // Get the last few significant tokens
  const tokens = tokenizeEnd(trimmed, 4);
  const last = tokens[tokens.length - 1]?.toUpperCase() ?? "";
  const secondLast = tokens[tokens.length - 2]?.toUpperCase() ?? "";

  // ── Table context ──
  if (
    last === "FROM" || last === "JOIN" || last === "INTO" ||
    last === "UPDATE" || last === "TABLE" || last === "TRUNCATE" ||
    last === "VIEW" || last === "DESCRIBE" ||
    (secondLast === "LEFT" && last === "JOIN") ||
    (secondLast === "RIGHT" && last === "JOIN") ||
    (secondLast === "INNER" && last === "JOIN") ||
    (secondLast === "OUTER" && last === "JOIN") ||
    (secondLast === "FULL" && last === "JOIN") ||
    (secondLast === "CROSS" && last === "JOIN") ||
    (secondLast === "NATURAL" && last === "JOIN") ||
    (secondLast === "INSERT" && last === "INTO") ||
    (secondLast === "DELETE" && last === "FROM")
  ) {
    return { kind: "table", referencedTables };
  }

  // ── Column context ──
  if (last === "SELECT" || last === "DISTINCT") {
    return { kind: "column", referencedTables };
  }
  if (
    last === "WHERE" || last === "AND" || last === "OR" ||
    last === "SET" || last === "HAVING" ||
    last === "WHEN" || last === "THEN" || last === "ELSE" ||
    last === "CASE"
  ) {
    return { kind: "column", referencedTables };
  }
  if (
    last === "BY" &&
    (secondLast === "GROUP" || secondLast === "ORDER" || secondLast === "PARTITION")
  ) {
    return { kind: "column", referencedTables };
  }
  if (last === "ON") {
    return { kind: "joinCondition", referencedTables };
  }

  // ── Data type context ──
  if (last === "AS" && secondLast === "(") {
    // CAST(expr AS ...)
    return { kind: "datatype", referencedTables };
  }

  // ── After comma in SELECT list or GROUP BY etc. ──
  const clauseCtx = findActiveClause(cleaned);
  if (clauseCtx === "SELECT" || clauseCtx === "GROUP BY" || clauseCtx === "ORDER BY") {
    return { kind: "column", referencedTables };
  }
  if (clauseCtx === "FROM" || clauseCtx === "JOIN") {
    // After comma in FROM — another table
    if (last === ",") return { kind: "table", referencedTables };
  }
  if (clauseCtx === "WHERE" || clauseCtx === "HAVING" || clauseCtx === "ON" || clauseCtx === "SET") {
    return { kind: "column", referencedTables };
  }

  // ── Keyword context: start of statement or after semicolon ──
  if (
    trimmed === "" ||
    trimmed.endsWith(";") ||
    last === ";"
  ) {
    return { kind: "keyword", referencedTables };
  }

  // ── Function context: after ( or , in expression position ──
  if (last === "(" || (last === "," && (clauseCtx === "SELECT" || clauseCtx === "WHERE"))) {
    return { kind: "function", referencedTables };
  }

  return { kind: "general", referencedTables };
}

// ── SQL function definitions by category ─────────────────────

interface SqlFunction {
  name: string;
  signature: string;
  description: string;
  category: string;
}

function getSqlFunctions(driver: Driver | ""): SqlFunction[] {
  const fns: SqlFunction[] = [
    // Aggregate
    { name: "COUNT", signature: "COUNT(${1:expr})", description: "Count rows", category: "Aggregate" },
    { name: "SUM", signature: "SUM(${1:expr})", description: "Sum values", category: "Aggregate" },
    { name: "AVG", signature: "AVG(${1:expr})", description: "Average value", category: "Aggregate" },
    { name: "MIN", signature: "MIN(${1:expr})", description: "Minimum value", category: "Aggregate" },
    { name: "MAX", signature: "MAX(${1:expr})", description: "Maximum value", category: "Aggregate" },
    // String
    { name: "UPPER", signature: "UPPER(${1:str})", description: "Uppercase string", category: "String" },
    { name: "LOWER", signature: "LOWER(${1:str})", description: "Lowercase string", category: "String" },
    { name: "TRIM", signature: "TRIM(${1:str})", description: "Trim whitespace", category: "String" },
    { name: "LENGTH", signature: "LENGTH(${1:str})", description: "String length", category: "String" },
    { name: "SUBSTRING", signature: "SUBSTRING(${1:str} FROM ${2:start} FOR ${3:length})", description: "Extract substring", category: "String" },
    { name: "REPLACE", signature: "REPLACE(${1:str}, ${2:from}, ${3:to})", description: "Replace occurrences", category: "String" },
    { name: "CONCAT", signature: "CONCAT(${1:str1}, ${2:str2})", description: "Concatenate strings", category: "String" },
    { name: "POSITION", signature: "POSITION(${1:substr} IN ${2:str})", description: "Find position of substring", category: "String" },
    // Conditional
    { name: "COALESCE", signature: "COALESCE(${1:val1}, ${2:val2})", description: "First non-null value", category: "Conditional" },
    { name: "NULLIF", signature: "NULLIF(${1:val1}, ${2:val2})", description: "NULL if equal", category: "Conditional" },
    { name: "CAST", signature: "CAST(${1:expr} AS ${2:type})", description: "Type cast", category: "Conditional" },
    // Date/Time
    { name: "NOW", signature: "NOW()", description: "Current timestamp", category: "Date" },
    { name: "CURRENT_DATE", signature: "CURRENT_DATE", description: "Current date", category: "Date" },
    { name: "CURRENT_TIMESTAMP", signature: "CURRENT_TIMESTAMP", description: "Current timestamp", category: "Date" },
    { name: "EXTRACT", signature: "EXTRACT(${1:field} FROM ${2:source})", description: "Extract date part", category: "Date" },
    // Math
    { name: "ABS", signature: "ABS(${1:num})", description: "Absolute value", category: "Math" },
    { name: "CEIL", signature: "CEIL(${1:num})", description: "Round up", category: "Math" },
    { name: "FLOOR", signature: "FLOOR(${1:num})", description: "Round down", category: "Math" },
    { name: "ROUND", signature: "ROUND(${1:num}, ${2:decimals})", description: "Round to precision", category: "Math" },
    { name: "POWER", signature: "POWER(${1:base}, ${2:exp})", description: "Power/exponent", category: "Math" },
    // Window
    { name: "ROW_NUMBER", signature: "ROW_NUMBER() OVER (${1:partition})", description: "Row number", category: "Window" },
    { name: "RANK", signature: "RANK() OVER (${1:partition})", description: "Rank with gaps", category: "Window" },
    { name: "DENSE_RANK", signature: "DENSE_RANK() OVER (${1:partition})", description: "Rank without gaps", category: "Window" },
    { name: "LAG", signature: "LAG(${1:expr}, ${2:offset}) OVER (${3:partition})", description: "Previous row value", category: "Window" },
    { name: "LEAD", signature: "LEAD(${1:expr}, ${2:offset}) OVER (${3:partition})", description: "Next row value", category: "Window" },
    { name: "FIRST_VALUE", signature: "FIRST_VALUE(${1:expr}) OVER (${2:partition})", description: "First value in window", category: "Window" },
    { name: "LAST_VALUE", signature: "LAST_VALUE(${1:expr}) OVER (${2:partition})", description: "Last value in window", category: "Window" },
    { name: "NTILE", signature: "NTILE(${1:n}) OVER (${2:partition})", description: "Distribute rows into buckets", category: "Window" },
  ];

  // Dialect-specific
  if (driver === "postgres") {
    fns.push(
      { name: "STRING_AGG", signature: "STRING_AGG(${1:expr}, ${2:delimiter})", description: "Aggregate strings", category: "Aggregate" },
      { name: "ARRAY_AGG", signature: "ARRAY_AGG(${1:expr})", description: "Aggregate into array", category: "Aggregate" },
      { name: "JSON_AGG", signature: "JSON_AGG(${1:expr})", description: "Aggregate into JSON array", category: "Aggregate" },
      { name: "JSONB_AGG", signature: "JSONB_AGG(${1:expr})", description: "Aggregate into JSONB array", category: "Aggregate" },
      { name: "JSON_BUILD_OBJECT", signature: "JSON_BUILD_OBJECT(${1:key}, ${2:value})", description: "Build JSON object", category: "JSON" },
      { name: "JSONB_BUILD_OBJECT", signature: "JSONB_BUILD_OBJECT(${1:key}, ${2:value})", description: "Build JSONB object", category: "JSON" },
      { name: "TO_CHAR", signature: "TO_CHAR(${1:value}, ${2:format})", description: "Format to string", category: "String" },
      { name: "TO_DATE", signature: "TO_DATE(${1:str}, ${2:format})", description: "Parse date string", category: "Date" },
      { name: "TO_TIMESTAMP", signature: "TO_TIMESTAMP(${1:str}, ${2:format})", description: "Parse timestamp string", category: "Date" },
      { name: "DATE_TRUNC", signature: "DATE_TRUNC(${1:field}, ${2:source})", description: "Truncate to precision", category: "Date" },
      { name: "AGE", signature: "AGE(${1:timestamp1}, ${2:timestamp2})", description: "Interval between timestamps", category: "Date" },
      { name: "GENERATE_SERIES", signature: "GENERATE_SERIES(${1:start}, ${2:stop}, ${3:step})", description: "Generate a series of values", category: "Set" },
    );
  } else if (driver === "mysql") {
    fns.push(
      { name: "GROUP_CONCAT", signature: "GROUP_CONCAT(${1:expr} SEPARATOR ${2:','})", description: "Aggregate strings", category: "Aggregate" },
      { name: "IFNULL", signature: "IFNULL(${1:expr}, ${2:default})", description: "Default if null", category: "Conditional" },
      { name: "IF", signature: "IF(${1:cond}, ${2:then}, ${3:else})", description: "Conditional expression", category: "Conditional" },
      { name: "DATE_FORMAT", signature: "DATE_FORMAT(${1:date}, ${2:format})", description: "Format date", category: "Date" },
      { name: "STR_TO_DATE", signature: "STR_TO_DATE(${1:str}, ${2:format})", description: "Parse date string", category: "Date" },
      { name: "DATE_ADD", signature: "DATE_ADD(${1:date}, INTERVAL ${2:n} ${3:DAY})", description: "Add to date", category: "Date" },
      { name: "DATE_SUB", signature: "DATE_SUB(${1:date}, INTERVAL ${2:n} ${3:DAY})", description: "Subtract from date", category: "Date" },
      { name: "JSON_EXTRACT", signature: "JSON_EXTRACT(${1:json}, ${2:path})", description: "Extract JSON value", category: "JSON" },
      { name: "JSON_OBJECT", signature: "JSON_OBJECT(${1:key}, ${2:value})", description: "Create JSON object", category: "JSON" },
    );
  } else if (driver === "mssql") {
    fns.push(
      { name: "STRING_AGG", signature: "STRING_AGG(${1:expr}, ${2:delimiter})", description: "Aggregate strings", category: "Aggregate" },
      { name: "ISNULL", signature: "ISNULL(${1:expr}, ${2:default})", description: "Default if null", category: "Conditional" },
      { name: "IIF", signature: "IIF(${1:cond}, ${2:then}, ${3:else})", description: "Inline if", category: "Conditional" },
      { name: "CONVERT", signature: "CONVERT(${1:type}, ${2:expr})", description: "Type conversion", category: "Conditional" },
      { name: "FORMAT", signature: "FORMAT(${1:value}, ${2:format})", description: "Format value", category: "String" },
      { name: "GETDATE", signature: "GETDATE()", description: "Current date/time", category: "Date" },
      { name: "GETUTCDATE", signature: "GETUTCDATE()", description: "Current UTC date/time", category: "Date" },
      { name: "DATEADD", signature: "DATEADD(${1:part}, ${2:n}, ${3:date})", description: "Add to date", category: "Date" },
      { name: "DATEDIFF", signature: "DATEDIFF(${1:part}, ${2:start}, ${3:end})", description: "Date difference", category: "Date" },
      { name: "DATEPART", signature: "DATEPART(${1:part}, ${2:date})", description: "Extract date part", category: "Date" },
      { name: "JSON_VALUE", signature: "JSON_VALUE(${1:json}, ${2:path})", description: "Extract JSON scalar", category: "JSON" },
      { name: "JSON_QUERY", signature: "JSON_QUERY(${1:json}, ${2:path})", description: "Extract JSON object/array", category: "JSON" },
    );
  }

  return fns;
}

// ── SQL keywords grouped by where they're typically used ──────

const STATEMENT_STARTERS = [
  "SELECT", "INSERT", "UPDATE", "DELETE", "CREATE", "ALTER", "DROP",
  "TRUNCATE", "GRANT", "REVOKE", "BEGIN", "COMMIT", "ROLLBACK",
  "WITH", "EXPLAIN", "ANALYZE", "VACUUM", "MERGE",
];

const CLAUSE_KEYWORDS = [
  "FROM", "WHERE", "AND", "OR", "NOT", "IN", "IS", "NULL",
  "LIKE", "BETWEEN", "EXISTS", "CASE", "WHEN", "THEN", "ELSE", "END",
  "AS", "ON", "JOIN", "INNER", "LEFT", "RIGHT", "FULL", "OUTER", "CROSS",
  "INTO", "VALUES", "SET", "RETURNING", "OUTPUT",
  "GROUP", "BY", "ORDER", "ASC", "DESC", "HAVING", "LIMIT", "OFFSET",
  "UNION", "ALL", "INTERSECT", "EXCEPT", "DISTINCT",
  "OVER", "PARTITION", "ROWS", "RANGE", "PRECEDING", "FOLLOWING",
  "UNBOUNDED", "CURRENT", "ROW", "FETCH", "NEXT", "ONLY", "PERCENT",
  "NATURAL", "USING", "LATERAL", "RECURSIVE",
  "TOP", "WITH", "TIES", "NULLS", "FIRST", "LAST", "FILTER",
  "GROUPING", "SETS", "CUBE", "ROLLUP",
  "CONFLICT", "NOTHING", "EXCLUDED", "DO",
];

const DDL_KEYWORDS = [
  "TABLE", "INDEX", "VIEW", "DATABASE", "SCHEMA", "SEQUENCE",
  "PRIMARY", "KEY", "FOREIGN", "REFERENCES", "UNIQUE", "CHECK", "DEFAULT",
  "CONSTRAINT", "CASCADE", "RESTRICT", "COLUMN", "ADD", "MODIFY", "RENAME",
  "TRIGGER", "PROCEDURE", "FUNCTION", "MATERIALIZED", "CONCURRENTLY",
  "IF", "EXISTS", "NOT", "REPLACE", "TEMPORARY", "TEMP",
  "GENERATED", "ALWAYS", "IDENTITY", "AUTOINCREMENT", "AUTO_INCREMENT",
  "NONCLUSTERED", "CLUSTERED", "INCLUDE", "FILLFACTOR",
  "ENGINE", "CHARSET", "COLLATE", "COMMENT",
];

const DATA_TYPES = [
  "INTEGER", "INT", "BIGINT", "SMALLINT", "TINYINT", "MEDIUMINT",
  "SERIAL", "BIGSERIAL", "SMALLSERIAL",
  "NUMERIC", "DECIMAL", "REAL", "DOUBLE", "PRECISION", "FLOAT",
  "BOOLEAN", "BOOL", "BIT",
  "VARCHAR", "NVARCHAR", "CHAR", "NCHAR", "TEXT", "NTEXT", "CLOB",
  "BYTEA", "BLOB", "BINARY", "VARBINARY", "IMAGE",
  "DATE", "TIME", "TIMESTAMP", "DATETIME", "DATETIME2", "DATETIMEOFFSET",
  "SMALLDATETIME", "INTERVAL", "TIMESTAMPTZ", "TIMETZ",
  "JSON", "JSONB", "XML", "UUID", "UNIQUEIDENTIFIER", "ARRAY",
  "MONEY", "SMALLMONEY",
];

const TRANSACTION_KEYWORDS = [
  "BEGIN", "COMMIT", "ROLLBACK", "SAVEPOINT", "TRANSACTION", "START",
  "RELEASE", "ISOLATION", "LEVEL", "READ", "WRITE",
  "COMMITTED", "UNCOMMITTED", "REPEATABLE", "SERIALIZABLE",
];

const ALL_KEYWORDS = [
  ...new Set([
    ...STATEMENT_STARTERS, ...CLAUSE_KEYWORDS, ...DDL_KEYWORDS,
    ...DATA_TYPES, ...TRANSACTION_KEYWORDS,
  ]),
];

// ── Helper utilities ─────────────────────────────────────────

/** Strip string literals and comments, preserving character positions (replaced with spaces). */
export function stripStringsAndComments(text: string): string {
  let result = "";
  let inSQ = false, inDQ = false, inLC = false, inBC = false;

  for (let i = 0; i < text.length; i++) {
    const ch = text[i];
    const next = text[i + 1];
    if (inBC) { if (ch === "*" && next === "/") { inBC = false; result += "  "; i++; } else { result += " "; } continue; }
    if (inLC) { if (ch === "\n") { inLC = false; result += ch; } else { result += " "; } continue; }
    if (inSQ) { if (ch === "'" && next === "'") { result += "  "; i++; continue; } if (ch === "'") { inSQ = false; result += ch; } else { result += " "; } continue; }
    if (inDQ) { if (ch === '"' && next === '"') { result += "  "; i++; continue; } if (ch === '"') { inDQ = false; result += ch; } else { result += " "; } continue; }
    if (ch === "-" && next === "-") { inLC = true; result += " "; continue; }
    if (ch === "/" && next === "*") { inBC = true; result += " "; i++; continue; }
    if (ch === "'") { inSQ = true; result += ch; continue; }
    if (ch === '"') { inDQ = true; result += ch; continue; }
    result += ch;
  }
  return result;
}

/** Get the last word (identifier) from text */
function getLastWord(text: string): string {
  const m = text.match(/(\w+)\s*$/);
  return m ? m[1] : "";
}

/**
 * Extract the full current SQL statement from the document text,
 * given the cursor offset. Statements are delimited by semicolons.
 */
function getCurrentStatement(strippedFullText: string, cursorOffset: number): string {
  // Find the statement boundaries around the cursor
  let start = 0;
  let end = strippedFullText.length;

  // Search backwards for a semicolon
  for (let i = cursorOffset - 1; i >= 0; i--) {
    if (strippedFullText[i] === ";") {
      start = i + 1;
      break;
    }
  }

  // Search forwards for a semicolon
  for (let i = cursorOffset; i < strippedFullText.length; i++) {
    if (strippedFullText[i] === ";") {
      end = i;
      break;
    }
  }

  return strippedFullText.slice(start, end);
}

/** Extract the last N whitespace-separated tokens from text */
function tokenizeEnd(text: string, n: number): string[] {
  // Split on whitespace and punctuation, keeping words and single punctuation
  const tokens: string[] = [];
  const re = /(\w+|[^\s\w])/g;
  let m;
  while ((m = re.exec(text)) !== null) {
    tokens.push(m[1]);
  }
  return tokens.slice(-n);
}

/** Determine which SQL clause the cursor is currently inside */
function findActiveClause(textBeforeCursor: string): string | null {
  const upper = textBeforeCursor.toUpperCase();
  // Find the last major clause keyword, scanning backwards
  const clauses = [
    "SELECT", "FROM", "WHERE", "GROUP BY", "ORDER BY", "HAVING",
    "SET", "VALUES", "INTO", "ON", "JOIN", "LEFT JOIN", "RIGHT JOIN",
    "INNER JOIN", "FULL JOIN", "CROSS JOIN",
  ];

  let latestIdx = -1;
  let latestClause: string | null = null;

  for (const clause of clauses) {
    // Use word boundary to avoid matching substrings
    const re = new RegExp(`\\b${clause}\\b`, "g");
    let m;
    while ((m = re.exec(upper)) !== null) {
      if (m.index > latestIdx) {
        latestIdx = m.index;
        latestClause = clause;
      }
    }
  }

  return latestClause;
}

/** Extract table references (with aliases) from SQL text */
export function extractReferencedTables(text: string): ReferencedTable[] {
  const tables: ReferencedTable[] = [];

  // Match: FROM/JOIN schema.table alias, FROM/JOIN table alias
  const re = /\b(?:FROM|JOIN)\s+([\w.]+)(?:\s+(?:AS\s+)?(\w+))?/gi;
  let m;
  while ((m = re.exec(text)) !== null) {
    const fullRef = m[1];
    const alias = m[2];
    // Skip if alias is a keyword
    if (alias && isKeyword(alias.toUpperCase())) continue;

    const parts = fullRef.split(".");
    if (parts.length === 2) {
      tables.push({ schema: parts[0], table: parts[1], alias });
    } else {
      tables.push({ table: parts[0], alias });
    }
  }

  // Also match UPDATE table
  const updateRe = /\bUPDATE\s+([\w.]+)(?:\s+(?:AS\s+)?(\w+))?\s+SET\b/gi;
  while ((m = updateRe.exec(text)) !== null) {
    const parts = m[1].split(".");
    const alias = m[2];
    if (parts.length === 2) {
      tables.push({ schema: parts[0], table: parts[1], alias });
    } else {
      tables.push({ table: parts[0], alias });
    }
  }

  return tables;
}

function isKeyword(word: string): boolean {
  return ALL_KEYWORDS.includes(word);
}

// ── Main completion provider ─────────────────────────────────

export function createSqlCompletionProvider(): languages.CompletionItemProvider {
  return {
    triggerCharacters: [".", ",", "("],

    provideCompletionItems: (model, position) => {
      const word = model.getWordUntilPosition(position);
      const range: IRange = {
        startLineNumber: position.lineNumber,
        endLineNumber: position.lineNumber,
        startColumn: word.startColumn,
        endColumn: word.endColumn,
      };

      // Get text from start of document to cursor
      const textBeforeCursor = model.getValueInRange({
        startLineNumber: 1,
        startColumn: 1,
        endLineNumber: position.lineNumber,
        endColumn: position.column,
      });
      const fullText = model.getValue();

      // Detect context
      const ctx = detectContext(textBeforeCursor, fullText);

      // Get driver
      const connStore = useConnectionStore.getState();
      const conn = connStore.connections.find((c) => c.id === connStore.activeConnectionId);
      const driver: Driver | "" = conn?.driver ?? "";

      // Get schema data
      const store = useSchemaStore.getState();

      const suggestions: languages.CompletionItem[] = [];
      let sortBase: number = 0;
      void sortBase;

      // ── Context-specific suggestions (highest priority) ────

      switch (ctx.kind) {
        case "schema": {
          // After "schemaName." → show tables in that schema
          const schemaKey = ctx.schemaName ?? "";
          const schemaTableList = findTablesInSchema(store, schemaKey);
          for (const t of schemaTableList) {
            suggestions.push({
              label: { label: t.name, description: t.tableType },
              kind: 1, // Class
              insertText: t.name,
              detail: `${t.tableType} in ${schemaKey}`,
              range,
              sortText: `0_${t.name}`,
            });
          }
          // Also show routines in that schema
          const schemaRoutines = store.routines[schemaKey] ?? [];
          for (const r of schemaRoutines) {
            suggestions.push({
              label: { label: r.name, description: r.routineType },
              kind: r.routineType === "function" ? 3 : 2, // Function : Method
              insertText: r.name + (r.routineType === "function" ? "(${1})" : ""),
              insertTextRules: 4,
              detail: r.routineType,
              range,
              sortText: `1_${r.name}`,
            });
          }
          return { suggestions };
        }

        case "tableColumn": {
          // After "table." → show columns of that table
          const cols = resolveTableColumns(store, ctx.tableName ?? "", ctx.referencedTables);
          for (const col of cols) {
            suggestions.push({
              label: { label: col.name, description: col.dataType },
              kind: 4, // Field
              insertText: col.name,
              detail: `${col.dataType}${col.isPrimaryKey ? " PK" : ""}${col.isNullable ? "" : " NOT NULL"}`,
              range,
              sortText: `0_${String(col.ordinalPosition).padStart(4, "0")}`,
            });
          }
          return { suggestions };
        }

        case "table": {
          // Show tables (qualified with schema if multiple schemas)
          const multiSchema = store.schemas.length > 1;
          for (const [schemaName, tableList] of Object.entries(store.tables)) {
            for (const t of tableList) {
              const label = multiSchema ? `${schemaName}.${t.name}` : t.name;
              suggestions.push({
                label: { label, description: t.tableType },
                kind: 1,
                insertText: label,
                detail: `${t.tableType}${multiSchema ? ` (${schemaName})` : ""}`,
                range,
                sortText: `0_${t.name}`,
              });
            }
          }
          // Also suggest schema names if multiple schemas
          if (multiSchema) {
            for (const s of store.schemas) {
              suggestions.push({
                label: s.name,
                kind: 9, // Module
                insertText: s.name + ".",
                detail: "schema",
                range,
                sortText: `1_${s.name}`,
                command: { id: "editor.action.triggerSuggest", title: "" },
              });
            }
          }
          sortBase = 2;
          break;
        }

        case "column":
        case "joinCondition": {
          // Show columns — scoped to referenced tables if possible
          const refTables = ctx.referencedTables;
          if (refTables.length > 0) {
            // Show columns from referenced tables, prefixed with alias/table name
            const seen = new Set<string>();
            for (const ref of refTables) {
              const cols = resolveTableColumns(store, ref.table, refTables, ref.schema);
              const prefix = ref.alias || ref.table;
              for (const col of cols) {
                const qualifiedLabel = `${prefix}.${col.name}`;
                if (seen.has(qualifiedLabel)) continue;
                seen.add(qualifiedLabel);
                suggestions.push({
                  label: { label: qualifiedLabel, description: col.dataType },
                  kind: 4,
                  insertText: qualifiedLabel,
                  detail: `${col.dataType}${col.isPrimaryKey ? " PK" : ""}`,
                  range,
                  sortText: `0_${prefix}_${String(col.ordinalPosition).padStart(4, "0")}`,
                });
                // Also add unqualified column name
                if (!seen.has(col.name)) {
                  seen.add(col.name);
                  suggestions.push({
                    label: { label: col.name, description: col.dataType },
                    kind: 4,
                    insertText: col.name,
                    detail: `${col.dataType} (${ref.table})`,
                    range,
                    sortText: `1_${col.name}`,
                  });
                }
              }
            }
            // Also add table aliases for qualified access
            for (const ref of refTables) {
              const name = ref.alias || ref.table;
              suggestions.push({
                label: name,
                kind: 5, // Variable
                insertText: name + ".",
                detail: `alias for ${ref.table}`,
                range,
                sortText: `0A_${name}`,
                command: { id: "editor.action.triggerSuggest", title: "" },
              });
            }
          } else {
            // No referenced tables — show all columns
            for (const [key, colList] of Object.entries(store.columns)) {
              const tableName = key.split(".").pop() ?? key;
              for (const col of colList) {
                suggestions.push({
                  label: { label: col.name, description: col.dataType },
                  kind: 4,
                  insertText: col.name,
                  detail: `${col.dataType} (${tableName})`,
                  range,
                  sortText: `0_${col.name}`,
                });
              }
            }
          }

          // For column context, also add functions (useful in SELECT, WHERE)
          if (ctx.kind === "column") {
            const fns = getSqlFunctions(driver);
            for (const fn of fns) {
              suggestions.push({
                label: { label: fn.name, description: fn.category },
                kind: 3, // Function
                insertText: fn.signature,
                insertTextRules: 4, // InsertAsSnippet
                detail: fn.description,
                range,
                sortText: `2_${fn.name}`,
              });
            }
          }

          // Add * for SELECT context
          const clause = findActiveClause(stripStringsAndComments(textBeforeCursor));
          if (clause === "SELECT") {
            suggestions.push({
              label: "*",
              kind: 14, // Constant
              insertText: "*",
              detail: "All columns",
              range,
              sortText: "0_*",
            });
          }

          sortBase = 3;
          break;
        }

        case "function": {
          const fns = getSqlFunctions(driver);
          for (const fn of fns) {
            suggestions.push({
              label: { label: fn.name, description: fn.category },
              kind: 3,
              insertText: fn.signature,
              insertTextRules: 4,
              detail: fn.description,
              range,
              sortText: `0_${fn.name}`,
            });
          }
          // Also add stored routines
          for (const routineList of Object.values(store.routines)) {
            for (const r of routineList) {
              if (r.routineType !== "function") continue;
              suggestions.push({
                label: { label: r.name, description: "user function" },
                kind: 3,
                insertText: r.name + "(${1})",
                insertTextRules: 4,
                detail: `function (${r.schema})`,
                range,
                sortText: `1_${r.name}`,
              });
            }
          }
          sortBase = 2;
          break;
        }

        case "datatype": {
          for (const dt of DATA_TYPES) {
            suggestions.push({
              label: dt,
              kind: 25, // TypeParameter
              insertText: dt,
              range,
              sortText: `0_${dt}`,
            });
          }
          return { suggestions };
        }

        case "keyword": {
          // Start of statement — suggest statement starters
          for (const kw of STATEMENT_STARTERS) {
            suggestions.push({
              label: kw,
              kind: 17,
              insertText: kw,
              range,
              sortText: `0_${kw}`,
            });
          }
          sortBase = 1;
          break;
        }

        case "general":
        default:
          sortBase = 0;
          break;
      }

      // ── Always-available suggestions (lower priority) ──────
      const kind = ctx.kind as string;

      // Keywords (clause-level)
      if (kind !== "keyword" && kind !== "datatype") {
        for (const kw of CLAUSE_KEYWORDS) {
          suggestions.push({
            label: kw,
            kind: 17,
            insertText: kw,
            range,
            sortText: `${sortBase + 1}_kw_${kw}`,
          });
        }
        // Statement starters at lower priority
        for (const kw of STATEMENT_STARTERS) {
          if (!suggestions.some((s) => typeof s.label === "string" && s.label === kw)) {
            suggestions.push({
              label: kw,
              kind: 17,
              insertText: kw,
              range,
              sortText: `${sortBase + 2}_st_${kw}`,
            });
          }
        }
      }

      // Tables (if not already added as primary suggestions)
      if (kind !== "table" && kind !== "schema" && kind !== "tableColumn") {
        for (const tableList of Object.values(store.tables)) {
          for (const t of tableList) {
            suggestions.push({
              label: t.name,
              kind: 1,
              insertText: t.name,
              detail: t.tableType,
              range,
              sortText: `${sortBase + 1}_tbl_${t.name}`,
            });
          }
        }
      }

      // Columns (if not already added as primary suggestions)
      if (kind !== "column" && kind !== "joinCondition" && kind !== "tableColumn") {
        const seen = new Set<string>();
        for (const colList of Object.values(store.columns)) {
          for (const col of colList) {
            if (seen.has(col.name)) continue;
            seen.add(col.name);
            suggestions.push({
              label: col.name,
              kind: 4,
              insertText: col.name,
              detail: col.dataType,
              range,
              sortText: `${sortBase + 2}_col_${col.name}`,
            });
          }
        }
      }

      // Stored routines
      if (kind !== "function" && kind !== "schema") {
        for (const routineList of Object.values(store.routines)) {
          for (const r of routineList) {
            suggestions.push({
              label: { label: r.name, description: r.routineType },
              kind: r.routineType === "function" ? 3 : 2,
              insertText: r.routineType === "function" ? r.name + "(${1})" : r.name,
              insertTextRules: r.routineType === "function" ? 4 : undefined,
              detail: `${r.routineType} (${r.schema})`,
              range,
              sortText: `${sortBase + 2}_rtn_${r.name}`,
            });
          }
        }
      }

      // Snippets (always available)
      const snippets = useSnippetStore.getState().allSnippets;
      for (const snippet of snippets) {
        suggestions.push({
          label: { label: snippet.prefix, description: snippet.name },
          kind: 27,
          insertText: snippet.body,
          insertTextRules: 4,
          detail: snippet.description ?? snippet.name,
          documentation: snippet.body,
          range,
          sortText: `${sortBase}_snip_${snippet.prefix}`,
        });
      }

      // Deduplicate by label (keep the one with lowest sortText)
      const deduped = deduplicateSuggestions(suggestions);

      return { suggestions: deduped };
    },
  };
}

// ── Schema resolution helpers ─────────────────────────────────

interface SchemaStoreSnapshot {
  schemas: { name: string }[];
  tables: Record<string, { name: string; tableType: string }[]>;
  columns: Record<string, import("../types/schema").ColumnInfo[]>;
  routines: Record<string, import("../types/schema").RoutineInfo[]>;
}

/** Find tables in a specific schema */
function findTablesInSchema(store: SchemaStoreSnapshot, schemaName: string) {
  const key = Object.keys(store.tables).find(
    (k) => k.toUpperCase() === schemaName.toUpperCase(),
  );
  return key ? store.tables[key] : [];
}

/** Resolve columns for a table name or alias */
function resolveTableColumns(
  store: SchemaStoreSnapshot,
  nameOrAlias: string,
  referencedTables: ReferencedTable[],
  specificSchema?: string,
): import("../types/schema").ColumnInfo[] {
  // Find the actual table name (might be an alias)
  const upper = nameOrAlias.toUpperCase();
  const ref = referencedTables.find(
    (r) => (r.alias?.toUpperCase() === upper) || (r.table.toUpperCase() === upper),
  );
  const tableName = ref?.table ?? nameOrAlias;
  const schemaHint = specificSchema ?? ref?.schema;

  // Search columns store: keys are "schema.table"
  for (const [key, cols] of Object.entries(store.columns)) {
    const parts = key.split(".");
    const keyTable = parts[parts.length - 1];
    const keySchema = parts.length > 1 ? parts[0] : undefined;

    if (keyTable.toUpperCase() === tableName.toUpperCase()) {
      if (schemaHint && keySchema && keySchema.toUpperCase() !== schemaHint.toUpperCase()) continue;
      return cols;
    }
  }

  return [];
}

/** Remove duplicate suggestions keeping the highest priority (lowest sortText) */
function deduplicateSuggestions(items: languages.CompletionItem[]): languages.CompletionItem[] {
  const seen = new Map<string, languages.CompletionItem>();
  for (const item of items) {
    const key = typeof item.label === "string" ? item.label : item.label.label;
    const existing = seen.get(key);
    if (!existing || (item.sortText ?? "") < (existing.sortText ?? "")) {
      seen.set(key, item);
    }
  }
  return Array.from(seen.values());
}
