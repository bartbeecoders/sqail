import type { ColumnInfo, ForeignKeyInfo } from "../types/schema";
import type { DiagramTablePosition } from "../types/diagram";

export const TABLE_WIDTH = 240;
export const HEADER_HEIGHT = 32;
export const COLUMN_HEIGHT = 22;

export function tableHeight(columnCount: number): number {
  return HEADER_HEIGHT + Math.max(1, columnCount) * COLUMN_HEIGHT + 8;
}

export function columnY(columnIndex: number): number {
  return HEADER_HEIGHT + columnIndex * COLUMN_HEIGHT + COLUMN_HEIGHT / 2;
}

/**
 * Arrange tables in a grid. Tables with more inbound FKs go to the center.
 * Deterministic, fast, and good enough as a starting layout.
 */
export function autoLayout(
  tables: { schema: string; name: string; columns: ColumnInfo[] }[],
  foreignKeys: ForeignKeyInfo[],
): Record<string, DiagramTablePosition> {
  const PADDING_X = 80;
  const PADDING_Y = 40;
  const cols = Math.ceil(Math.sqrt(Math.max(1, tables.length)));

  // Rank tables by total connection count (FKs + referenced by FKs)
  const connCount = new Map<string, number>();
  for (const fk of foreignKeys) {
    const src = `${fk.sourceSchema}.${fk.sourceTable}`;
    const tgt = `${fk.targetSchema}.${fk.targetTable}`;
    connCount.set(src, (connCount.get(src) ?? 0) + 1);
    connCount.set(tgt, (connCount.get(tgt) ?? 0) + 1);
  }

  const sorted = [...tables].sort((a, b) => {
    const ka = `${a.schema}.${a.name}`;
    const kb = `${b.schema}.${b.name}`;
    return (connCount.get(kb) ?? 0) - (connCount.get(ka) ?? 0);
  });

  const positions: Record<string, DiagramTablePosition> = {};
  let maxRowHeight = 0;
  const rowHeights: number[] = [];

  // Pre-compute row heights
  for (let i = 0; i < sorted.length; i++) {
    const row = Math.floor(i / cols);
    const h = tableHeight(sorted[i].columns.length);
    rowHeights[row] = Math.max(rowHeights[row] ?? 0, h);
    maxRowHeight = Math.max(maxRowHeight, h);
  }

  let yOffset = 40;
  for (let row = 0; row * cols < sorted.length; row++) {
    for (let c = 0; c < cols; c++) {
      const idx = row * cols + c;
      if (idx >= sorted.length) break;
      const t = sorted[idx];
      const key = `${t.schema}.${t.name}`;
      positions[key] = {
        key,
        x: 40 + c * (TABLE_WIDTH + PADDING_X),
        y: yOffset,
      };
    }
    yOffset += (rowHeights[row] ?? maxRowHeight) + PADDING_Y;
  }

  return positions;
}

/** Compute the anchor point on a table for a given column, at the nearer edge. */
export function columnAnchor(
  table: { x: number; y: number; columns: ColumnInfo[] },
  columnName: string,
  otherTable: { x: number } | null,
): { x: number; y: number } {
  const idx = Math.max(
    0,
    table.columns.findIndex((c) => c.name === columnName),
  );
  const y = table.y + columnY(idx);
  if (!otherTable) return { x: table.x + TABLE_WIDTH, y };
  const onRight = otherTable.x > table.x + TABLE_WIDTH / 2;
  return { x: onRight ? table.x + TABLE_WIDTH : table.x, y };
}
