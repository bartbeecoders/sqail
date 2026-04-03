import type { editor as monacoEditor } from "monaco-editor";
import { useSchemaStore } from "../stores/schemaStore";

interface SqlError {
  line: number;
  column: number;
  endColumn: number;
  message: string;
  severity: "error" | "warning";
}

// ── Comprehensive SQL keyword set ────────────────────────────
// Used both for unknown-word detection and typo suggestions.

const SQL_KEYWORDS = new Set([
  // DML
  "SELECT", "FROM", "WHERE", "AND", "OR", "NOT", "IN", "IS", "NULL",
  "LIKE", "BETWEEN", "EXISTS", "CASE", "WHEN", "THEN", "ELSE", "END",
  "AS", "ON", "JOIN", "INNER", "LEFT", "RIGHT", "FULL", "OUTER", "CROSS",
  "INSERT", "INTO", "VALUES", "UPDATE", "SET", "DELETE", "MERGE", "USING",
  "MATCHED", "OUTPUT",
  // DDL
  "CREATE", "ALTER", "DROP", "TABLE", "INDEX", "VIEW", "DATABASE", "SCHEMA",
  "PRIMARY", "KEY", "FOREIGN", "REFERENCES", "UNIQUE", "CHECK", "DEFAULT",
  "CONSTRAINT", "CASCADE", "RESTRICT", "COLUMN", "ADD", "MODIFY", "RENAME",
  "TRIGGER", "PROCEDURE", "FUNCTION", "SEQUENCE", "TYPE", "ENUM", "DOMAIN",
  "TABLESPACE", "EXTENSION", "MATERIALIZED", "CONCURRENTLY", "IF",
  // Clauses
  "GROUP", "BY", "ORDER", "ASC", "DESC", "HAVING", "LIMIT", "OFFSET",
  "UNION", "ALL", "INTERSECT", "EXCEPT", "DISTINCT", "TOP", "FETCH",
  "NEXT", "ROWS", "ONLY", "FIRST", "LAST", "PERCENT", "WITH", "TIES",
  "RECURSIVE", "RETURNING", "OVER", "PARTITION", "WINDOW", "RANGE",
  "PRECEDING", "FOLLOWING", "UNBOUNDED", "CURRENT", "ROW",
  // Aggregates & functions
  "COUNT", "SUM", "AVG", "MIN", "MAX", "COALESCE", "NULLIF", "IFNULL",
  "CAST", "CONVERT", "EXTRACT", "SUBSTRING", "TRIM", "UPPER", "LOWER",
  "LENGTH", "REPLACE", "CONCAT", "POSITION", "OVERLAY", "TRANSLATE",
  "NOW", "CURRENT_TIMESTAMP", "CURRENT_DATE", "CURRENT_TIME",
  "GETDATE", "GETUTCDATE", "DATEADD", "DATEDIFF", "DATENAME", "DATEPART",
  "YEAR", "MONTH", "DAY", "HOUR", "MINUTE", "SECOND",
  "ABS", "CEIL", "CEILING", "FLOOR", "ROUND", "POWER", "SQRT", "MOD",
  "SIGN", "RANDOM", "RAND",
  "ROW_NUMBER", "RANK", "DENSE_RANK", "NTILE", "LAG", "LEAD",
  "FIRST_VALUE", "LAST_VALUE", "NTH_VALUE",
  "STRING_AGG", "ARRAY_AGG", "JSON_AGG", "JSONB_AGG", "GROUP_CONCAT",
  "LISTAGG", "XMLAGG",
  // Literals & booleans
  "TRUE", "FALSE", "UNKNOWN",
  // Transaction
  "BEGIN", "COMMIT", "ROLLBACK", "SAVEPOINT", "TRANSACTION", "START",
  "RELEASE", "WORK", "ISOLATION", "LEVEL", "READ", "WRITE", "COMMITTED",
  "UNCOMMITTED", "REPEATABLE", "SERIALIZABLE",
  // Admin
  "GRANT", "REVOKE", "TRUNCATE", "EXPLAIN", "ANALYZE", "VACUUM",
  "REINDEX", "CLUSTER", "REFRESH", "COMMENT", "LOCK", "UNLOCK",
  // Modifiers
  "REPLACE", "TEMPORARY", "TEMP", "EXISTS", "FORCE", "IGNORE",
  "RESTRICT", "NO", "ACTION", "DEFERRABLE", "INITIALLY", "DEFERRED",
  "IMMEDIATE", "ENABLE", "DISABLE", "VALIDATE", "NOVALIDATE",
  // Types
  "SERIAL", "BIGSERIAL", "SMALLSERIAL", "IDENTITY", "AUTOINCREMENT",
  "AUTO_INCREMENT", "GENERATED", "ALWAYS", "STORED", "VIRTUAL",
  "INTEGER", "INT", "BIGINT", "SMALLINT", "TINYINT", "MEDIUMINT",
  "NUMERIC", "DECIMAL", "REAL", "DOUBLE", "PRECISION", "FLOAT",
  "BOOLEAN", "BOOL", "BIT",
  "VARCHAR", "NVARCHAR", "CHAR", "NCHAR", "TEXT", "NTEXT",
  "CLOB", "BYTEA", "BLOB", "BINARY", "VARBINARY", "IMAGE",
  "DATE", "TIME", "TIMESTAMP", "DATETIME", "DATETIME2", "DATETIMEOFFSET",
  "SMALLDATETIME", "INTERVAL", "TIMESTAMPTZ", "TIMETZ",
  "JSON", "JSONB", "XML", "UUID", "UNIQUEIDENTIFIER", "ARRAY",
  "MONEY", "SMALLMONEY", "GEOGRAPHY", "GEOMETRY", "POINT",
  "CIDR", "INET", "MACADDR", "HSTORE", "TSQUERY", "TSVECTOR",
  "REGCLASS", "OID",
  // MSSQL-specific
  "NONCLUSTERED", "CLUSTERED", "INCLUDE", "FILLFACTOR", "GO",
  "NOLOCK", "HOLDLOCK", "ROWLOCK", "TABLOCK", "UPDLOCK",
  "EXEC", "EXECUTE", "DECLARE", "PRINT", "RAISERROR", "THROW",
  "TRY", "CATCH", "WAITFOR", "DELAY", "OPENQUERY", "OPENROWSET",
  "PIVOT", "UNPIVOT", "APPLY", "OPTION", "MAXRECURSION",
  // PostgreSQL-specific
  "ILIKE", "SIMILAR", "LATERAL", "TABLESAMPLE", "BERNOULLI", "SYSTEM",
  "INHERITS", "OWNER", "GRANTED", "PRIVILEGES", "USAGE",
  "DO", "LANGUAGE", "PLPGSQL", "VOLATILE", "STABLE", "IMMUTABLE",
  "RETURNS", "SETOF", "RAISE", "NOTICE", "EXCEPTION", "PERFORM",
  // MySQL-specific
  "ENGINE", "CHARSET", "COLLATE", "UNSIGNED", "ZEROFILL",
  "SHOW", "DESCRIBE", "USE", "DATABASES", "TABLES", "COLUMNS",
  "STATUS", "PROCESSLIST", "VARIABLES", "WARNINGS", "ERRORS",
  "LOAD", "DATA", "INFILE", "OUTFILE", "TERMINATED", "ENCLOSED",
  "ESCAPED", "LINES", "STARTING",
  // Common clauses & misc
  "NATURAL", "STRAIGHT_JOIN", "SOME", "ANY", "EACH", "FOR",
  "OF", "TO", "VARYING", "WITHOUT", "ZONE", "LOCAL",
  "GLOBAL", "SESSION", "SYSTEM_USER", "USER", "PUBLIC", "ROLE",
  "AUTHORIZATION", "COLLATION", "CATALOG", "CONNECT", "PRIVILEGES",
  "REVOKE", "OPTION", "ADMIN", "MEMBER",
  "AFTER", "BEFORE", "INSTEAD", "EACH", "STATEMENT", "REFERENCING",
  "OLD", "NEW", "WHEN",
  "WHILE", "LOOP", "REPEAT", "UNTIL", "LEAVE", "ITERATE", "CURSOR",
  "OPEN", "CLOSE", "FETCH", "HANDLER", "CONTINUE", "EXIT",
  "RETURN", "RETURNS", "CALL", "SIGNAL", "RESIGNAL",
  "ELSEIF", "ELSIF",
  "LIKE", "ILIKE", "ESCAPE", "GLOB", "REGEXP", "RLIKE",
  "LIMIT", "OFFSET", "NULLS", "FILTER",
  "WITHIN", "RESPECT", "PRECEDING", "FOLLOWING",
  "TABLESAMPLE",
  "EXCEPT", "MINUS",
  "HAVING", "QUALIFY",
  "PIVOT", "UNPIVOT",
  "LATERAL",
  "UNNEST", "ORDINALITY",
  "GROUPING", "SETS", "CUBE", "ROLLUP",
  "CONFLICT", "NOTHING", "EXCLUDED",
  "OVERRIDING", "VALUE",
]);

