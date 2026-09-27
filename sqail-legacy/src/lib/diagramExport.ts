import { save } from "@tauri-apps/plugin-dialog";
import { writeFile } from "@tauri-apps/plugin-fs";
import type { ColumnInfo, ForeignKeyInfo, TableInfo } from "../types/schema";
import type { DiagramState } from "../types/diagram";
import { TABLE_WIDTH, tableHeight, COLUMN_HEIGHT, HEADER_HEIGHT } from "./diagramLayout";

/** Serialize the given <svg> element, inlining computed styles for portability. */
export function serializeSvg(svg: SVGSVGElement): string {
  const clone = svg.cloneNode(true) as SVGSVGElement;
  // Ensure it has proper XML namespace
  clone.setAttribute("xmlns", "http://www.w3.org/2000/svg");
  clone.setAttribute("xmlns:xlink", "http://www.w3.org/1999/xlink");
  return '<?xml version="1.0" encoding="UTF-8"?>\n' + clone.outerHTML;
}

/**
 * Render an SVG element to a PNG Blob at the given scale.
 * The SVG's viewBox defines the canvas dimensions.
 */
export async function svgToPngBlob(svg: SVGSVGElement, scale = 2): Promise<Blob> {
  const vb = svg.viewBox.baseVal;
  const width = vb.width || svg.clientWidth || 1200;
  const height = vb.height || svg.clientHeight || 800;

  const xml = serializeSvg(svg);
  const blob = new Blob([xml], { type: "image/svg+xml;charset=utf-8" });
  const url = URL.createObjectURL(blob);

  try {
    const img = new Image();
    img.crossOrigin = "anonymous";
    await new Promise<void>((resolve, reject) => {
      img.onload = () => resolve();
      img.onerror = () => reject(new Error("Failed to load SVG for rasterization"));
      img.src = url;
    });

    const canvas = document.createElement("canvas");
    canvas.width = Math.floor(width * scale);
    canvas.height = Math.floor(height * scale);
    const ctx = canvas.getContext("2d");
    if (!ctx) throw new Error("Canvas 2D context unavailable");
    ctx.fillStyle = "#ffffff";
    ctx.fillRect(0, 0, canvas.width, canvas.height);
    ctx.drawImage(img, 0, 0, canvas.width, canvas.height);

    return await new Promise<Blob>((resolve, reject) => {
      canvas.toBlob((b) => (b ? resolve(b) : reject(new Error("PNG encoding failed"))), "image/png");
    });
  } finally {
    URL.revokeObjectURL(url);
  }
}

