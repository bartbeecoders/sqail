import type { QueryColumn } from "../types/query";

type CellValue = string | number | boolean | null;

function cellToString(value: CellValue): string {
  if (value === null) return "";
  return String(value);
}

// ── CSV ────────────────────────────────────────────────────

function escapeCsvField(value: string): string {
  if (value.includes(",") || value.includes('"') || value.includes("\n") || value.includes("\r")) {
    return `"${value.replace(/"/g, '""')}"`;
  }
  return value;
}

export function toCsv(columns: QueryColumn[], rows: CellValue[][]): string {
  const header = columns.map((c) => escapeCsvField(c.name)).join(",");
  const body = rows.map((row) => row.map((v) => escapeCsvField(cellToString(v))).join(","));
  return [header, ...body].join("\r\n");
}

// ── JSON ───────────────────────────────────────────────────

export function toJson(columns: QueryColumn[], rows: CellValue[][]): string {
  const objects = rows.map((row) => {
    const obj: Record<string, CellValue> = {};
    columns.forEach((col, i) => {
      obj[col.name] = row[i];
    });
    return obj;
  });
  return JSON.stringify(objects, null, 2);
}

// ── XML ────────────────────────────────────────────────────

function escapeXml(value: string): string {
  return value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&apos;");
}

function toXmlTag(name: string): string {
  // XML tag names: replace invalid chars with underscores, ensure starts with letter
  let tag = name.replace(/[^a-zA-Z0-9_.-]/g, "_");
  if (!/^[a-zA-Z_]/.test(tag)) tag = `_${tag}`;
  return tag;
}

export function toXml(columns: QueryColumn[], rows: CellValue[][]): string {
  const lines: string[] = ['<?xml version="1.0" encoding="UTF-8"?>', "<results>"];
  for (const row of rows) {
    lines.push("  <row>");
    columns.forEach((col, i) => {
      const tag = toXmlTag(col.name);
      const val = row[i];
      if (val === null) {
        lines.push(`    <${tag} xsi:nil="true" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"/>`);
      } else {
        lines.push(`    <${tag}>${escapeXml(String(val))}</${tag}>`);
      }
    });
    lines.push("  </row>");
  }
  lines.push("</results>");
  return lines.join("\n");
}

// ── Excel (XLSX via simple XML spreadsheet) ────────────────

function escapeXmlContent(value: string): string {
  return escapeXml(value);
}

export function toExcelXml(columns: QueryColumn[], rows: CellValue[][]): string {
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
    '  </Styles>',
    '  <Worksheet ss:Name="Results">',
    '    <Table>',
  ];

  // Header row
  lines.push("      <Row>");
  for (const col of columns) {
    lines.push(
      `        <Cell ss:StyleID="header"><Data ss:Type="String">${escapeXmlContent(col.name)}</Data></Cell>`,
    );
  }
  lines.push("      </Row>");

  // Data rows
  for (const row of rows) {
    lines.push("      <Row>");
    for (let i = 0; i < columns.length; i++) {
      const val = row[i];
      if (val === null) {
        lines.push("        <Cell/>");
      } else if (typeof val === "number") {
        lines.push(`        <Cell><Data ss:Type="Number">${val}</Data></Cell>`);
      } else if (typeof val === "boolean") {
        lines.push(
          `        <Cell><Data ss:Type="Boolean">${val ? "1" : "0"}</Data></Cell>`,
        );
      } else {
        lines.push(
          `        <Cell><Data ss:Type="String">${escapeXmlContent(String(val))}</Data></Cell>`,
        );
      }
    }
    lines.push("      </Row>");
  }

  lines.push("    </Table>");
  lines.push("  </Worksheet>");
  lines.push("</Workbook>");
  return lines.join("\n");
}

// ── Download helper ────────────────────────────────────────

export type ExportFormat = "csv" | "json" | "xml" | "excel";

const FORMAT_CONFIG: Record<ExportFormat, { mime: string; ext: string }> = {
  csv: { mime: "text/csv;charset=utf-8", ext: "csv" },
  json: { mime: "application/json;charset=utf-8", ext: "json" },
  xml: { mime: "application/xml;charset=utf-8", ext: "xml" },
  excel: { mime: "application/vnd.ms-excel", ext: "xls" },
};

export function exportResults(
  format: ExportFormat,
  columns: QueryColumn[],
  rows: CellValue[][],
  filename = "results",
): void {
  let content: string;
  switch (format) {
    case "csv":
      content = toCsv(columns, rows);
      break;
    case "json":
      content = toJson(columns, rows);
      break;
    case "xml":
      content = toXml(columns, rows);
      break;
    case "excel":
      content = toExcelXml(columns, rows);
      break;
  }

  const { mime, ext } = FORMAT_CONFIG[format];
  const blob = new Blob([content], { type: mime });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = `${filename}.${ext}`;
  document.body.appendChild(a);
  a.click();
  document.body.removeChild(a);
  URL.revokeObjectURL(url);
}