// Words that look like identifiers and should never be flagged (common aliases, etc.)
const SAFE_SINGLE_LETTERS = new Set("ABCDEFGHIJKLMNOPQRSTUVWXYZ".split(""));

/**
 * Lightweight SQL validator that catches common syntax issues.
 * Dialect-aware: adjusts checks based on the database driver.
 */
export function validateSql(text: string, driver?: string): SqlError[] {
  const errors: SqlError[] = [];
  const lines = text.split("\n");

  // Track state across lines
  let inSingleQuote = false;
  let inDoubleQuote = false;
  let inLineComment = false;
  let inBlockComment = false;
  let parenDepth = 0;
  const parenStack: { line: number; col: number }[] = [];
  let stringStartLine = -1;
  let stringStartCol = -1;
  let blockCommentStartLine = -1;

  for (let lineIdx = 0; lineIdx < lines.length; lineIdx++) {
    const line = lines[lineIdx];
    inLineComment = false;

    for (let i = 0; i < line.length; i++) {
      const ch = line[i];
      const next = line[i + 1];

      // Block comment end
      if (inBlockComment) {
        if (ch === "*" && next === "/") {
          inBlockComment = false;
          i++;
        }
        continue;
      }

      // Line comment
      if (inLineComment) continue;

      // String handling
      if (inSingleQuote) {
        if (ch === "'" && next === "'") { i++; continue; } // escaped quote
        if (ch === "'") inSingleQuote = false;
        continue;
      }
      if (inDoubleQuote) {
        if (ch === '"' && next === '"') { i++; continue; }
        if (ch === '"') inDoubleQuote = false;
        continue;
      }

      // Start contexts
      if (ch === "-" && next === "-") { inLineComment = true; continue; }
      if (ch === "/" && next === "*") {
        inBlockComment = true;
        blockCommentStartLine = lineIdx;
        i++;
        continue;
      }
      if (ch === "'") {
        inSingleQuote = true;
        stringStartLine = lineIdx;
        stringStartCol = i;
        continue;
      }
      if (ch === '"') {
        inDoubleQuote = true;
        stringStartLine = lineIdx;
        stringStartCol = i;
        continue;
      }

      // Paren tracking
      if (ch === "(") {
        parenStack.push({ line: lineIdx + 1, col: i + 1 });
        parenDepth++;
      }
      if (ch === ")") {
        if (parenDepth <= 0) {
          errors.push({
            line: lineIdx + 1,
            column: i + 1,
            endColumn: i + 2,
            message: "Unmatched closing parenthesis",
            severity: "error",
          });
        } else {
          parenStack.pop();
          parenDepth--;
        }
      }
    }

    // Unclosed single-line string (SQL doesn't support multi-line string literals)
    if (inSingleQuote && !inBlockComment) {
      errors.push({
        line: stringStartLine + 1,
        column: stringStartCol + 1,
        endColumn: line.length + 1,
        message: "Unclosed string literal (single quote)",
        severity: "error",
      });
      inSingleQuote = false;
    }
  }

  // Unclosed double-quoted identifier
  if (inDoubleQuote) {
    errors.push({
      line: stringStartLine + 1,
      column: stringStartCol + 1,
      endColumn: lines[stringStartLine].length + 1,
      message: "Unclosed quoted identifier (double quote)",
      severity: "error",
    });
  }

  // Unclosed block comment
  if (inBlockComment) {
    errors.push({
      line: blockCommentStartLine + 1,
      column: 1,
      endColumn: lines[blockCommentStartLine].length + 1,
      message: "Unclosed block comment (missing */)",
      severity: "error",
    });
  }

  // Unmatched opening parens
  for (const p of parenStack) {
    errors.push({
      line: p.line,
      column: p.col,
      endColumn: p.col + 1,
      message: "Unmatched opening parenthesis",
      severity: "error",
    });
  }

  // Statement-level checks
  const stripped = stripCommentsAndStrings(text);
  checkStatements(stripped, lines, errors, driver);

  // Unknown word / typo detection
  checkUnknownWords(stripped, lines, errors);

  return errors;
}

