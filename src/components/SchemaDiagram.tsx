import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Download,
  FileImage,
  FileText,
  FileType,
  Grid3x3,
  Key,
  Link2,
  Maximize,
  MousePointer,
  Palette,
  Plus,
  Minus,
  Square,
  Type as TypeIcon,
  Slash,
  RefreshCw,
  Trash2,
  X,
} from "lucide-react";
import { cn } from "../lib/utils";
import { useEditorStore } from "../stores/editorStore";
import { useConnectionStore } from "../stores/connectionStore";
import type {
  ColumnInfo,
  ForeignKeyInfo,
  TableInfo,
} from "../types/schema";
import {
  snapValue,
  type DiagramAnnotation,
  type DiagramColorSettings,
  type DiagramState,
} from "../types/diagram";
import {
  COLUMN_HEIGHT,
  HEADER_HEIGHT,
  TABLE_WIDTH,
  columnAnchor,
  columnY,
  tableHeight,
} from "../lib/diagramLayout";
import {
  buildDrawioXml,
  pngBlobToPdf,
  saveBytesAs,
  saveTextAs,
  serializeSvg,
  svgToPngBlob,
} from "../lib/diagramExport";

interface Props {
  tabId: string;
}

type Tool = "select" | "add-rect" | "add-text" | "add-line";

const DEFAULT_COLORS: DiagramColorSettings = {
  tableHeader: "#3b82f6",
  tableBorder: "#1e40af",
  tableBody: "#ffffff",
  pkColor: "#eab308",
  fkColor: "#8b5cf6",
  columnText: "#1f2937",
  relationship: "#64748b",
};

const SWATCH_COLORS = [
  "#3b82f6",
  "#10b981",
  "#8b5cf6",
  "#ef4444",
  "#f59e0b",
  "#ec4899",
  "#14b8a6",
  "#6366f1",
  "#64748b",
];

type LoadedSchema = {
  tables: TableInfo[];
  columns: Record<string, ColumnInfo[]>;
  foreignKeys: ForeignKeyInfo[];
};

