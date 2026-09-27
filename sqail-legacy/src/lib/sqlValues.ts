type CellValue = string | number | boolean | null;

export function valueToSqlLiteral(v: CellValue): string {
  if (v === null) return "NULL";
  if (typeof v === "number") return Number.isFinite(v) ? String(v) : "NULL";
  if (typeof v === "boolean") return v ? "TRUE" : "FALSE";
  return `'${String(v).replace(/'/g, "''")}'`;
}

/** Render values as a comma-separated SQL literal list, deduped, preserving order. */
export function valuesToInList(values: CellValue[]): string {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const v of values) {
    const lit = valueToSqlLiteral(v);
    if (seen.has(lit)) continue;
    seen.add(lit);
    out.push(lit);
  }
  return out.join(", ");
}