// ── Unknown word / typo detection ─────────────────────────────

function checkUnknownWords(stripped: string, _lines: string[], errors: SqlError[]) {
  // Gather known schema identifiers to exclude from flagging
  const schemaNames = new Set<string>();
  try {
    for (const n of useSchemaStore.getState().getAllTableNames()) schemaNames.add(n.toUpperCase());
    for (const n of useSchemaStore.getState().getAllColumnNames()) schemaNames.add(n.toUpperCase());
  } catch {
    // Store not available (e.g., in tests) — skip schema check
  }

  const strippedLines = stripped.split("\n");

  for (let lineIdx = 0; lineIdx < strippedLines.length; lineIdx++) {
    const line = strippedLines[lineIdx];
    // Match bare words (letters, digits, underscore — must start with letter or underscore)
    const wordRe = /\b([A-Za-z_]\w*)\b/g;
    let m;
    while ((m = wordRe.exec(line)) !== null) {
      const word = m[1];
      const wordUpper = word.toUpperCase();

      // Skip if it's a known keyword
      if (SQL_KEYWORDS.has(wordUpper)) continue;

      // Skip single-letter aliases (t, a, b, c, p, etc.)
      if (word.length <= 1 && SAFE_SINGLE_LETTERS.has(wordUpper)) continue;

      // Skip numbers-only after underscore prefix or mixed (identifiers like col1, t2)
      // Skip words that contain digits (very likely identifiers: id1, col2, table3)
      if (/\d/.test(word)) continue;

      // Skip known schema identifiers
      if (schemaNames.has(wordUpper)) continue;

      // Skip short words (2 chars) — too many valid aliases (id, pk, fk, ...)
      if (word.length <= 2) continue;

      // Skip words with underscores — almost certainly identifiers (user_id, created_at)
      if (word.includes("_")) continue;

      // Skip words that are preceded by AS (alias definition)
      const before = line.slice(0, m.index).trimEnd();
      if (/\bAS$/i.test(before)) continue;

      // Skip words preceded by a dot (qualified identifier: schema.table, table.column)
      if (m.index > 0 && line[m.index - 1] === ".") continue;

      // Skip words followed by a dot (schema or table prefix)
      const afterIdx = m.index + word.length;
      if (afterIdx < line.length && line[afterIdx] === ".") continue;

      // Skip if preceded by JOIN ... ON — likely table alias
      if (/\b(?:JOIN|FROM)\s+\S+\s*$/i.test(before)) continue;

      // Now check: is this word close to a known keyword? (likely typo)
      const suggestion = findClosestKeyword(wordUpper);
      if (suggestion) {
        const col = m.index + 1;
        errors.push({
          line: lineIdx + 1,
          column: col,
          endColumn: col + word.length,
          message: `Unknown keyword "${word}" — did you mean ${suggestion}?`,
          severity: "warning",
        });
      }
    }
  }
}

