import type { QueryColumn } from "../types/query";

type CellValue = string | number | boolean | null;

const NULL_LITERAL = "NULL";
const COLUMN_GAP = "  ";
const MAX_CELL_WIDTH = 80;

function cellToString(v: CellValue): string {
  if (v === null) return NULL_LITERAL;
  if (typeof v === "boolean") return v ? "true" : "false";
  return String(v);
}

function truncate(s: string): string {
  if (s.length <= MAX_CELL_WIDTH) return s;
  return s.slice(0, MAX_CELL_WIDTH - 1) + "…";
}

/**
 * Render a result set as a fixed-width text table (SSMS "results to text" style).
 * Each column is padded to the wider of its header or its widest visible value,
 * capped at MAX_CELL_WIDTH. NULL is rendered as the literal `NULL`.
 */
export function formatResultAsText(
  columns: QueryColumn[],
  rows: CellValue[][],
): string {
  if (columns.length === 0) return "(no columns)";

  const stringRows: string[][] = rows.map((row) =>
    columns.map((_, i) => truncate(cellToString(row[i] ?? null))),
  );

  const widths = columns.map((col, i) => {
    let w = col.name.length;
    for (const row of stringRows) {
      if (row[i].length > w) w = row[i].length;
    }
    return Math.min(w, MAX_CELL_WIDTH);
  });

  const header = columns.map((c, i) => c.name.padEnd(widths[i])).join(COLUMN_GAP);
  const separator = widths.map((w) => "-".repeat(w)).join(COLUMN_GAP);
  const body = stringRows.map((row) =>
    row.map((s, i) => s.padEnd(widths[i])).join(COLUMN_GAP),
  );
  const footer = `(${rows.length} row${rows.length !== 1 ? "s" : ""})`;

  return [header, separator, ...body, "", footer].join("\n");
}
