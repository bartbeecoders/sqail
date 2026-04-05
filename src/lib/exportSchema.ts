import type { TableInfo, ColumnInfo, IndexInfo } from "../types/schema";
import type { ObjectMetadata } from "../types/metadata";

export type SchemaExportFormat = "markdown" | "excel";

interface ExportableObject {
  schema: string;
  name: string;
  type: "table" | "view" | "function" | "procedure";
  columns?: ColumnInfo[];
  indexes?: IndexInfo[];
  metadata?: ObjectMetadata;
}

/** Collect all filtered objects into a flat list for export. */
export function collectExportObjects(
  tableGroups: { schema: string; items: TableInfo[] }[],
  viewGroups: { schema: string; items: TableInfo[] }[],
  routineGroups: { schema: string; items: { name: string; schema: string; routineType: "function" | "procedure" }[] }[],
  columns: Record<string, ColumnInfo[]>,
  indexes: Record<string, IndexInfo[]>,
  getMetadata: (schema: string, name: string) => ObjectMetadata | undefined,
): ExportableObject[] {
  const objects: ExportableObject[] = [];

  for (const group of tableGroups) {
    for (const t of group.items) {
      const key = `${group.schema}.${t.name}`;
      objects.push({
        schema: group.schema,
        name: t.name,
        type: "table",
        columns: columns[key],
        indexes: indexes[key],
        metadata: getMetadata(group.schema, t.name),
      });
    }
  }

  for (const group of viewGroups) {
    for (const v of group.items) {
      const key = `${group.schema}.${v.name}`;
      objects.push({
        schema: group.schema,
        name: v.name,
        type: "view",
        columns: columns[key],
        indexes: indexes[key],
        metadata: getMetadata(group.schema, v.name),
      });
    }
  }

  for (const group of routineGroups) {
    for (const r of group.items) {
      objects.push({
        schema: group.schema,
        name: r.name,
        type: r.routineType,
        metadata: getMetadata(group.schema, r.name),
      });
    }
  }

  return objects;
}

// ── Markdown ─────────────────────────────────────────────

function escMd(s: string): string {
  return s.replace(/\|/g, "\\|").replace(/\n/g, " ");
}

export function toSchemaMarkdown(objects: ExportableObject[]): string {
  const lines: string[] = [];
  lines.push("# Database Schema Export");
  lines.push("");
  lines.push(`*Exported ${objects.length} object(s) on ${new Date().toISOString().slice(0, 10)}*`);
  lines.push("");

  // Group by schema
  const bySchema = new Map<string, ExportableObject[]>();
  for (const obj of objects) {
    const list = bySchema.get(obj.schema) || [];
    list.push(obj);
    bySchema.set(obj.schema, list);
  }

  for (const [schema, objs] of bySchema) {
    lines.push(`## Schema: ${schema}`);
    lines.push("");

    for (const obj of objs) {
      const typeLabel = obj.type.charAt(0).toUpperCase() + obj.type.slice(1);
      lines.push(`### ${typeLabel}: ${obj.name}`);
      lines.push("");

      // AI metadata
      if (obj.metadata?.metadata) {
        const m = obj.metadata.metadata;
        if (m.description) {
          lines.push(`> ${escMd(m.description)}`);
          lines.push("");
        }
        if (m.exampleUsage) {
          lines.push("**Example Usage:**");
          lines.push("```sql");
          lines.push(m.exampleUsage);
          lines.push("```");
          lines.push("");
        }
        if (m.relatedObjects?.length) {
          lines.push(`**Related Objects:** ${m.relatedObjects.join(", ")}`);
          lines.push("");
        }
        if (m.dependencies?.length) {
          lines.push(`**Dependencies:** ${m.dependencies.join(", ")}`);
          lines.push("");
        }
      }

      // Columns
      if (obj.columns && obj.columns.length > 0) {
        lines.push("| Column | Type | Nullable | Default | PK | Description |");
        lines.push("| --- | --- | --- | --- | --- | --- |");
        for (const col of obj.columns) {
          const colMeta = obj.metadata?.metadata?.columns?.find((c) => c.name === col.name);
          lines.push(
            `| ${escMd(col.name)} | ${escMd(col.dataType)} | ${col.isNullable ? "Yes" : "No"} | ${col.columnDefault ? escMd(col.columnDefault) : "-"} | ${col.isPrimaryKey ? "Yes" : "-"} | ${colMeta?.description ? escMd(colMeta.description) : "-"} |`,
          );
        }
        lines.push("");
      }

      // Indexes
      if (obj.indexes && obj.indexes.length > 0) {
        lines.push("**Indexes:**");
        lines.push("");
        lines.push("| Name | Unique | Columns |");
        lines.push("| --- | --- | --- |");
        for (const idx of obj.indexes) {
          lines.push(`| ${escMd(idx.name)} | ${idx.isUnique ? "Yes" : "No"} | ${idx.columns.map(escMd).join(", ")} |`);
        }
        lines.push("");
      }

      lines.push("---");
      lines.push("");
    }
  }

  return lines.join("\n");
}

// ── Excel (XML Spreadsheet) ──────────────────────────────