/**
 * Find the closest SQL keyword to a given word using Levenshtein distance.
 * Returns the keyword if distance is within threshold, otherwise null.
 */
function findClosestKeyword(word: string): string | null {
  const len = word.length;
  // Only check words 3+ chars (shorter words produce too many false matches)
  if (len < 3) return null;

  // Threshold scales with word length: 1 for short, 2 for longer
  const maxDist = len <= 5 ? 1 : 2;

  let bestMatch: string | null = null;
  let bestDist = maxDist + 1;

  for (const kw of SQL_KEYWORDS) {
    // Quick length filter — distance can't be less than length difference
    if (Math.abs(kw.length - len) > maxDist) continue;

    const d = levenshtein(word, kw);
    if (d > 0 && d < bestDist) {
      bestDist = d;
      bestMatch = kw;
      if (d === 1) break; // can't do better than 1
    }
  }

  return bestDist <= maxDist ? bestMatch : null;
}

/** Levenshtein edit distance (optimised single-row DP) */
function levenshtein(a: string, b: string): number {
  const m = a.length;
  const n = b.length;
  if (m === 0) return n;
  if (n === 0) return m;

  const row = new Uint16Array(n + 1);
  for (let j = 0; j <= n; j++) row[j] = j;

  for (let i = 1; i <= m; i++) {
    let prev = i - 1;
    row[0] = i;
    for (let j = 1; j <= n; j++) {
      const cost = a[i - 1] === b[j - 1] ? 0 : 1;
      const val = Math.min(row[j] + 1, row[j - 1] + 1, prev + cost);
      prev = row[j];
      row[j] = val;
    }
  }
  return row[n];
}