export default function SchemaDiagram({ tabId }: Props) {
  const tab = useEditorStore((s) => s.tabs.find((t) => t.id === tabId));
  const updateDiagram = useEditorStore((s) => s.updateDiagram);
  const activeConnectionId = useConnectionStore((s) => s.activeConnectionId);
  const connections = useConnectionStore((s) => s.connections);

  const [loaded, setLoaded] = useState<LoadedSchema | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [tool, setTool] = useState<Tool>("select");
  const [selectedTable, setSelectedTable] = useState<string | null>(null);
  const [selectedAnnotation, setSelectedAnnotation] = useState<string | null>(null);
  const [colorPickerOpen, setColorPickerOpen] = useState(false);
  const [exportMenuOpen, setExportMenuOpen] = useState(false);
  const svgRef = useRef<SVGSVGElement>(null);
  // Schemas whose FK set has been fetched; prevents duplicate fetches.
  const fetchedFkSchemas = useRef<Set<string>>(new Set());

  const connectionId = tab?.connectionId ?? activeConnectionId ?? undefined;
  const connection = connections.find((c) => c.id === connectionId);

  // --- Adding a single table (drop target) -----------------------------------
  const addTable = useCallback(
    async (schemaName: string, tableName: string, worldX: number, worldY: number) => {
      if (!connectionId) {
        setError("No connection selected");
        return;
      }
      const key = `${schemaName}.${tableName}`;
      setError(null);
      setLoading(true);
      try {
        const cols = await invoke<ColumnInfo[]>("list_columns", {
          connectionId,
          schemaName,
          tableName,
        });

        let newFks: ForeignKeyInfo[] = [];
        if (!fetchedFkSchemas.current.has(schemaName)) {
          try {
            newFks = await invoke<ForeignKeyInfo[]>("list_foreign_keys", {
              connectionId,
              schemaName,
            });
          } catch {
            newFks = [];
          }
          fetchedFkSchemas.current.add(schemaName);
        }

        setLoaded((prev) => {
          const base = prev ?? { tables: [], columns: {}, foreignKeys: [] };
          const already = base.tables.some(
            (t) => t.schema === schemaName && t.name === tableName,
          );
          return {
            tables: already
              ? base.tables
              : [
                  ...base.tables,
                  { schema: schemaName, name: tableName, tableType: "table" },
                ],
            columns: { ...base.columns, [key]: cols },
            foreignKeys:
              newFks.length > 0 ? [...base.foreignKeys, ...newFks] : base.foreignKeys,
          };
        });

        updateDiagram(tabId, (d) => {
          const existing = d.positions[key];
          const snap = d.snapToGrid;
          const x = snapValue(worldX, snap);
          const y = snapValue(worldY, snap);
          if (existing) {
            // Already on the diagram — just move it to the drop point.
            return {
              ...d,
              positions: { ...d.positions, [key]: { ...existing, x, y } },
              loaded: true,
            };
          }
          return {
            ...d,
            positions: { ...d.positions, [key]: { key, x, y } },
            loaded: true,
          };
        });
      } catch (e) {
        setError(String(e));
      } finally {
        setLoading(false);
      }
    },
    [connectionId, tabId, updateDiagram],
  );

  // --- Refresh all currently-visible tables ----------------------------------
  const refresh = useCallback(async () => {
    if (!connectionId || !loaded || loaded.tables.length === 0) return;
    setLoading(true);
    setError(null);
    try {
      const newColumns: Record<string, ColumnInfo[]> = {};
      await Promise.all(
        loaded.tables.map(async (t) => {
          try {
            newColumns[`${t.schema}.${t.name}`] = await invoke<ColumnInfo[]>("list_columns", {
              connectionId,
              schemaName: t.schema,
              tableName: t.name,
            });
          } catch {
            newColumns[`${t.schema}.${t.name}`] = loaded.columns[`${t.schema}.${t.name}`] ?? [];
          }
        }),
      );
      const schemas = new Set(loaded.tables.map((t) => t.schema));
      const allFks: ForeignKeyInfo[] = [];
      for (const s of schemas) {
        try {
          const fks = await invoke<ForeignKeyInfo[]>("list_foreign_keys", {
            connectionId,
            schemaName: s,
          });
          allFks.push(...fks);
        } catch {
          // ignore
        }
      }
      fetchedFkSchemas.current = schemas;
      setLoaded({ tables: loaded.tables, columns: newColumns, foreignKeys: allFks });
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, [connectionId, loaded]);

  // Delete / Backspace removes the selected annotation, or hides the selected table.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Delete" && e.key !== "Backspace") return;
      const target = e.target as HTMLElement | null;
      if (target && /^(INPUT|TEXTAREA|SELECT)$/i.test(target.tagName)) return;
      if (target && target.isContentEditable) return;
      if (selectedAnnotation) {
        e.preventDefault();
        updateDiagram(tabId, (d) => ({
          ...d,
          annotations: d.annotations.filter((a) => a.id !== selectedAnnotation),
        }));
        setSelectedAnnotation(null);
      } else if (selectedTable) {
        e.preventDefault();
        updateDiagram(tabId, (d) => {
          const { [selectedTable]: _removed, ...rest } = d.positions;
          void _removed;
          return { ...d, positions: rest };
        });
        setSelectedTable(null);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [selectedAnnotation, selectedTable, tabId, updateDiagram]);

  if (!tab || !tab.diagram) {
    return <div className="p-4 text-sm text-muted-foreground">Tab not found</div>;
  }

  const diagram = tab.diagram;
  const colors = { ...DEFAULT_COLORS, ...(diagram.colors ?? {}) };

  return (
    <div className="flex flex-1 flex-col overflow-hidden bg-background">
      <Toolbar
        diagram={diagram}
        loading={loading}
        tool={tool}
        onTool={setTool}
        connection={connection?.name}
        tableCount={loaded?.tables.length ?? 0}
        onRefresh={refresh}
        onZoom={(delta) =>
          updateDiagram(tabId, (d) => ({
            ...d,
            zoom: Math.max(0.2, Math.min(3, d.zoom + delta)),
          }))
        }
        onResetView={() =>
          updateDiagram(tabId, (d) => ({ ...d, zoom: 1, panX: 0, panY: 0 }))
        }
        onFit={() =>
          fitToContent(svgRef.current, diagram, loaded, (updater) =>
            updateDiagram(tabId, updater),
          )
        }
        onToggleSnap={() =>
          updateDiagram(tabId, (d) => ({ ...d, snapToGrid: !d.snapToGrid }))
        }
        colorPickerOpen={colorPickerOpen}
        onToggleColors={() => setColorPickerOpen((v) => !v)}
        exportMenuOpen={exportMenuOpen}
        onToggleExport={() => setExportMenuOpen((v) => !v)}
        onExport={async (kind) => {
          setExportMenuOpen(false);
          await handleExport(kind, svgRef.current, diagram, loaded);
        }}
      />
      {colorPickerOpen && (
        <ColorPicker
          colors={colors}
          onChange={(patch) =>
            updateDiagram(tabId, (d) => ({
              ...d,
              colors: { ...(d.colors ?? {}), ...patch },
            }))
          }
          onClose={() => setColorPickerOpen(false)}
        />
      )}
      {error && (
        <div className="border-b border-destructive/40 bg-destructive/10 px-3 py-1.5 text-xs text-destructive">
          {error}
        </div>
      )}

      <Canvas
        svgRef={svgRef}
        diagram={diagram}
        loaded={loaded}
        colors={colors}
        tool={tool}
        onResetTool={() => setTool("select")}
        onDropTable={(schemaName, tableName, worldX, worldY) =>
          addTable(schemaName, tableName, worldX, worldY)
        }
        selectedTable={selectedTable}
        selectedAnnotation={selectedAnnotation}
        onSelectTable={(k) => {
          setSelectedTable(k);
          setSelectedAnnotation(null);
        }}
        onSelectAnnotation={(id) => {
          setSelectedAnnotation(id);
          setSelectedTable(null);
        }}
        onPan={(dx, dy) =>
          updateDiagram(tabId, (d) => ({ ...d, panX: d.panX + dx, panY: d.panY + dy }))
        }
        onZoom={(delta, cx, cy) =>
          updateDiagram(tabId, (d) => {
            const next = Math.max(0.2, Math.min(3, d.zoom + delta));
            const ratio = next / d.zoom;
            // Zoom towards cursor
            const panX = cx - (cx - d.panX) * ratio;
            const panY = cy - (cy - d.panY) * ratio;
            return { ...d, zoom: next, panX, panY };
          })
        }
        onSetTablePos={(key, x, y) =>
          updateDiagram(tabId, (d) => {
            const pos = d.positions[key];
            if (!pos) return d;
            return {
              ...d,
              positions: { ...d.positions, [key]: { ...pos, x, y } },
            };
          })
        }
        onAddAnnotation={(ann) =>
          updateDiagram(tabId, (d) => ({
            ...d,
            annotations: [...d.annotations, ann],
          }))
        }
        onUpdateAnnotation={(id, patch) =>
          updateDiagram(tabId, (d) => ({
            ...d,
            annotations: d.annotations.map((a) => (a.id === id ? { ...a, ...patch } : a)),
          }))
        }
        onSetTableColor={(key, color) =>
          updateDiagram(tabId, (d) => {
            const pos = d.positions[key];
            if (!pos) return d;
            return {
              ...d,
              positions: { ...d.positions, [key]: { ...pos, color } },
            };
          })
        }
      />

      {selectedAnnotation && (
        <AnnotationInspector
          annotation={diagram.annotations.find((a) => a.id === selectedAnnotation)!}
          onChange={(patch) =>
            updateDiagram(tabId, (d) => ({
              ...d,
              annotations: d.annotations.map((a) =>
                a.id === selectedAnnotation ? { ...a, ...patch } : a,
              ),
            }))
          }
          onDelete={() => {
            updateDiagram(tabId, (d) => ({
              ...d,
              annotations: d.annotations.filter((a) => a.id !== selectedAnnotation),
            }));
            setSelectedAnnotation(null);
          }}
          onClose={() => setSelectedAnnotation(null)}
        />
      )}

      {selectedTable && (
        <TableInspector
          tableKey={selectedTable}
          color={diagram.positions[selectedTable]?.color}
          onSetColor={(color) => {
            updateDiagram(tabId, (d) => {
              const pos = d.positions[selectedTable];
              if (!pos) return d;
              return {
                ...d,
                positions: { ...d.positions, [selectedTable]: { ...pos, color } },
              };
            });
          }}
          onClose={() => setSelectedTable(null)}
        />
      )}
    </div>
  );
}

// ──────────────────────────────────────────────────────────────────────────────
// Toolbar
// ──────────────────────────────────────────────────────────────────────────────

interface ToolbarProps {
  diagram: DiagramState;
  loading: boolean;
  tool: Tool;
  onTool: (t: Tool) => void;
  connection?: string;
  tableCount: number;
  onRefresh: () => void;
  onZoom: (delta: number) => void;
  onResetView: () => void;
  onFit: () => void;
  onToggleSnap: () => void;
  colorPickerOpen: boolean;
  onToggleColors: () => void;
  exportMenuOpen: boolean;
  onToggleExport: () => void;
  onExport: (kind: "png" | "svg" | "pdf" | "drawio") => void;
}

function Toolbar({
  diagram,
  loading,
  tool,
  onTool,
  connection,
  tableCount,
  onRefresh,
  onZoom,
  onResetView,
  onFit,
  onToggleSnap,
  colorPickerOpen,
  onToggleColors,
  exportMenuOpen,
  onToggleExport,
  onExport,
}: ToolbarProps) {
  return (
    <div className="flex shrink-0 items-center gap-2 border-b border-border bg-muted/20 px-2 py-1.5 text-xs">
      <span className="text-muted-foreground">
        {connection ? `${connection} · ` : ""}Diagram
      </span>
      <span className="text-[10px] text-muted-foreground/70">
        {tableCount} {tableCount === 1 ? "table" : "tables"}
      </span>
      <button
        onClick={onRefresh}
        disabled={loading || tableCount === 0}
        className="rounded p-1 text-muted-foreground hover:bg-accent hover:text-foreground disabled:opacity-40"
        title="Reload columns & foreign keys"
      >
        <RefreshCw size={12} className={loading ? "animate-spin" : ""} />
      </button>

      <div className="mx-1 h-4 w-px bg-border" />

      <ToolBtn active={tool === "select"} onClick={() => onTool("select")} title="Select / drag tables (V)">
        <MousePointer size={12} />
      </ToolBtn>
      <ToolBtn active={tool === "add-rect"} onClick={() => onTool("add-rect")} title="Draw rectangle (R)">
        <Square size={12} />
      </ToolBtn>
      <ToolBtn active={tool === "add-text"} onClick={() => onTool("add-text")} title="Add text (T)">
        <TypeIcon size={12} />
      </ToolBtn>
      <ToolBtn active={tool === "add-line"} onClick={() => onTool("add-line")} title="Draw line (L)">
        <Slash size={12} />
      </ToolBtn>

      <div className="mx-1 h-4 w-px bg-border" />

      <button
        onClick={() => onZoom(-0.1)}
        className="rounded p-1 text-muted-foreground hover:bg-accent hover:text-foreground"
        title="Zoom out"
      >
        <Minus size={12} />
      </button>
      <span className="min-w-10 text-center text-[10px] text-muted-foreground">
        {Math.round(diagram.zoom * 100)}%
      </span>
      <button
        onClick={() => onZoom(0.1)}
        className="rounded p-1 text-muted-foreground hover:bg-accent hover:text-foreground"
        title="Zoom in"
      >
        <Plus size={12} />
      </button>
      <button
        onClick={onFit}
        className="rounded p-1 text-muted-foreground hover:bg-accent hover:text-foreground"
        title="Fit to content"
      >
        <Maximize size={12} />
      </button>
      <button
        onClick={onResetView}
        className="rounded px-2 py-0.5 text-[10px] text-muted-foreground hover:bg-accent hover:text-foreground"
        title="Reset view"
      >
        Reset
      </button>

      <div className="mx-1 h-4 w-px bg-border" />

      <button
        onClick={onToggleSnap}
        className={cn(
          "flex items-center gap-1 rounded px-2 py-1 text-[10px]",
          diagram.snapToGrid
            ? "bg-primary/15 text-primary"
            : "text-muted-foreground hover:bg-accent hover:text-foreground",
        )}
        title={diagram.snapToGrid ? "Snap to grid: on" : "Snap to grid: off"}
      >
        <Grid3x3 size={12} /> Snap
      </button>

      <div className="ml-auto flex items-center gap-1">
        <button
          onClick={onToggleColors}
          className={cn(
            "flex items-center gap-1 rounded px-2 py-1 text-[10px]",
            colorPickerOpen
              ? "bg-primary/10 text-primary"
              : "text-muted-foreground hover:bg-accent hover:text-foreground",
          )}
        >
          <Palette size={12} /> Colors
        </button>
        <div className="relative">
          <button
            onClick={onToggleExport}
            className={cn(
              "flex items-center gap-1 rounded px-2 py-1 text-[10px]",
              exportMenuOpen
                ? "bg-primary/10 text-primary"
                : "text-muted-foreground hover:bg-accent hover:text-foreground",
            )}
          >
            <Download size={12} /> Export
          </button>
          {exportMenuOpen && (
            <div className="absolute right-0 top-full z-30 mt-1 min-w-40 rounded-md border border-border bg-background py-1 shadow-lg">
              <ExportItem icon={<FileImage size={12} />} label="PNG image" onClick={() => onExport("png")} />
              <ExportItem icon={<FileImage size={12} />} label="SVG image" onClick={() => onExport("svg")} />
              <ExportItem icon={<FileText size={12} />} label="PDF document" onClick={() => onExport("pdf")} />
              <ExportItem icon={<FileType size={12} />} label="drawio file" onClick={() => onExport("drawio")} />
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

function ToolBtn({
  active,
  onClick,
  title,
  children,
}: {
  active: boolean;
  onClick: () => void;
  title: string;
  children: React.ReactNode;
}) {
  return (
    <button
      onClick={onClick}
      title={title}
      className={cn(
        "rounded p-1",
        active
          ? "bg-primary/15 text-primary"
          : "text-muted-foreground hover:bg-accent hover:text-foreground",
      )}
    >
      {children}
    </button>
  );
}

function ExportItem({
  icon,
  label,
  onClick,
}: {
  icon: React.ReactNode;
  label: string;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-xs hover:bg-accent hover:text-accent-foreground"
    >
      {icon}
      {label}
    </button>
  );
}

// ──────────────────────────────────────────────────────────────────────────────
// Canvas
// ──────────────────────────────────────────────────────────────────────────────

interface CanvasProps {
  svgRef: React.RefObject<SVGSVGElement | null>;
  diagram: DiagramState;
  loaded: LoadedSchema | null;
  colors: DiagramColorSettings;
  tool: Tool;
  onResetTool: () => void;
  onDropTable: (schemaName: string, tableName: string, worldX: number, worldY: number) => void;
  selectedTable: string | null;
  selectedAnnotation: string | null;
  onSelectTable: (k: string | null) => void;
  onSelectAnnotation: (id: string | null) => void;
  onPan: (dx: number, dy: number) => void;
  onZoom: (delta: number, cx: number, cy: number) => void;
  onSetTablePos: (key: string, x: number, y: number) => void;
  onAddAnnotation: (ann: DiagramAnnotation) => void;
  onUpdateAnnotation: (id: string, patch: Partial<DiagramAnnotation>) => void;
  onSetTableColor: (key: string, color: string) => void;
}

function Canvas({
  svgRef,
  diagram,
  loaded,
  colors,
  tool,
  onResetTool,
  onDropTable,
  selectedTable,
  selectedAnnotation,
  onSelectTable,
  onSelectAnnotation,
  onPan,
  onZoom,
  onSetTablePos,
  onAddAnnotation,
  onUpdateAnnotation,
}: CanvasProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [draftAnn, setDraftAnn] = useState<DiagramAnnotation | null>(null);
  const [dropOver, setDropOver] = useState(false);
  const snap = diagram.snapToGrid;
  const dragRef = useRef<
    | null
    | { kind: "pan"; startX: number; startY: number }
    | {
        kind: "table";
        key: string;
        startClientX: number;
        startClientY: number;
        startPosX: number;
        startPosY: number;
      }
    | { kind: "draw"; startX: number; startY: number }
  >(null);

  /** Convert client (mouse) coords to canvas world coords. */
  const toWorld = useCallback(
    (clientX: number, clientY: number) => {
      const el = containerRef.current;
      if (!el) return { x: 0, y: 0 };
      const rect = el.getBoundingClientRect();
      const localX = clientX - rect.left;
      const localY = clientY - rect.top;
      return {
        x: (localX - diagram.panX) / diagram.zoom,
        y: (localY - diagram.panY) / diagram.zoom,
      };
    },
    [diagram.panX, diagram.panY, diagram.zoom],
  );

  // Non-passive wheel listener so we can preventDefault and zoom to cursor.
  // Wheel-to-zoom is always active; Shift+wheel pans horizontally, plain Shift-less
  // scrolling with the Ctrl modifier still works because we preventDefault in all cases.
  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const handler = (e: WheelEvent) => {
      e.preventDefault();
      const rect = el.getBoundingClientRect();
      const cx = e.clientX - rect.left;
      const cy = e.clientY - rect.top;
      // Normalize deltaY across DOM_DELTA_PIXEL (0), DOM_DELTA_LINE (1), DOM_DELTA_PAGE (2).
      const unit = e.deltaMode === 1 ? 16 : e.deltaMode === 2 ? rect.height : 1;
      const scaledDelta = e.deltaY * unit;
      // Clamp per-tick to avoid huge jumps from high-resolution touchpad wheels.
      const clamped = Math.max(-120, Math.min(120, scaledDelta));
      const delta = -clamped * 0.0035;
      onZoom(delta, cx, cy);
    };
    el.addEventListener("wheel", handler, { passive: false });
    return () => el.removeEventListener("wheel", handler);
  }, [onZoom]);

  const onMouseDown = useCallback(
    (e: React.MouseEvent) => {
      if (e.button !== 0) return;
      if (tool === "select") {
        dragRef.current = { kind: "pan", startX: e.clientX, startY: e.clientY };
        onSelectTable(null);
        onSelectAnnotation(null);
      } else {
        const w = toWorld(e.clientX, e.clientY);
        const id = crypto.randomUUID();
        let ann: DiagramAnnotation;
        if (tool === "add-rect") {
          ann = {
            id,
            shape: "rect",
            x: w.x,
            y: w.y,
            width: 1,
            height: 1,
            color: colors.relationship,
            fill: "rgba(59,130,246,0.1)",
          };
        } else if (tool === "add-line") {
          ann = {
            id,
            shape: "line",
            x: w.x,
            y: w.y,
            x2: w.x,
            y2: w.y,
            color: colors.relationship,
            strokeWidth: 2,
          };
        } else {
          ann = {
            id,
            shape: "text",
            x: w.x,
            y: w.y,
            width: 160,
            height: 24,
            text: "Text",
            color: colors.columnText,
            fontSize: 14,
          };
          onAddAnnotation(ann);
          onSelectAnnotation(id);
          onResetTool();
          return;
        }
        setDraftAnn(ann);
        dragRef.current = { kind: "draw", startX: e.clientX, startY: e.clientY };
      }
    },
    [tool, toWorld, onSelectTable, onSelectAnnotation, onAddAnnotation, onResetTool, colors],
  );

  const onMouseMove = useCallback(
    (e: MouseEvent) => {
      const drag = dragRef.current;
      if (!drag) return;
      if (drag.kind === "pan") {
        const dx = e.clientX - drag.startX;
        const dy = e.clientY - drag.startY;
        if (dx !== 0 || dy !== 0) {
          onPan(dx, dy);
          dragRef.current = { ...drag, startX: e.clientX, startY: e.clientY };
        }
      } else if (drag.kind === "table") {
        const dx = (e.clientX - drag.startClientX) / diagram.zoom;
        const dy = (e.clientY - drag.startClientY) / diagram.zoom;
        const nx = snapValue(drag.startPosX + dx, snap);
        const ny = snapValue(drag.startPosY + dy, snap);
        onSetTablePos(drag.key, nx, ny);
      } else if (drag.kind === "draw" && draftAnn) {
        const w = toWorld(e.clientX, e.clientY);
        const sx = snapValue(w.x, snap);
        const sy = snapValue(w.y, snap);
        if (draftAnn.shape === "rect") {
          setDraftAnn({
            ...draftAnn,
            x: Math.min(draftAnn.x, sx),
            y: Math.min(draftAnn.y, sy),
            width: Math.abs(sx - draftAnn.x) || 1,
            height: Math.abs(sy - draftAnn.y) || 1,
          });
        } else if (draftAnn.shape === "line") {
          setDraftAnn({ ...draftAnn, x2: sx, y2: sy });
        }
      }
    },
    [diagram.zoom, draftAnn, onPan, onSetTablePos, toWorld, snap],
  );

  const onMouseUp = useCallback(() => {
    const drag = dragRef.current;
    dragRef.current = null;
    if (drag?.kind === "draw" && draftAnn) {
      // Commit the draft if it has size
      const ok =
        (draftAnn.shape === "rect" && (draftAnn.width ?? 0) > 4 && (draftAnn.height ?? 0) > 4) ||
        (draftAnn.shape === "line" &&
          (Math.abs((draftAnn.x2 ?? 0) - draftAnn.x) > 4 ||
            Math.abs((draftAnn.y2 ?? 0) - draftAnn.y) > 4));
      if (ok) {
        onAddAnnotation(draftAnn);
        onSelectAnnotation(draftAnn.id);
      }
      setDraftAnn(null);
      onResetTool();
    }
  }, [draftAnn, onAddAnnotation, onSelectAnnotation, onResetTool]);

  useEffect(() => {
    const move = (e: MouseEvent) => onMouseMove(e);
    const up = () => onMouseUp();
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
    return () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
    };
  }, [onMouseMove, onMouseUp]);

  return (
    <div
      ref={containerRef}
      className={cn(
        "relative flex-1 overflow-hidden bg-muted/10",
        dropOver && "ring-2 ring-inset ring-primary/40",
      )}
      style={{ cursor: tool === "select" ? "grab" : "crosshair" }}
      onMouseDown={onMouseDown}
      onDragOver={(e) => {
        if (!e.dataTransfer.types.includes("application/sqlai-table")) return;
        e.preventDefault();
        e.dataTransfer.dropEffect = "copy";
        setDropOver(true);
      }}
      onDragLeave={(e) => {
        // Only clear when leaving the container itself, not inner children.
        if (e.currentTarget === e.target) setDropOver(false);
      }}
      onDrop={(e) => {
        setDropOver(false);
        const raw = e.dataTransfer.getData("application/sqlai-table");
        if (!raw) return;
        e.preventDefault();
        try {
          const { schemaName, tableName } = JSON.parse(raw) as {
            schemaName: string;
            tableName: string;
          };
          const w = toWorld(e.clientX, e.clientY);
          onDropTable(schemaName, tableName, w.x, w.y);
        } catch {
          // ignore malformed payload
        }
      }}
    >
      {/* Background dot grid */}
      <div
        className="pointer-events-none absolute inset-0 opacity-60"
        style={{
          backgroundImage:
            "radial-gradient(circle, var(--color-muted-foreground) 1px, transparent 1px)",
          backgroundSize: `${20 * diagram.zoom}px ${20 * diagram.zoom}px`,
          backgroundPosition: `${diagram.panX}px ${diagram.panY}px`,
          opacity: 0.15,
        }}
      />

      <svg
        ref={svgRef}
        className="absolute inset-0 h-full w-full"
      >
        <defs>
          <marker
            id="arrowhead-fk"
            viewBox="0 0 12 12"
            refX="10"
            refY="6"
            markerWidth="10"
            markerHeight="10"
            orient="auto-start-reverse"
          >
            <path d="M 0 0 L 12 6 L 0 12 z" fill={colors.relationship} />
          </marker>
        </defs>
        <g
          transform={`translate(${diagram.panX} ${diagram.panY}) scale(${diagram.zoom})`}
        >
          {/* Annotations behind tables */}
          {diagram.annotations
            .filter((a) => a.shape !== "line")
            .map((a) => (
              <AnnotationNode
                key={a.id}
                annotation={a}
                selected={selectedAnnotation === a.id}
                onSelect={() => onSelectAnnotation(a.id)}
                onStartDrag={(e) => {
                  if (tool !== "select") return;
                  e.stopPropagation();
                  dragRef.current = {
                    kind: "draw",
                    startX: e.clientX,
                    startY: e.clientY,
                  };
                  // Use a custom ann-move drag by reusing draftAnn logic:
                  // Simpler: move via direct state update per mousemove.
                  const startWorld = toWorld(e.clientX, e.clientY);
                  const origX = a.x;
                  const origY = a.y;
                  const move = (ev: MouseEvent) => {
                    const w = toWorld(ev.clientX, ev.clientY);
                    onUpdateAnnotation(a.id, {
                      x: snapValue(origX + (w.x - startWorld.x), snap),
                      y: snapValue(origY + (w.y - startWorld.y), snap),
                    });
                  };
                  const up = () => {
                    window.removeEventListener("mousemove", move);
                    window.removeEventListener("mouseup", up);
                  };
                  window.addEventListener("mousemove", move);
                  window.addEventListener("mouseup", up);
                }}
              />
            ))}

          {/* FK relationships */}
          {loaded && <Relationships loaded={loaded} diagram={diagram} color={colors.relationship} />}

          {/* Tables */}
          {loaded?.tables.map((t) => {
            const key = `${t.schema}.${t.name}`;
            const pos = diagram.positions[key];
            if (!pos) return null;
            const cols = loaded.columns[key] ?? [];
            return (
              <TableNode
                key={key}
                tableKey={key}
                name={t.name}
                columns={cols}
                x={pos.x}
                y={pos.y}
                color={pos.color}
                colors={colors}
                foreignKeys={loaded.foreignKeys}
                selected={selectedTable === key}
                onSelect={() => onSelectTable(key)}
                onStartDrag={(e) => {
                  e.stopPropagation();
                  dragRef.current = {
                    kind: "table",
                    key,
                    startClientX: e.clientX,
                    startClientY: e.clientY,
                    startPosX: pos.x,
                    startPosY: pos.y,
                  };
                }}
              />
            );
          })}

          {/* Line annotations in front */}
          {diagram.annotations
            .filter((a) => a.shape === "line")
            .map((a) => (
              <AnnotationNode
                key={a.id}
                annotation={a}
                selected={selectedAnnotation === a.id}
                onSelect={() => onSelectAnnotation(a.id)}
                onStartDrag={(e) => {
                  if (tool !== "select") return;
                  e.stopPropagation();
                  const startWorld = toWorld(e.clientX, e.clientY);
                  const origX = a.x;
                  const origY = a.y;
                  const origX2 = a.x2 ?? a.x;
                  const origY2 = a.y2 ?? a.y;
                  const move = (ev: MouseEvent) => {
                    const w = toWorld(ev.clientX, ev.clientY);
                    const dx = w.x - startWorld.x;
                    const dy = w.y - startWorld.y;
                    onUpdateAnnotation(a.id, {
                      x: snapValue(origX + dx, snap),
                      y: snapValue(origY + dy, snap),
                      x2: snapValue(origX2 + dx, snap),
                      y2: snapValue(origY2 + dy, snap),
                    });
                  };
                  const up = () => {
                    window.removeEventListener("mousemove", move);
                    window.removeEventListener("mouseup", up);
                  };
                  window.addEventListener("mousemove", move);
                  window.addEventListener("mouseup", up);
                }}
              />
            ))}

          {/* Draft annotation */}
          {draftAnn && (
            <AnnotationNode
              annotation={draftAnn}
              selected={false}
              onSelect={() => {}}
              onStartDrag={() => {}}
            />
          )}

          {/* Resize handles for selected annotation */}
          {(() => {
            if (!selectedAnnotation) return null;
            const ann = diagram.annotations.find((a) => a.id === selectedAnnotation);
            if (!ann) return null;
            return (
              <SelectionHandles
                annotation={ann}
                zoom={diagram.zoom}
                snap={snap}
                toWorld={toWorld}
                onUpdate={onUpdateAnnotation}
              />
            );
          })()}
        </g>
      </svg>

      {(!loaded || loaded.tables.length === 0) && (
        <div className="pointer-events-none absolute inset-0 flex items-center justify-center">
          <div className="pointer-events-none rounded-md border border-dashed border-border bg-background/70 px-4 py-3 text-center text-xs text-muted-foreground">
            Drag tables from the schema tree to add them to the diagram.
          </div>
        </div>
      )}
    </div>
  );
}

// ──────────────────────────────────────────────────────────────────────────────
// Resize handles
// ──────────────────────────────────────────────────────────────────────────────

const MIN_RECT_SIZE = 8;
const MIN_TEXT_WIDTH = 20;

function Handle({
  cx,
  cy,
  size,
  strokeW,
  cursor,
  onDown,
  round,
}: {
  cx: number;
  cy: number;
  size: number;
  strokeW: number;
  cursor: string;
  round?: boolean;
  onDown: (e: React.MouseEvent) => void;
}) {
  const half = size / 2;
  return round ? (
    <circle
      cx={cx}
      cy={cy}
      r={half}
      fill="#ffffff"
      stroke="#2563eb"
      strokeWidth={strokeW}
      style={{ cursor }}
      onMouseDown={onDown}
    />
  ) : (
    <rect
      x={cx - half}
      y={cy - half}
      width={size}
      height={size}
      fill="#ffffff"
      stroke="#2563eb"
      strokeWidth={strokeW}
      style={{ cursor }}
      onMouseDown={onDown}
    />
  );
}

function SelectionHandles({
  annotation,
  zoom,
  snap,
  toWorld,
  onUpdate,
}: {
  annotation: DiagramAnnotation;
  zoom: number;
  snap: boolean | undefined;
  toWorld: (clientX: number, clientY: number) => { x: number; y: number };
  onUpdate: (id: string, patch: Partial<DiagramAnnotation>) => void;
}) {
  const size = 8 / zoom;
  const strokeW = 1 / zoom;

  const startResize = (
    e: React.MouseEvent,
    compute: (wx: number, wy: number) => Partial<DiagramAnnotation>,
  ) => {
    e.stopPropagation();
    e.preventDefault();
    const move = (ev: MouseEvent) => {
      const w = toWorld(ev.clientX, ev.clientY);
      onUpdate(annotation.id, compute(snapValue(w.x, snap), snapValue(w.y, snap)));
    };
    const up = () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  };

  if (annotation.shape === "rect") {
    const x = annotation.x;
    const y = annotation.y;
    const w = annotation.width ?? 100;
    const h = annotation.height ?? 60;
    const right = x + w;
    const bottom = y + h;
    return (
      <g>
        {/* Corners */}
        <Handle size={size} strokeW={strokeW}
          cx={x}
          cy={y}
          cursor="nwse-resize"
          onDown={(e) =>
            startResize(e, (wx, wy) => {
              const nx = Math.min(wx, right - MIN_RECT_SIZE);
              const ny = Math.min(wy, bottom - MIN_RECT_SIZE);
              return { x: nx, y: ny, width: right - nx, height: bottom - ny };
            })
          }
        />
        <Handle size={size} strokeW={strokeW}
          cx={right}
          cy={y}
          cursor="nesw-resize"
          onDown={(e) =>
            startResize(e, (wx, wy) => {
              const ny = Math.min(wy, bottom - MIN_RECT_SIZE);
              return {
                y: ny,
                width: Math.max(MIN_RECT_SIZE, wx - x),
                height: bottom - ny,
              };
            })
          }
        />
        <Handle size={size} strokeW={strokeW}
          cx={x}
          cy={bottom}
          cursor="nesw-resize"
          onDown={(e) =>
            startResize(e, (wx, wy) => {
              const nx = Math.min(wx, right - MIN_RECT_SIZE);
              return {
                x: nx,
                width: right - nx,
                height: Math.max(MIN_RECT_SIZE, wy - y),
              };
            })
          }
        />
        <Handle size={size} strokeW={strokeW}
          cx={right}
          cy={bottom}
          cursor="nwse-resize"
          onDown={(e) =>
            startResize(e, (wx, wy) => ({
              width: Math.max(MIN_RECT_SIZE, wx - x),
              height: Math.max(MIN_RECT_SIZE, wy - y),
            }))
          }
        />
        {/* Edges */}
        <Handle size={size} strokeW={strokeW}
          cx={x + w / 2}
          cy={y}
          cursor="ns-resize"
          onDown={(e) =>
            startResize(e, (_wx, wy) => {
              const ny = Math.min(wy, bottom - MIN_RECT_SIZE);
              return { y: ny, height: bottom - ny };
            })
          }
        />
        <Handle size={size} strokeW={strokeW}
          cx={x + w / 2}
          cy={bottom}
          cursor="ns-resize"
          onDown={(e) =>
            startResize(e, (_wx, wy) => ({
              height: Math.max(MIN_RECT_SIZE, wy - y),
            }))
          }
        />
        <Handle size={size} strokeW={strokeW}
          cx={x}
          cy={y + h / 2}
          cursor="ew-resize"
          onDown={(e) =>
            startResize(e, (wx) => {
              const nx = Math.min(wx, right - MIN_RECT_SIZE);
              return { x: nx, width: right - nx };
            })
          }
        />
        <Handle size={size} strokeW={strokeW}
          cx={right}
          cy={y + h / 2}
          cursor="ew-resize"
          onDown={(e) =>
            startResize(e, (wx) => ({
              width: Math.max(MIN_RECT_SIZE, wx - x),
            }))
          }
        />
      </g>
    );
  }

  if (annotation.shape === "line") {
    const x1 = annotation.x;
    const y1 = annotation.y;
    const x2 = annotation.x2 ?? x1;
    const y2 = annotation.y2 ?? y1;
    return (
      <g>
        <Handle size={size} strokeW={strokeW}
          cx={x1}
          cy={y1}
          cursor="move"
          round
          onDown={(e) => startResize(e, (wx, wy) => ({ x: wx, y: wy }))}
        />
        <Handle size={size} strokeW={strokeW}
          cx={x2}
          cy={y2}
          cursor="move"
          round
          onDown={(e) => startResize(e, (wx, wy) => ({ x2: wx, y2: wy }))}
        />
      </g>
    );
  }

  if (annotation.shape === "text") {
    const x = annotation.x;
    const y = annotation.y;
    const w = annotation.width ?? 160;
    const h = annotation.height ?? 24;
    const fontSize = annotation.fontSize ?? 14;
    return (
      <g>
        {/* Right edge: resize width only */}
        <Handle size={size} strokeW={strokeW}
          cx={x + w}
          cy={y + h / 2}
          cursor="ew-resize"
          onDown={(e) =>
            startResize(e, (wx) => ({
              width: Math.max(MIN_TEXT_WIDTH, wx - x),
            }))
          }
        />
        {/* Bottom-right corner: scale font size proportional to height */}
        <Handle size={size} strokeW={strokeW}
          cx={x + w}
          cy={y + h}
          cursor="nwse-resize"
          onDown={(e) =>
            startResize(e, (wx, wy) => {
              const newW = Math.max(MIN_TEXT_WIDTH, wx - x);
              const newH = Math.max(10, wy - y);
              const scale = newH / h;
              return {
                width: newW,
                height: newH,
                fontSize: Math.max(8, Math.round(fontSize * scale)),
              };
            })
          }
        />
      </g>
    );
  }

  return null;
}

// ──────────────────────────────────────────────────────────────────────────────
// Sub-components
// ──────────────────────────────────────────────────────────────────────────────

function TableNode({
  tableKey,
  name,
  columns,
  x,
  y,
  color,
  colors,
  foreignKeys,
  selected,
  onSelect,
  onStartDrag,
}: {
  tableKey: string;
  name: string;
  columns: ColumnInfo[];
  x: number;
  y: number;
  color?: string;
  colors: DiagramColorSettings;
  foreignKeys: ForeignKeyInfo[];
  selected: boolean;
  onSelect: () => void;
  onStartDrag: (e: React.MouseEvent) => void;
}) {
  const h = tableHeight(columns.length);
  const headerColor = color ?? colors.tableHeader;
  const dotIdx = tableKey.indexOf(".");
  const schema = dotIdx >= 0 ? tableKey.slice(0, dotIdx) : "";
  const fkCols = new Set<string>();
  for (const fk of foreignKeys) {
    if (fk.sourceSchema === schema && fk.sourceTable === name) {
      fkCols.add(fk.sourceColumn);
    }
  }

  return (
    <g
      transform={`translate(${x} ${y})`}
      onMouseDown={(e) => {
        onSelect();
        onStartDrag(e);
      }}
      style={{ cursor: "move" }}
    >
      <rect
        width={TABLE_WIDTH}
        height={h}
        rx={6}
        ry={6}
        fill={colors.tableBody}
        stroke={selected ? "#2563eb" : colors.tableBorder}
        strokeWidth={selected ? 2 : 1}
      />
      <rect
        width={TABLE_WIDTH}
        height={HEADER_HEIGHT}
        rx={6}
        ry={6}
        fill={headerColor}
      />
      {/* Clip bottom corners of header */}
      <rect y={HEADER_HEIGHT - 6} width={TABLE_WIDTH} height={6} fill={headerColor} />
      <text x={10} y={HEADER_HEIGHT / 2} dy="0.35em" fill="#ffffff" fontSize={13} fontWeight={600}>
        {schema ? (
          <>
            <tspan fillOpacity={0.7} fontWeight={400}>
              {schema}.
            </tspan>
            <tspan>{name}</tspan>
          </>
        ) : (
          name
        )}
      </text>
      {columns.map((col, i) => {
        const cy = HEADER_HEIGHT + i * COLUMN_HEIGHT;
        const isPk = col.isPrimaryKey;
        const isFk = fkCols.has(col.name);
        return (
          <g key={col.name}>
            {i % 2 === 1 && (
              <rect
                x={0}
                y={cy}
                width={TABLE_WIDTH}
                height={COLUMN_HEIGHT}
                fill="rgba(0,0,0,0.02)"
              />
            )}
            {/* PK/FK marker */}
            {(isPk || isFk) && (
              <circle
                cx={12}
                cy={cy + COLUMN_HEIGHT / 2}
                r={4}
                fill={isPk ? colors.pkColor : colors.fkColor}
                stroke="#ffffff"
                strokeWidth={1}
              />
            )}
            <text
              x={24}
              y={cy + COLUMN_HEIGHT / 2}
              dy="0.35em"
              fontSize={11}
              fill={colors.columnText}
              fontWeight={isPk ? 600 : 400}
            >
              {col.name}
            </text>
            <text
              x={TABLE_WIDTH - 8}
              y={cy + COLUMN_HEIGHT / 2}
              dy="0.35em"
              fontSize={10}
              fill={colors.columnText}
              opacity={0.6}
              textAnchor="end"
            >
              {truncate(col.dataType, 14)}
            </text>
          </g>
        );
      })}
    </g>
  );
}

function Relationships({
  loaded,
  diagram,
  color,
}: {
  loaded: LoadedSchema;
  diagram: DiagramState;
  color: string;
}) {
  const paths: { path: string; id: string }[] = [];
  for (const fk of loaded.foreignKeys) {
    const srcKey = `${fk.sourceSchema}.${fk.sourceTable}`;
    const tgtKey = `${fk.targetSchema}.${fk.targetTable}`;
    const srcPos = diagram.positions[srcKey];
    const tgtPos = diagram.positions[tgtKey];
    if (!srcPos || !tgtPos) continue;
    const srcCols = loaded.columns[srcKey] ?? [];
    const tgtCols = loaded.columns[tgtKey] ?? [];
    if (srcCols.length === 0 || tgtCols.length === 0) continue;

    const a = columnAnchor(
      { x: srcPos.x, y: srcPos.y, columns: srcCols },
      fk.sourceColumn,
      { x: tgtPos.x },
    );
    const b = columnAnchor(
      { x: tgtPos.x, y: tgtPos.y, columns: tgtCols },
      fk.targetColumn,
      { x: srcPos.x },
    );

    const dx = Math.abs(b.x - a.x);
    const c1x = a.x + (a.x < b.x ? dx / 2 : -dx / 2);
    const c2x = b.x + (a.x < b.x ? -dx / 2 : dx / 2);
    const d = `M ${a.x} ${a.y} C ${c1x} ${a.y}, ${c2x} ${b.y}, ${b.x} ${b.y}`;
    paths.push({ path: d, id: `${fk.constraintName}-${fk.sourceColumn}` });
  }

  return (
    <g>
      {paths.map(({ path, id }) => (
        <path
          key={id}
          d={path}
          fill="none"
          stroke={color}
          strokeWidth={1.5}
          markerEnd="url(#arrowhead-fk)"
          opacity={0.75}
        />
      ))}
    </g>
  );
}

function AnnotationNode({
  annotation,
  selected,
  onSelect,
  onStartDrag,
}: {
  annotation: DiagramAnnotation;
  selected: boolean;
  onSelect: () => void;
  onStartDrag: (e: React.MouseEvent) => void;
}) {
  const handleMouseDown = (e: React.MouseEvent) => {
    onSelect();
    onStartDrag(e);
  };
  const stroke = selected ? "#2563eb" : annotation.color ?? "#64748b";
  if (annotation.shape === "rect") {
    return (
      <g onMouseDown={handleMouseDown} style={{ cursor: "move" }}>
        <rect
          x={annotation.x}
          y={annotation.y}
          width={annotation.width ?? 100}
          height={annotation.height ?? 60}
          fill={annotation.fill ?? "rgba(59,130,246,0.08)"}
          stroke={stroke}
          strokeWidth={selected ? 2 : 1.5}
          strokeDasharray={annotation.fill ? undefined : "4 2"}
          rx={4}
        />
        {annotation.text && (
          <text
            x={annotation.x + 8}
            y={annotation.y + 16}
            fontSize={annotation.fontSize ?? 12}
            fill={annotation.color ?? "#111827"}
          >
            {annotation.text}
          </text>
        )}
      </g>
    );
  }
  if (annotation.shape === "text") {
    return (
      <g onMouseDown={handleMouseDown} style={{ cursor: "move" }}>
        <text
          x={annotation.x}
          y={annotation.y + (annotation.fontSize ?? 14)}
          fontSize={annotation.fontSize ?? 14}
          fill={annotation.color ?? "#111827"}
          fontWeight={500}
        >
          {annotation.text ?? "Text"}
        </text>
        {selected && (
          <rect
            x={annotation.x - 2}
            y={annotation.y - 2}
            width={(annotation.width ?? 160) + 4}
            height={(annotation.height ?? 24) + 4}
            fill="none"
            stroke="#2563eb"
            strokeWidth={1}
            strokeDasharray="3 2"
          />
        )}
      </g>
    );
  }
  // line
  return (
    <g onMouseDown={handleMouseDown} style={{ cursor: "move" }}>
      <line
        x1={annotation.x}
        y1={annotation.y}
        x2={annotation.x2 ?? annotation.x + 80}
        y2={annotation.y2 ?? annotation.y}
        stroke={stroke}
        strokeWidth={annotation.strokeWidth ?? 2}
      />
    </g>
  );
}

// ──────────────────────────────────────────────────────────────────────────────
// Inspectors
// ──────────────────────────────────────────────────────────────────────────────

function TableInspector({
  tableKey,
  color,
  onSetColor,
  onClose,
}: {
  tableKey: string;
  color?: string;
  onSetColor: (c: string) => void;
  onClose: () => void;
}) {
  const [, tname] = tableKey.split(".", 2);
  return (
    <div className="absolute right-4 top-28 z-20 w-52 rounded-md border border-border bg-background p-3 shadow-lg">
      <div className="flex items-center justify-between">
        <div className="text-xs font-semibold">{tname}</div>
        <button
          onClick={onClose}
          className="rounded p-0.5 text-muted-foreground hover:bg-accent"
        >
          <X size={11} />
        </button>
      </div>
      <div className="mt-2 text-[10px] uppercase tracking-wide text-muted-foreground">
        Header color
      </div>
      <div className="mt-1 flex flex-wrap gap-1">
        {SWATCH_COLORS.map((c) => (
          <button
            key={c}
            onClick={() => onSetColor(c)}
            style={{ backgroundColor: c }}
            className={cn(
              "h-5 w-5 rounded border-2",
              color === c ? "border-foreground" : "border-transparent",
            )}
          />
        ))}
      </div>
    </div>
  );
}

function AnnotationInspector({
  annotation,
  onChange,
  onDelete,
  onClose,
}: {
  annotation: DiagramAnnotation;
  onChange: (patch: Partial<DiagramAnnotation>) => void;
  onDelete: () => void;
  onClose: () => void;
}) {
  return (
    <div className="absolute right-4 top-28 z-20 w-56 rounded-md border border-border bg-background p-3 shadow-lg">
      <div className="flex items-center justify-between">
        <div className="text-xs font-semibold capitalize">{annotation.shape}</div>
        <button
          onClick={onClose}
          className="rounded p-0.5 text-muted-foreground hover:bg-accent"
        >
          <X size={11} />
        </button>
      </div>
      {(annotation.shape === "text" || annotation.shape === "rect") && (
        <div className="mt-2">
          <div className="text-[10px] uppercase tracking-wide text-muted-foreground">
            Text
          </div>
          <input
            value={annotation.text ?? ""}
            onChange={(e) => onChange({ text: e.target.value })}
            className="mt-1 w-full rounded border border-border bg-background px-1.5 py-0.5 text-xs"
          />
        </div>
      )}
      <div className="mt-2 text-[10px] uppercase tracking-wide text-muted-foreground">
        Color
      </div>
      <div className="mt-1 flex flex-wrap gap-1">
        {SWATCH_COLORS.map((c) => (
          <button
            key={c}
            onClick={() => onChange({ color: c })}
            style={{ backgroundColor: c }}
            className={cn(
              "h-5 w-5 rounded border-2",
              annotation.color === c ? "border-foreground" : "border-transparent",
            )}
          />
        ))}
      </div>
      {annotation.shape === "rect" && (
        <>
          <div className="mt-2 text-[10px] uppercase tracking-wide text-muted-foreground">
            Fill
          </div>
          <div className="mt-1 flex flex-wrap gap-1">
            <button
              onClick={() => onChange({ fill: undefined })}
              className={cn(
                "h-5 w-5 rounded border-2 bg-white",
                !annotation.fill ? "border-foreground" : "border-transparent",
              )}
              title="No fill"
            />
            {SWATCH_COLORS.map((c) => (
              <button
                key={c}
                onClick={() => onChange({ fill: c + "33" })}
                style={{ backgroundColor: c + "33" }}
                className={cn(
                  "h-5 w-5 rounded border-2",
                  annotation.fill === c + "33" ? "border-foreground" : "border-transparent",
                )}
              />
            ))}
          </div>
        </>
      )}
      <button
        onClick={onDelete}
        className="mt-3 flex w-full items-center justify-center gap-1 rounded bg-destructive/10 py-1 text-xs text-destructive hover:bg-destructive/20"
      >
        <Trash2 size={11} /> Delete
      </button>
    </div>
  );
}

function ColorPicker({
  colors,
  onChange,
  onClose,
}: {
  colors: DiagramColorSettings;
  onChange: (patch: Partial<DiagramColorSettings>) => void;
  onClose: () => void;
}) {
  const fields: { key: keyof DiagramColorSettings; label: string; icon: React.ReactNode }[] = [
    { key: "tableHeader", label: "Table header", icon: null },
    { key: "tableBorder", label: "Table border", icon: null },
    { key: "pkColor", label: "Primary key", icon: <Key size={10} /> },
    { key: "fkColor", label: "Foreign key", icon: <Link2 size={10} /> },
    { key: "relationship", label: "Relationship", icon: null },
    { key: "columnText", label: "Column text", icon: null },
  ];
  return (
    <div className="flex shrink-0 items-center gap-3 border-b border-border bg-muted/10 px-3 py-2 text-xs">
      {fields.map((f) => (
        <div key={f.key} className="flex items-center gap-1">
          {f.icon}
          <span className="text-[10px] text-muted-foreground">{f.label}</span>
          <input
            type="color"
            value={colors[f.key]}
            onChange={(e) => onChange({ [f.key]: e.target.value } as Partial<DiagramColorSettings>)}
            className="h-5 w-6 cursor-pointer rounded border border-border"
          />
        </div>
      ))}
      <button
        onClick={onClose}
        className="ml-auto rounded p-1 text-muted-foreground hover:bg-accent"
      >
        <X size={12} />
      </button>
    </div>
  );
}

// ──────────────────────────────────────────────────────────────────────────────
// Utilities
// ──────────────────────────────────────────────────────────────────────────────

function truncate(s: string, n: number): string {
  return s.length > n ? s.slice(0, n - 1) + "…" : s;
}

function computeViewBox(
  diagram: DiagramState,
  loaded: LoadedSchema | null,
): { x: number; y: number; width: number; height: number } {
  let minX = 0,
    minY = 0,
    maxX = 1200,
    maxY = 800;
  const apply = (x: number, y: number, w = 0, h = 0) => {
    minX = Math.min(minX, x);
    minY = Math.min(minY, y);
    maxX = Math.max(maxX, x + w);
    maxY = Math.max(maxY, y + h);
  };
  if (loaded) {
    for (const t of loaded.tables) {
      const key = `${t.schema}.${t.name}`;
      const p = diagram.positions[key];
      if (!p) continue;
      const cols = loaded.columns[key] ?? [];
      apply(p.x, p.y, TABLE_WIDTH, tableHeight(cols.length));
    }
  }
  for (const a of diagram.annotations) {
    if (a.shape === "line") {
      apply(Math.min(a.x, a.x2 ?? a.x), Math.min(a.y, a.y2 ?? a.y));
      apply(Math.max(a.x, a.x2 ?? a.x), Math.max(a.y, a.y2 ?? a.y));
    } else {
      apply(a.x, a.y, a.width ?? 0, a.height ?? 0);
    }
  }
  // Provide some padding
  const padding = 40;
  return {
    x: minX - padding,
    y: minY - padding,
    width: maxX - minX + padding * 2,
    height: maxY - minY + padding * 2,
  };
}

function fitToContent(
  svg: SVGSVGElement | null,
  diagram: DiagramState,
  loaded: LoadedSchema | null,
  setDiagram: (updater: (d: DiagramState) => DiagramState) => void,
) {
  if (!svg) return;
  const container = svg.parentElement;
  if (!container) return;
  const box = computeViewBox(diagram, loaded);
  const cw = container.clientWidth;
  const ch = container.clientHeight;
  if (box.width <= 0 || box.height <= 0) return;
  const zoom = Math.min(cw / box.width, ch / box.height, 1);
  const panX = (cw - box.width * zoom) / 2 - box.x * zoom;
  const panY = (ch - box.height * zoom) / 2 - box.y * zoom;
  setDiagram((d) => ({ ...d, zoom, panX, panY }));
  void columnY;
}

/**
 * Prepare a cloned <svg> for export: sets viewBox to tightly fit all content
 * and strips the interactive pan/zoom transform so the output renders at 1:1.
 */
function prepareExportSvg(
  svg: SVGSVGElement,
  diagram: DiagramState,
  loaded: LoadedSchema | null,
): { clone: SVGSVGElement; width: number; height: number } {
  const box = computeViewBox(diagram, loaded);
  const clone = svg.cloneNode(true) as SVGSVGElement;
  clone.setAttribute("xmlns", "http://www.w3.org/2000/svg");
  clone.setAttribute("xmlns:xlink", "http://www.w3.org/1999/xlink");
  clone.setAttribute("viewBox", `${box.x} ${box.y} ${box.width} ${box.height}`);
  clone.setAttribute("width", String(Math.max(1, Math.round(box.width))));
  clone.setAttribute("height", String(Math.max(1, Math.round(box.height))));
  clone.removeAttribute("class");
  // Strip the interactive pan/zoom transform from the top-level <g>.
  const topG = clone.querySelector(":scope > g");
  if (topG) topG.removeAttribute("transform");
  return { clone, width: box.width, height: box.height };
}

// ──────────────────────────────────────────────────────────────────────────────
// Export handler
// ──────────────────────────────────────────────────────────────────────────────

async function handleExport(
  kind: "png" | "svg" | "pdf" | "drawio",
  svg: SVGSVGElement | null,
  diagram: DiagramState,
  loaded: LoadedSchema | null,
) {
  if (!svg) return;
  const base = `schema-${diagram.schemaName || "diagram"}`;
  try {
    if (kind === "drawio") {
      if (!loaded) return;
      const xml = buildDrawioXml(diagram, loaded.tables, loaded.columns, loaded.foreignKeys);
      await saveTextAs(xml, `${base}.drawio`, [{ name: "drawio", extensions: ["drawio", "xml"] }]);
      return;
    }
    const { clone, width, height } = prepareExportSvg(svg, diagram, loaded);
    if (kind === "svg") {
      const xml = serializeSvg(clone);
      await saveTextAs(xml, `${base}.svg`, [{ name: "SVG", extensions: ["svg"] }]);
      return;
    }
    if (kind === "png") {
      const blob = await svgToPngBlob(clone, 2);
      const bytes = new Uint8Array(await blob.arrayBuffer());
      await saveBytesAs(bytes, `${base}.png`, [{ name: "PNG", extensions: ["png"] }]);
      return;
    }
    if (kind === "pdf") {
      const png = await svgToPngBlob(clone, 2);
      const pdf = await pngBlobToPdf(png, width, height);
      const bytes = new Uint8Array(await pdf.arrayBuffer());
      await saveBytesAs(bytes, `${base}.pdf`, [{ name: "PDF", extensions: ["pdf"] }]);
      return;
    }
  } catch (err) {
    console.error("Export failed:", err);
  }
}