function escXml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function cell(value: string, styleId?: string): string {
  const style = styleId ? ` ss:StyleID="${styleId}"` : "";
  return `        <Cell${style}><Data ss:Type="String">${escXml(value)}</Data></Cell>`;
}

export function toSchemaExcel(objects: ExportableObject[]): string {
  const lines: string[] = [
    '<?xml version="1.0" encoding="UTF-8"?>',
    '<?mso-application progid="Excel.Sheet"?>',
    '<Workbook xmlns="urn:schemas-microsoft-com:office:spreadsheet"',
    '  xmlns:ss="urn:schemas-microsoft-com:office:spreadsheet">',
    '  <Styles>',
    '    <Style ss:ID="header">',
    '      <Font ss:Bold="1"/>',
    '      <Interior ss:Color="#E2E8F0" ss:Pattern="Solid"/>',
    '    </Style>',
    '    <Style ss:ID="section">',
    '      <Font ss:Bold="1" ss:Size="12"/>',
    '      <Interior ss:Color="#CBD5E1" ss:Pattern="Solid"/>',
    '    </Style>',
    '  </Styles>',
  ];

  // ── Objects overview sheet ──
  lines.push('  <Worksheet ss:Name="Objects">');
  lines.push("    <Table>");
  lines.push("      <Row>");
  for (const h of ["Schema", "Name", "Type", "Description", "Related Objects", "Dependencies"]) {
    lines.push(cell(h, "header"));
  }
  lines.push("      </Row>");
  for (const obj of objects) {
    const m = obj.metadata?.metadata;
    lines.push("      <Row>");
    lines.push(cell(obj.schema));
    lines.push(cell(obj.name));
    lines.push(cell(obj.type));
    lines.push(cell(m?.description ?? ""));
    lines.push(cell(m?.relatedObjects?.join(", ") ?? ""));
    lines.push(cell(m?.dependencies?.join(", ") ?? ""));
    lines.push("      </Row>");
  }
  lines.push("    </Table>");
  lines.push("  </Worksheet>");

  // ── Columns sheet ──
  lines.push('  <Worksheet ss:Name="Columns">');
  lines.push("    <Table>");
  lines.push("      <Row>");
  for (const h of ["Schema", "Object", "Object Type", "Column", "Data Type", "Nullable", "Default", "PK", "Description"]) {
    lines.push(cell(h, "header"));
  }
  lines.push("      </Row>");
  for (const obj of objects) {
    if (!obj.columns) continue;
    for (const col of obj.columns) {
      const colMeta = obj.metadata?.metadata?.columns?.find((c) => c.name === col.name);
      lines.push("      <Row>");
      lines.push(cell(obj.schema));
      lines.push(cell(obj.name));
      lines.push(cell(obj.type));
      lines.push(cell(col.name));
      lines.push(cell(col.dataType));
      lines.push(cell(col.isNullable ? "Yes" : "No"));
      lines.push(cell(col.columnDefault ?? ""));
      lines.push(cell(col.isPrimaryKey ? "Yes" : ""));
      lines.push(cell(colMeta?.description ?? ""));
      lines.push("      </Row>");
    }
  }
  lines.push("    </Table>");
  lines.push("  </Worksheet>");

  // ── Indexes sheet ──
  lines.push('  <Worksheet ss:Name="Indexes">');
  lines.push("    <Table>");
  lines.push("      <Row>");
  for (const h of ["Schema", "Object", "Index Name", "Unique", "Columns"]) {
    lines.push(cell(h, "header"));
  }
  lines.push("      </Row>");
  for (const obj of objects) {
    if (!obj.indexes) continue;
    for (const idx of obj.indexes) {
      lines.push("      <Row>");
      lines.push(cell(obj.schema));
      lines.push(cell(obj.name));
      lines.push(cell(idx.name));
      lines.push(cell(idx.isUnique ? "Yes" : "No"));
      lines.push(cell(idx.columns.join(", ")));
      lines.push("      </Row>");
    }
  }
  lines.push("    </Table>");
  lines.push("  </Worksheet>");

  lines.push("</Workbook>");
  return lines.join("\n");
}

// ── Save helper (Tauri dialog + fs) ─────────────────────

import { save } from "@tauri-apps/plugin-dialog";
import { writeTextFile } from "@tauri-apps/plugin-fs";

const FORMAT_CONFIG: Record<SchemaExportFormat, { ext: string; filterName: string }> = {
  markdown: { ext: "md", filterName: "Markdown Files" },
  excel: { ext: "xls", filterName: "Excel Files" },
};

export async function exportSchema(
  format: SchemaExportFormat,
  objects: ExportableObject[],
  filename = "schema-export",
): Promise<void> {
  const { ext, filterName } = FORMAT_CONFIG[format];
  const content = format === "markdown" ? toSchemaMarkdown(objects) : toSchemaExcel(objects);

  const filePath = await save({
    filters: [{ name: filterName, extensions: [ext] }],
    defaultPath: `${filename}.${ext}`,
  });

  if (!filePath) return; // user cancelled
  await writeTextFile(filePath, content);
}