// ── Statement-level checks ────────────────────────────────────

function checkStatements(stripped: string, lines: string[], errors: SqlError[], driver?: string) {
  const stmts = splitStatements(stripped);

  for (const stmt of stmts) {
    const upper = stmt.text.toUpperCase().trim();
    if (!upper) continue;

    // SELECT without FROM (unless it's a simple expression like SELECT 1, SELECT GETDATE())
    if (upper.startsWith("SELECT") && !upper.includes("FROM")) {
      const afterSelect = upper.slice(6).trim();
      if (afterSelect.includes(",") || afterSelect.length > 50) {
        const lineIdx = findKeywordLine(lines, stmt.startLine, "SELECT");
        if (lineIdx >= 0) {
          errors.push({
            line: lineIdx + 1,
            column: 1,
            endColumn: lines[lineIdx].length + 1,
            message: "SELECT without FROM clause",
            severity: "warning",
          });
        }
      }
    }

    // INSERT without INTO
    if (upper.startsWith("INSERT") && !upper.includes("INTO")) {
      errors.push({
        line: stmt.startLine + 1,
        column: 1,
        endColumn: lines[stmt.startLine]?.length + 1 || 1,
        message: "INSERT without INTO",
        severity: "error",
      });
    }

    // UPDATE without SET
    if (upper.startsWith("UPDATE") && !upper.includes("SET")) {
      errors.push({
        line: stmt.startLine + 1,
        column: 1,
        endColumn: lines[stmt.startLine]?.length + 1 || 1,
        message: "UPDATE without SET clause",
        severity: "error",
      });
    }

    // DELETE without FROM
    if (upper.startsWith("DELETE") && !upper.includes("FROM")) {
      errors.push({
        line: stmt.startLine + 1,
        column: 1,
        endColumn: lines[stmt.startLine]?.length + 1 || 1,
        message: "DELETE without FROM clause",
        severity: "error",
      });
    }

    // Dialect-specific: LIMIT used in MSSQL (should use TOP)
    if (driver === "mssql" && upper.includes("LIMIT")) {
      const lineIdx = findKeywordLine(lines, stmt.startLine, "LIMIT");
      if (lineIdx >= 0) {
        const col = lines[lineIdx].toUpperCase().indexOf("LIMIT") + 1;
        errors.push({
          line: lineIdx + 1,
          column: col,
          endColumn: col + 5,
          message: "LIMIT is not supported in SQL Server — use TOP instead",
          severity: "error",
        });
      }
    }

    // Dialect-specific: TOP used in non-MSSQL
    if (driver && driver !== "mssql" && /\bTOP\s+\d/.test(upper)) {
      const lineIdx = findKeywordLine(lines, stmt.startLine, "TOP");
      if (lineIdx >= 0) {
        const col = lines[lineIdx].toUpperCase().indexOf("TOP") + 1;
        errors.push({
          line: lineIdx + 1,
          column: col,
          endColumn: col + 3,
          message: `TOP is not supported in ${driverLabel(driver)} — use LIMIT instead`,
          severity: "error",
        });
      }
    }

    // Dialect-specific: ILIKE only works in PostgreSQL
    if (driver && driver !== "postgres" && upper.includes("ILIKE")) {
      const lineIdx = findKeywordLine(lines, stmt.startLine, "ILIKE");
      if (lineIdx >= 0) {
        const col = lines[lineIdx].toUpperCase().indexOf("ILIKE") + 1;
        errors.push({
          line: lineIdx + 1,
          column: col,
          endColumn: col + 5,
          message: `ILIKE is not supported in ${driverLabel(driver)} — use LOWER() with LIKE instead`,
          severity: "error",
        });
      }
    }

    // Dialect-specific: RETURNING only works in PostgreSQL (and SQLite 3.35+)
    if (driver === "mysql" || driver === "mssql") {
      if (upper.includes("RETURNING")) {
        const lineIdx = findKeywordLine(lines, stmt.startLine, "RETURNING");
        if (lineIdx >= 0) {
          const col = lines[lineIdx].toUpperCase().indexOf("RETURNING") + 1;
          const hint = driver === "mssql" ? "use OUTPUT instead" : "not supported in MySQL";
          errors.push({
            line: lineIdx + 1,
            column: col,
            endColumn: col + 9,
            message: `RETURNING is not supported in ${driverLabel(driver)} — ${hint}`,
            severity: "error",
          });
        }
      }
    }

    // Dialect-specific: backtick quoting in PostgreSQL/MSSQL
    if ((driver === "postgres" || driver === "mssql") && stmt.text.includes("`")) {
      const lineIdx = findKeywordLine(lines, stmt.startLine, "`");
      if (lineIdx >= 0) {
        const col = lines[lineIdx].indexOf("`") + 1;
        const hint = driver === "postgres" ? 'use double quotes (")' : "use square brackets ([])";
        errors.push({
          line: lineIdx + 1,
          column: col,
          endColumn: col + 1,
          message: `Backtick quoting is not supported in ${driverLabel(driver)} — ${hint}`,
          severity: "warning",
        });
      }
    }

    // Trailing comma before FROM/WHERE/GROUP/ORDER/HAVING
    const trailingCommaRe = /,\s*(FROM|WHERE|GROUP|ORDER|HAVING|LIMIT|SET|VALUES)\b/gi;
    let match;
    while ((match = trailingCommaRe.exec(stripped)) !== null) {
      const pos = match.index;
      const beforePos = stripped.slice(0, pos);
      const lineIdx = beforePos.split("\n").length - 1;
      if (lineIdx >= stmt.startLine) {
        const lineText = lines[lineIdx] ?? "";
        const col = lineText.indexOf(",", 0) + 1 || 1;
        errors.push({
          line: lineIdx + 1,
          column: col,
          endColumn: col + 1,
          message: `Trailing comma before ${match[1]}`,
          severity: "error",
        });
      }
    }
  }
}