/** Encode a PNG Blob into a minimal PDF document with the image centered on an A4 page. */
export async function pngBlobToPdf(png: Blob, pxWidth: number, pxHeight: number): Promise<Blob> {
  // PDFs with JPEG images are far simpler than with raw PNG/Flate. Convert first.
  const jpegBlob = await pngBlobToJpegBlob(png);
  const jpegBytes = new Uint8Array(await jpegBlob.arrayBuffer());

  // A4 landscape/portrait in PDF points (1 pt = 1/72 in). 842 × 595 pt.
  const pageW = pxWidth >= pxHeight ? 842 : 595;
  const pageH = pxWidth >= pxHeight ? 595 : 842;
  const margin = 24;
  const availW = pageW - margin * 2;
  const availH = pageH - margin * 2;
  const scale = Math.min(availW / pxWidth, availH / pxHeight);
  const drawW = pxWidth * scale;
  const drawH = pxHeight * scale;
  const tx = (pageW - drawW) / 2;
  const ty = (pageH - drawH) / 2;

  const enc = new TextEncoder();
  const chunks: Uint8Array[] = [];
  const offsets: number[] = [];
  let cursor = 0;

  const writeString = (s: string) => {
    const bytes = enc.encode(s);
    chunks.push(bytes);
    cursor += bytes.length;
  };
  const writeBytes = (b: Uint8Array) => {
    chunks.push(b);
    cursor += b.length;
  };

  // Header
  writeString("%PDF-1.4\n%\xff\xff\xff\xff\n");

  const beginObj = () => {
    offsets.push(cursor);
    const num = offsets.length;
    writeString(`${num} 0 obj\n`);
    return num;
  };
  const endObj = () => writeString("\nendobj\n");

  // Obj 1: Catalog
  beginObj();
  writeString("<< /Type /Catalog /Pages 2 0 R >>");
  endObj();

  // Obj 2: Pages
  beginObj();
  writeString("<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
  endObj();

  // Obj 3: Page
  beginObj();
  writeString(
    `<< /Type /Page /Parent 2 0 R /MediaBox [0 0 ${pageW} ${pageH}] ` +
      `/Resources << /XObject << /Im0 4 0 R >> /ProcSet [/PDF /ImageC] >> ` +
      `/Contents 5 0 R >>`,
  );
  endObj();

  // Obj 4: Image XObject (JPEG via DCTDecode)
  beginObj();
  writeString(
    `<< /Type /XObject /Subtype /Image /Width ${pxWidth} /Height ${pxHeight} ` +
      `/ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode /Length ${jpegBytes.length} >>\nstream\n`,
  );
  writeBytes(jpegBytes);
  writeString("\nendstream");
  endObj();

  // Obj 5: Page content stream
  const stream = `q\n${drawW.toFixed(3)} 0 0 ${drawH.toFixed(3)} ${tx.toFixed(3)} ${ty.toFixed(3)} cm\n/Im0 Do\nQ\n`;
  const streamBytes = enc.encode(stream);
  beginObj();
  writeString(`<< /Length ${streamBytes.length} >>\nstream\n`);
  writeBytes(streamBytes);
  writeString("endstream");
  endObj();

  // xref table
  const xrefOffset = cursor;
  let xref = `xref\n0 ${offsets.length + 1}\n0000000000 65535 f \n`;
  for (const off of offsets) {
    xref += off.toString().padStart(10, "0") + " 00000 n \n";
  }
  writeString(xref);
  writeString(
    `trailer\n<< /Size ${offsets.length + 1} /Root 1 0 R >>\nstartxref\n${xrefOffset}\n%%EOF\n`,
  );

  // Concatenate all chunks
  const total = chunks.reduce((n, c) => n + c.length, 0);
  const out = new Uint8Array(total);
  let o = 0;
  for (const c of chunks) {
    out.set(c, o);
    o += c.length;
  }
  return new Blob([out], { type: "application/pdf" });
}

async function pngBlobToJpegBlob(png: Blob): Promise<Blob> {
  const url = URL.createObjectURL(png);
  try {
    const img = new Image();
    await new Promise<void>((resolve, reject) => {
      img.onload = () => resolve();
      img.onerror = () => reject(new Error("Failed to load PNG"));
      img.src = url;
    });
    const canvas = document.createElement("canvas");
    canvas.width = img.naturalWidth;
    canvas.height = img.naturalHeight;
    const ctx = canvas.getContext("2d");
    if (!ctx) throw new Error("Canvas context unavailable");
    ctx.fillStyle = "#ffffff";
    ctx.fillRect(0, 0, canvas.width, canvas.height);
    ctx.drawImage(img, 0, 0);
    return await new Promise<Blob>((resolve, reject) => {
      canvas.toBlob(
        (b) => (b ? resolve(b) : reject(new Error("JPEG encoding failed"))),
        "image/jpeg",
        0.92,
      );
    });
  } finally {
    URL.revokeObjectURL(url);
  }
}

function escapeXml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

/** Build a drawio (.drawio) XML file representing the diagram. */
export function buildDrawioXml(
  diagram: DiagramState,
  tables: TableInfo[],
  columnsByKey: Record<string, ColumnInfo[]>,
  foreignKeys: ForeignKeyInfo[],
): string {
  const cells: string[] = [];
  const idForTable = new Map<string, string>();
  const idForColumn = new Map<string, string>();
  let nextId = 2;

  const genId = () => String(nextId++);

  // Root cells
  cells.push(`<mxCell id="0" />`);
  cells.push(`<mxCell id="1" parent="0" />`);

  for (const t of tables) {
    const key = `${t.schema}.${t.name}`;
    const cols = columnsByKey[key] ?? [];
    const pos = diagram.positions[key];
    if (!pos) continue;
    const w = TABLE_WIDTH;
    const h = tableHeight(cols.length);
    const tableId = genId();
    idForTable.set(key, tableId);
    const headerColor = pos.color ?? "#3b82f6";
    cells.push(
      `<mxCell id="${tableId}" value="${escapeXml(t.name)}" ` +
        `style="shape=table;startSize=${HEADER_HEIGHT};container=1;collapsible=0;childLayout=tableLayout;fontSize=12;strokeColor=${headerColor};fillColor=#ffffff;fontColor=#ffffff;align=left;spacingLeft=8;fillColor=${headerColor};verticalAlign=middle;" ` +
        `vertex="1" parent="1">` +
        `<mxGeometry x="${pos.x}" y="${pos.y}" width="${w}" height="${h}" as="geometry" />` +
        `</mxCell>`,
    );
    cols.forEach((col, i) => {
      const rowId = genId();
      const cellId = genId();
      idForColumn.set(`${key}.${col.name}`, cellId);
      cells.push(
        `<mxCell id="${rowId}" value="" ` +
          `style="shape=tableRow;horizontal=0;startSize=0;swimlaneHead=0;swimlaneBody=0;strokeColor=inherit;top=0;left=0;bottom=0;right=0;collapsible=0;dropTarget=0;fillColor=none;points=[[0,0.5],[1,0.5]];portConstraint=eastwest;fontSize=11;" ` +
          `vertex="1" parent="${tableId}">` +
          `<mxGeometry y="${HEADER_HEIGHT + i * COLUMN_HEIGHT}" width="${w}" height="${COLUMN_HEIGHT}" as="geometry" />` +
          `</mxCell>`,
      );
      const label = `${col.isPrimaryKey ? "\uD83D\uDD11 " : ""}${col.name}: ${col.dataType}`;
      cells.push(
        `<mxCell id="${cellId}" value="${escapeXml(label)}" ` +
          `style="shape=partialRectangle;html=1;whiteSpace=wrap;connectable=0;strokeColor=inherit;overflow=hidden;fillColor=none;top=0;left=0;bottom=0;right=0;pointerEvents=1;align=left;spacingLeft=6;fontSize=11;${col.isPrimaryKey ? "fontStyle=4;" : ""}" ` +
          `vertex="1" parent="${rowId}">` +
          `<mxGeometry width="${w}" height="${COLUMN_HEIGHT}" as="geometry" />` +
          `</mxCell>`,
      );
    });
  }

  // Relationships as edges between column cells
  for (const fk of foreignKeys) {
    const srcId = idForColumn.get(`${fk.sourceSchema}.${fk.sourceTable}.${fk.sourceColumn}`);
    const tgtId = idForColumn.get(`${fk.targetSchema}.${fk.targetTable}.${fk.targetColumn}`);
    if (!srcId || !tgtId) continue;
    cells.push(
      `<mxCell id="${genId()}" value="" ` +
        `style="edgeStyle=entityRelationEdgeStyle;fontSize=11;html=1;endArrow=ERmany;startArrow=ERone;rounded=0;exitX=1;exitY=0.5;entryX=0;entryY=0.5;" ` +
        `edge="1" parent="1" source="${srcId}" target="${tgtId}">` +
        `<mxGeometry relative="1" as="geometry" /></mxCell>`,
    );
  }

  // Annotations
  for (const ann of diagram.annotations) {
    const aid = genId();
    if (ann.shape === "rect") {
      const style = `rounded=0;whiteSpace=wrap;html=1;fillColor=${ann.fill ?? "#ffffff"};strokeColor=${ann.color ?? "#94a3b8"};`;
      cells.push(
        `<mxCell id="${aid}" value="${escapeXml(ann.text ?? "")}" style="${style}" vertex="1" parent="1">` +
          `<mxGeometry x="${ann.x}" y="${ann.y}" width="${ann.width ?? 120}" height="${ann.height ?? 80}" as="geometry" /></mxCell>`,
      );
    } else if (ann.shape === "text") {
      const style = `text;html=1;strokeColor=none;fillColor=none;align=left;verticalAlign=top;fontSize=${ann.fontSize ?? 12};fontColor=${ann.color ?? "#111827"};`;
      cells.push(
        `<mxCell id="${aid}" value="${escapeXml(ann.text ?? "")}" style="${style}" vertex="1" parent="1">` +
          `<mxGeometry x="${ann.x}" y="${ann.y}" width="${ann.width ?? 160}" height="${ann.height ?? 24}" as="geometry" /></mxCell>`,
      );
    } else if (ann.shape === "line") {
      const style = `endArrow=none;html=1;strokeColor=${ann.color ?? "#475569"};strokeWidth=${ann.strokeWidth ?? 1.5};`;
      cells.push(
        `<mxCell id="${aid}" style="${style}" edge="1" parent="1">` +
          `<mxGeometry relative="1" as="geometry">` +
          `<mxPoint x="${ann.x}" y="${ann.y}" as="sourcePoint"/>` +
          `<mxPoint x="${ann.x2 ?? ann.x + 100}" y="${ann.y2 ?? ann.y}" as="targetPoint"/>` +
          `</mxGeometry></mxCell>`,
      );
    }
  }

  return (
    `<mxfile host="sqail" modified="${new Date().toISOString()}" agent="sqail" version="24.0.0">\n` +
    `<diagram name="${escapeXml(diagram.schemaName)}">\n` +
    `<mxGraphModel dx="1000" dy="600" grid="1" gridSize="10" guides="1" tooltips="1" connect="1" arrows="1" fold="1" page="1" pageScale="1" pageWidth="1200" pageHeight="800" math="0" shadow="0">\n` +
    `<root>\n${cells.join("\n")}\n</root>\n` +
    `</mxGraphModel>\n</diagram>\n</mxfile>\n`
  );
}

/** Prompt the user for a save path and write the given bytes. */
export async function saveBytesAs(
  bytes: Uint8Array,
  defaultName: string,
  filters: { name: string; extensions: string[] }[],
): Promise<boolean> {
  const path = await save({ defaultPath: defaultName, filters });
  if (!path) return false;
  await writeFile(path, bytes);
  return true;
}

export async function saveTextAs(
  text: string,
  defaultName: string,
  filters: { name: string; extensions: string[] }[],
): Promise<boolean> {
  const bytes = new TextEncoder().encode(text);
  return saveBytesAs(bytes, defaultName, filters);
}
