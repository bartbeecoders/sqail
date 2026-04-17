/**
 * Token-budget-aware schema context builder for inline AI completion.
 *
 * The full-schema dump in `schemaContext.ts` is fine for Ctrl+K one-shot
 * prompts but way too big for per-keystroke FIM calls. This helper keeps
 * the prompt lean by including only:
 *
 *   1. Tables referenced in the current statement (via
 *      `extractReferencedTables`), columns inlined.
 *   2. If no tables are referenced yet, a small header listing the
 *      most prominent tables the user has open (first 6 across schemas).
 *   3. Driver / dialect label on the first line.
 *
 * Hard cap: 1500 characters. Columns are dropped in order of
 * "dispensability" (TEXT/BLOB bodies first, then non-null non-PK
 * non-FK) until we fit.
 */

import { extractReferencedTables, stripStringsAndComments } from "./sqlCompletions";
import { useSchemaStore } from "../stores/schemaStore";
import type { ColumnInfo, TableInfo } from "../types/schema";
import type { Driver } from "../types/connection";

const MAX_CONTEXT_CHARS = 1500;

export interface InlineContext {
  /** The rendered prompt prefix. Empty when schema isn't loaded. */
  prefix: string;
  /** Stable hash used for the LRU cache key. */
  hash: string;
}

interface CandidateTable {
  schema: string;
  table: string;
  columns: ColumnInfo[];
}

/**
 * Build the schema context block that goes in front of the user's code
 * when we send a FIM request.
 *
 * @param statementText The full SQL statement surrounding the cursor.
 *                      Pass the *unstripped* text — we'll strip strings
 *                      and comments internally before pattern matching.
 * @param driver        Active connection driver (for the dialect hint).
 */
export function buildInlineContext(
  statementText: string,
  driver: Driver | "",
): InlineContext {
  const schemaState = useSchemaStore.getState();
  if (schemaState.schemas.length === 0) {
    return { prefix: "", hash: "no-schema" };
  }

  const stripped = stripStringsAndComments(statementText);
  const refs = extractReferencedTables(stripped);

  const candidates: CandidateTable[] = [];
  const seen = new Set<string>();

  // Referenced tables first.
  for (const ref of refs) {
    const t = findTable(ref.schema, ref.table);
    if (!t) continue;
    const key = `${t.schema}.${t.name}`;
    if (seen.has(key)) continue;
    seen.add(key);
    candidates.push({
      schema: t.schema,
      table: t.name,
      columns: schemaState.columns[key] ?? [],
    });
  }

  // If nothing referenced yet, fall back to the first handful of tables
  // the schema tree has loaded — gives the model something concrete to
  // anchor on when the user is still typing `SELECT … FROM `.
  if (candidates.length === 0) {
    const fallback = collectFallbackTables(6);
    for (const c of fallback) {
      const key = `${c.schema}.${c.table}`;
      if (seen.has(key)) continue;
      seen.add(key);
      candidates.push(c);
    }
  }

  const lines: string[] = [];
  lines.push(`-- Dialect: ${driverLabel(driver)}`);
  if (candidates.length > 0) {
    lines.push("-- Tables in scope:");
    for (const c of candidates) {
      lines.push(renderTableLine(c));
    }
  }

  let prefix = lines.join("\n");
  if (prefix.length > MAX_CONTEXT_CHARS) {
    prefix = shrink(prefix, candidates);
  }

  return { prefix, hash: djb2(prefix) };
}

function findTable(
  schema: string | undefined,
  table: string,
): { schema: string; name: string; tableType: TableInfo["tableType"] } | null {
  const { schemas, tables } = useSchemaStore.getState();
  const schemaNames = schema
    ? [schema]
    : schemas.map((s) => s.name);
  for (const sName of schemaNames) {
    const list = tables[sName] ?? [];
    const match = list.find((t) => t.name.toLowerCase() === table.toLowerCase());
    if (match) return { schema: sName, name: match.name, tableType: match.tableType };
  }
  return null;
}

function collectFallbackTables(limit: number): CandidateTable[] {
  const { schemas, tables, columns } = useSchemaStore.getState();
  const out: CandidateTable[] = [];
  for (const s of schemas) {
    const list = tables[s.name] ?? [];
    for (const t of list) {
      if (out.length >= limit) return out;
      out.push({
        schema: s.name,
        table: t.name,
        columns: columns[`${s.name}.${t.name}`] ?? [],
      });
    }
  }
  return out;
}

function renderTableLine(c: CandidateTable): string {
  const qualified = `${c.schema}.${c.table}`;
  if (c.columns.length === 0) return `-- ${qualified}`;
  const parts = c.columns.map(renderColumn);
  return `-- ${qualified}(${parts.join(", ")})`;
}

function renderColumn(col: ColumnInfo): string {
  let def = `${col.name} ${col.dataType}`;
  if (col.isPrimaryKey) def += " PK";
  if (!col.isNullable) def += " NOT NULL";
  return def;
}

/**
 * If the rendered context is over budget, aggressively shrink it:
 * 1. Drop TEXT/BLOB/JSON column bodies first (most dispensable).
 * 2. Then drop column lists beyond the top 6 columns per table.
 * 3. Finally truncate the whole string with an ellipsis comment.
 */
function shrink(prefix: string, candidates: CandidateTable[]): string {
  if (candidates.length === 0) return prefix.slice(0, MAX_CONTEXT_CHARS);

  const isBulky = (ty: string) => /\b(TEXT|BLOB|BYTEA|JSON|CLOB|VARCHAR\()/i.test(ty);

  // Pass 1: drop bulky columns entirely.
  const trimmed1 = candidates.map((c) => ({
    ...c,
    columns: c.columns.filter((col) => !isBulky(col.dataType)),
  }));
  let out = renderAll(trimmed1);
  if (out.length <= MAX_CONTEXT_CHARS) return out;

  // Pass 2: keep only the first 6 columns per table (but always PKs).
  const trimmed2 = trimmed1.map((c) => ({
    ...c,
    columns: [
      ...c.columns.filter((col) => col.isPrimaryKey),
      ...c.columns.filter((col) => !col.isPrimaryKey).slice(0, 6),
    ],
  }));
  out = renderAll(trimmed2);
  if (out.length <= MAX_CONTEXT_CHARS) return out;

  // Pass 3: hard truncate.
  return out.slice(0, MAX_CONTEXT_CHARS - 20).trimEnd() + "\n-- …truncated";
}

function renderAll(candidates: CandidateTable[]): string {
  return [
    "-- Tables in scope:",
    ...candidates.map((c) => renderTableLine(c)),
  ].join("\n");
}

function driverLabel(driver: Driver | ""): string {
  switch (driver) {
    case "postgres": return "PostgreSQL";
    case "mysql":    return "MySQL";
    case "sqlite":   return "SQLite";
    case "mssql":    return "Microsoft SQL Server";
    default:         return "SQL";
  }
}

function djb2(s: string): string {
  let h = 5381;
  for (let i = 0; i < s.length; i++) h = ((h << 5) + h + s.charCodeAt(i)) | 0;
  return (h >>> 0).toString(36);
}