function driverLabel(driver: string): string {
  switch (driver) {
    case "postgres": return "PostgreSQL";
    case "mysql": return "MySQL";
    case "mssql": return "SQL Server";
    case "sqlite": return "SQLite";
    default: return driver;
  }
}

function stripCommentsAndStrings(text: string): string {
  let result = "";
  let inSQ = false, inDQ = false, inLC = false, inBC = false;

  for (let i = 0; i < text.length; i++) {
    const ch = text[i];
    const next = text[i + 1];

    if (inBC) {
      if (ch === "*" && next === "/") { inBC = false; result += "  "; i++; } else { result += " "; }
      continue;
    }
    if (inLC) {
      if (ch === "\n") { inLC = false; result += ch; } else { result += " "; }
      continue;
    }
    if (inSQ) {
      if (ch === "'" && next === "'") { result += "  "; i++; continue; }
      if (ch === "'") { inSQ = false; result += ch; } else { result += " "; }
      continue;
    }
    if (inDQ) {
      if (ch === '"' && next === '"') { result += "  "; i++; continue; }
      if (ch === '"') { inDQ = false; result += ch; } else { result += " "; }
      continue;
    }

    if (ch === "-" && next === "-") { inLC = true; result += " "; continue; }
    if (ch === "/" && next === "*") { inBC = true; result += " "; i++; continue; }
    if (ch === "'") { inSQ = true; result += ch; continue; }
    if (ch === '"') { inDQ = true; result += ch; continue; }
    result += ch;
  }
  return result;
}

function splitStatements(stripped: string): { text: string; startLine: number }[] {
  const stmts: { text: string; startLine: number }[] = [];
  let current = "";
  let startLine = 0;
  let lineNum = 0;

  for (const ch of stripped) {
    if (ch === "\n") lineNum++;
    if (ch === ";") {
      if (current.trim()) stmts.push({ text: current, startLine });
      current = "";
      startLine = lineNum + 1;
    } else {
      if (!current.trim() && ch.trim()) startLine = lineNum;
      current += ch;
    }
  }
  if (current.trim()) stmts.push({ text: current, startLine });
  return stmts;
}

function findKeywordLine(lines: string[], startLine: number, keyword: string): number {
  for (let i = startLine; i < lines.length; i++) {
    if (lines[i].toUpperCase().includes(keyword.toUpperCase())) return i;
  }
  return startLine;
}

/** Convert SqlErrors to Monaco markers */
export function toMonacoMarkers(errors: SqlError[]): monacoEditor.IMarkerData[] {
  return errors.map((e) => ({
    startLineNumber: e.line,
    startColumn: e.column,
    endLineNumber: e.line,
    endColumn: e.endColumn,
    message: e.message,
    severity: e.severity === "error" ? 8 : 4, // MarkerSeverity.Error : MarkerSeverity.Warning
  }));
}
