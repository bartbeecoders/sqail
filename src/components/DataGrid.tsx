import { Fragment, useMemo, useRef, useState, useCallback, useEffect } from "react";
import {
  useReactTable,
  getCoreRowModel,
  getSortedRowModel,
  getFilteredRowModel,
  flexRender,
  type ColumnDef,
  type SortingState,
  type ColumnFiltersState,
} from "@tanstack/react-table";
import { useVirtualizer } from "@tanstack/react-virtual";
import { ArrowUp, ArrowDown, ArrowUpDown, Search, X, Filter, Copy, ClipboardList, FileJson, Table2, Parentheses } from "lucide-react";
import { cn } from "../lib/utils";
import { valuesToInList } from "../lib/sqlValues";
import type { QueryColumn } from "../types/query";

type CellValue = string | number | boolean | null;

interface DataGridProps {
  columns: QueryColumn[];
  rows: CellValue[][];
}

interface ContextMenu {
  x: number;
  y: number;
}

interface SelectedCell {
  rowIdx: number;
  colIdx: number;
}

function formatCell(value: CellValue): React.ReactNode {
  if (value === null) {
    return <span className="italic text-muted-foreground/50">NULL</span>;
  }
  if (typeof value === "boolean") {
    return <span className={value ? "text-success" : "text-destructive"}>{String(value)}</span>;
  }
  if (typeof value === "number") {
    return <span className="tabular-nums">{value}</span>;
  }
  const str = String(value);
  if (str.length > 200) {
    return <span title={str}>{str.slice(0, 200)}...</span>;
  }
  return str;
}

function cellToString(value: CellValue): string {
  return value === null ? "NULL" : String(value);
}

function rowToTsv(row: CellValue[]): string {
  return row.map(cellToString).join("\t");
}

function rowsToCsv(rows: CellValue[][], columns: QueryColumn[]): string {
  const header = columns.map((c) => `"${c.name.replace(/"/g, '""')}"`).join(",");
  const body = rows.map((row) =>
    row.map((v) => {
      const s = cellToString(v);
      return `"${s.replace(/"/g, '""')}"`;
    }).join(","),
  );
  return [header, ...body].join("\n");
}

function rowsToJson(rows: CellValue[][], columns: QueryColumn[]): string {
  const objects = rows.map((row) => {
    const obj: Record<string, CellValue> = {};
    columns.forEach((col, i) => {
      obj[col.name] = row[i];
    });
    return obj;
  });
  return JSON.stringify(objects, null, 2);
}

function rowsToMarkdown(rows: CellValue[][], columns: QueryColumn[]): string {
  const header = "| " + columns.map((c) => c.name).join(" | ") + " |";
  const separator = "| " + columns.map(() => "---").join(" | ") + " |";
  const body = rows.map((row) =>
    "| " + row.map((v) => cellToString(v).replace(/\|/g, "\\|")).join(" | ") + " |",
  );
  return [header, separator, ...body].join("\n");
}

export default function DataGrid({ columns, rows }: DataGridProps) {
  const [sorting, setSorting] = useState<SortingState>([]);
  const [columnFilters, setColumnFilters] = useState<ColumnFiltersState>([]);
  const [globalFilter, setGlobalFilter] = useState("");
  const [showColumnFilters, setShowColumnFilters] = useState(false);
  const parentRef = useRef<HTMLDivElement>(null);

  // Selection state: set of visible row indices (from tableRows, not original data)
  const [selectedRows, setSelectedRows] = useState<Set<number>>(new Set());
  const lastClickedRow = useRef<number | null>(null);
  // Single-cell selection (set by double-click, cleared by row click)
  const [selectedCell, setSelectedCell] = useState<SelectedCell | null>(null);
  // Column selection (Ctrl/Shift+click on header)
  const [selectedColumns, setSelectedColumns] = useState<Set<number>>(new Set());
  const lastClickedColumn = useRef<number | null>(null);
  // Drag-select state
  const isDragging = useRef(false);

  // Context menu
  const [contextMenu, setContextMenu] = useState<ContextMenu | null>(null);
  const contextRef = useRef<HTMLDivElement>(null);

  const [colWidths, setColWidths] = useState<number[]>(() =>
    columns.map(() => 150),
  );

  // Reset widths & selection when columns change
  useEffect(() => {
    setColWidths(columns.map(() => 150));
    setSelectedRows(new Set());
    setSelectedCell(null);
    setSelectedColumns(new Set());
    lastClickedRow.current = null;
    lastClickedColumn.current = null;
  }, [columns]);

  // Close context menu on outside click / escape
  useEffect(() => {
    if (!contextMenu) return;
    const onMouseDown = (e: MouseEvent) => {
      if (contextRef.current && !contextRef.current.contains(e.target as Node)) {
        setContextMenu(null);
      }
    };
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") setContextMenu(null);
    };
    document.addEventListener("mousedown", onMouseDown);
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("mousedown", onMouseDown);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [contextMenu]);

  const resizing = useRef<{ colIdx: number; startX: number; startW: number } | null>(null);

  const onResizeStart = useCallback(
    (e: React.MouseEvent, colIdx: number) => {
      e.preventDefault();
      e.stopPropagation();
      resizing.current = { colIdx, startX: e.clientX, startW: colWidths[colIdx] };

      const onMove = (ev: MouseEvent) => {
        if (!resizing.current) return;
        const delta = ev.clientX - resizing.current.startX;
        const newW = Math.max(50, resizing.current.startW + delta);
        setColWidths((prev) => {
          const next = [...prev];
          next[resizing.current!.colIdx] = newW;
          return next;
        });
      };

      const onUp = () => {
        resizing.current = null;
        document.removeEventListener("mousemove", onMove);
        document.removeEventListener("mouseup", onUp);
        document.body.style.cursor = "";
        document.body.style.userSelect = "";
      };

      document.body.style.cursor = "col-resize";
      document.body.style.userSelect = "none";
      document.addEventListener("mousemove", onMove);
      document.addEventListener("mouseup", onUp);
    },
    [colWidths],
  );

  const gridTemplateColumns = `40px ${colWidths.map((w) => `${w}px`).join(" ")}`;

  const columnDefs = useMemo<ColumnDef<CellValue[]>[]>(
    () =>
      columns.map((col, idx) => ({
        id: `col_${idx}`,
        header: col.name,
        meta: { typeName: col.typeName },
        accessorFn: (row) => row[idx],
        cell: ({ getValue }) => formatCell(getValue() as CellValue),
        filterFn: (row, columnId, filterValue: string) => {
          const val = row.getValue(columnId) as CellValue;
          if (!filterValue) return true;
          if (val === null) return "null".includes(filterValue.toLowerCase());
          return String(val).toLowerCase().includes(filterValue.toLowerCase());
        },
        sortingFn: (rowA, rowB, columnId) => {
          const a = rowA.getValue(columnId) as CellValue;
          const b = rowB.getValue(columnId) as CellValue;
          if (a === null && b === null) return 0;
          if (a === null) return -1;
          if (b === null) return 1;
          if (typeof a === "number" && typeof b === "number") return a - b;
          return String(a).localeCompare(String(b));
        },
      })),
    [columns],
  );

  const globalFilterFn = useCallback(
    (row: { getValue: (id: string) => unknown }, _columnId: string, filterValue: string) => {
      if (!filterValue) return true;
      const lower = filterValue.toLowerCase();
      for (let i = 0; i < columns.length; i++) {
        const val = row.getValue(`col_${i}`) as CellValue;
        if (val === null) {
          if ("null".includes(lower)) return true;
          continue;
        }
        if (String(val).toLowerCase().includes(lower)) return true;
      }
      return false;
    },
    [columns],
  );

  const table = useReactTable({
    data: rows,
    columns: columnDefs,
    state: { sorting, columnFilters, globalFilter },
    onSortingChange: setSorting,
    onColumnFiltersChange: setColumnFilters,
    onGlobalFilterChange: setGlobalFilter,
    getCoreRowModel: getCoreRowModel(),
    getSortedRowModel: getSortedRowModel(),
    getFilteredRowModel: getFilteredRowModel(),
    globalFilterFn,
  });

  const tableRows = table.getRowModel().rows;
  const totalRows = rows.length;
  const filteredCount = tableRows.length;
  const isFiltered = globalFilter || columnFilters.length > 0;

  // Clear selection when filters/sorting change (visible row indices shift)
  useEffect(() => {
    setSelectedRows(new Set());
    setSelectedCell(null);
    lastClickedRow.current = null;
  }, [sorting, columnFilters, globalFilter]);

  const handleRowMouseDown = useCallback(
    (e: React.MouseEvent, visibleIdx: number) => {
      // Clear cell + column selection on row click
      setSelectedCell(null);
      setSelectedColumns(new Set());
      lastClickedColumn.current = null;

      if (e.shiftKey && lastClickedRow.current !== null) {
        // Range select
        setSelectedRows((prev) => {
          const next = new Set(prev);
          const start = Math.min(lastClickedRow.current!, visibleIdx);
          const end = Math.max(lastClickedRow.current!, visibleIdx);
          for (let i = start; i <= end; i++) {
            next.add(i);
          }
          return next;
        });
      } else if (e.ctrlKey || e.metaKey) {
        // Toggle single row
        setSelectedRows((prev) => {
          const next = new Set(prev);
          if (next.has(visibleIdx)) {
            next.delete(visibleIdx);
          } else {
            next.add(visibleIdx);
          }
          return next;
        });
      } else {
        // Single select + start drag
        setSelectedRows(new Set([visibleIdx]));
        isDragging.current = true;
      }
      lastClickedRow.current = visibleIdx;
    },
    [],
  );

  const handleRowMouseEnter = useCallback(
    (visibleIdx: number) => {
      if (!isDragging.current || lastClickedRow.current === null) return;
      const start = Math.min(lastClickedRow.current, visibleIdx);
      const end = Math.max(lastClickedRow.current, visibleIdx);
      const next = new Set<number>();
      for (let i = start; i <= end; i++) {
        next.add(i);
      }
      setSelectedRows(next);
    },
    [],
  );

  // Stop drag on mouseup anywhere
  useEffect(() => {
    const onMouseUp = () => { isDragging.current = false; };
    document.addEventListener("mouseup", onMouseUp);
    return () => document.removeEventListener("mouseup", onMouseUp);
  }, []);

  const handleCellDoubleClick = useCallback(
    (e: React.MouseEvent, rowIdx: number, colIdx: number) => {
      e.stopPropagation();
      // Select single cell, deselect rows + columns
      setSelectedRows(new Set());
      setSelectedColumns(new Set());
      lastClickedColumn.current = null;
      setSelectedCell({ rowIdx, colIdx });
    },
    [],
  );

  const handleHeaderClick = useCallback(
    (
      e: React.MouseEvent,
      colIdx: number,
      sortHandler: ((event: unknown) => void) | undefined,
    ) => {
      // Shift / Ctrl / Cmd → column selection (overrides sort)
      if (e.shiftKey && lastClickedColumn.current !== null) {
        e.preventDefault();
        const start = Math.min(lastClickedColumn.current, colIdx);
        const end = Math.max(lastClickedColumn.current, colIdx);
        const next = new Set<number>();
        for (let i = start; i <= end; i++) next.add(i);
        setSelectedColumns(next);
        setSelectedRows(new Set());
        setSelectedCell(null);
        return;
      }
      if (e.ctrlKey || e.metaKey) {
        e.preventDefault();
        setSelectedColumns((prev) => {
          const next = new Set(prev);
          if (next.has(colIdx)) next.delete(colIdx);
          else next.add(colIdx);
          return next;
        });
        setSelectedRows(new Set());
        setSelectedCell(null);
        lastClickedColumn.current = colIdx;
        return;
      }
      // Plain click → sort. Still record the column as the anchor so a
      // following Shift+click range-selects from here.
      lastClickedColumn.current = colIdx;
      sortHandler?.(e);
    },
    [],
  );

  const handleHeaderDragStart = useCallback(
    (e: React.DragEvent, colIdx: number) => {
      // If the dragged column isn't already part of the selection,
      // make it the only selected column for clear visual feedback.
      let toExport: number[];
      if (selectedColumns.has(colIdx) && selectedColumns.size > 0) {
        toExport = Array.from(selectedColumns).sort((a, b) => a - b);
      } else {
        toExport = [colIdx];
        setSelectedColumns(new Set([colIdx]));
        setSelectedRows(new Set());
        setSelectedCell(null);
        lastClickedColumn.current = colIdx;
      }

      const payload = toExport.map((idx) => ({
        name: columns[idx].name,
        values: tableRows.map((r) => (r.original as CellValue[])[idx]),
      }));

      e.dataTransfer.setData(
        "application/sqlai-column-data",
        JSON.stringify(payload),
      );
      e.dataTransfer.effectAllowed = "copy";
    },
    [selectedColumns, tableRows, columns],
  );

  const handleContextMenu = useCallback(
    (e: React.MouseEvent, visibleIdx: number) => {
      e.preventDefault();
      // If right-clicked row isn't selected, select only it
      if (!selectedRows.has(visibleIdx)) {
        setSelectedRows(new Set([visibleIdx]));
        lastClickedRow.current = visibleIdx;
      }
      // A right-click on a row is a row-context action — clear column selection
      // so the menu shows the row commands.
      setSelectedColumns(new Set());
      lastClickedColumn.current = null;
      setContextMenu({ x: e.clientX, y: e.clientY });
    },
    [selectedRows],
  );

  const handleHeaderContextMenu = useCallback(
    (e: React.MouseEvent, colIdx: number) => {
      e.preventDefault();
      if (!selectedColumns.has(colIdx)) {
        setSelectedColumns(new Set([colIdx]));
        setSelectedRows(new Set());
        setSelectedCell(null);
        lastClickedColumn.current = colIdx;
      }
      setContextMenu({ x: e.clientX, y: e.clientY });
    },
    [selectedColumns],
  );

  // Get the actual data rows for the current selection. When columns are
  // selected, returns all visible rows narrowed to those columns. When rows
  // are selected, returns the chosen rows across all columns.
  const getSelectedData = useCallback((): {
    rows: CellValue[][];
    cols: QueryColumn[];
  } => {
    if (selectedColumns.size > 0) {
      const colIndices = Array.from(selectedColumns).sort((a, b) => a - b);
      const cols = colIndices.map((i) => columns[i]);
      const rows = tableRows.map((r) =>
        colIndices.map((i) => (r.original as CellValue[])[i]),
      );
      return { rows, cols };
    }
    const sorted = Array.from(selectedRows).sort((a, b) => a - b);
    const rows = sorted
      .map((idx) => tableRows[idx]?.original as CellValue[] | undefined)
      .filter((r): r is CellValue[] => Boolean(r));
    return { rows, cols: columns };
  }, [selectedRows, selectedColumns, tableRows, columns]);

  const copyAs = useCallback(
    (format: "tsv" | "csv" | "json" | "markdown") => {
      const { rows: data, cols } = getSelectedData();
      if (data.length === 0) return;
      let text: string;
      switch (format) {
        case "tsv":
          text = data.map(rowToTsv).join("\n");
          break;
        case "csv":
          text = rowsToCsv(data, cols);
          break;
        case "json":
          text = rowsToJson(data, cols);
          break;
        case "markdown":
          text = rowsToMarkdown(data, cols);
          break;
      }
      navigator.clipboard.writeText(text);
      setContextMenu(null);
    },
    [getSelectedData],
  );

  const copyColumnsAsInList = useCallback(() => {
    if (selectedColumns.size === 0) return;
    const colIndices = Array.from(selectedColumns).sort((a, b) => a - b);
    const blocks = colIndices.map((idx) => {
      const values = tableRows.map((r) => (r.original as CellValue[])[idx]);
      const list = valuesToInList(values);
      return colIndices.length > 1 ? `-- ${columns[idx].name}\n${list}` : list;
    });
    navigator.clipboard.writeText(blocks.join("\n\n"));
    setContextMenu(null);
  }, [selectedColumns, tableRows, columns]);

  const selectAll = useCallback(() => {
    const all = new Set<number>();
    for (let i = 0; i < tableRows.length; i++) all.add(i);
    setSelectedRows(all);
  }, [tableRows.length]);

  // Copy a single cell value to clipboard
  const copyCellValue = useCallback(() => {
    if (!selectedCell) return;
    const row = tableRows[selectedCell.rowIdx]?.original;
    if (!row) return;
    const value = row[selectedCell.colIdx];
    navigator.clipboard.writeText(cellToString(value));
  }, [selectedCell, tableRows]);

  // Keyboard: Ctrl+A to select all, Ctrl+C to copy, Escape to deselect cell
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (!parentRef.current?.contains(document.activeElement) &&
          document.activeElement !== parentRef.current) return;

      if ((e.ctrlKey || e.metaKey) && e.key === "a") {
        e.preventDefault();
        setSelectedCell(null);
        setSelectedColumns(new Set());
        selectAll();
      }
      if ((e.ctrlKey || e.metaKey) && e.key === "c") {
        if (selectedCell) {
          e.preventDefault();
          copyCellValue();
        } else if (selectedRows.size > 0 || selectedColumns.size > 0) {
          e.preventDefault();
          copyAs("tsv");
        }
      }
      if (e.key === "Escape") {
        if (selectedCell) setSelectedCell(null);
        if (selectedColumns.size > 0) {
          setSelectedColumns(new Set());
          lastClickedColumn.current = null;
        }
      }
    };
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [selectAll, copyAs, selectedRows.size, selectedColumns.size, selectedCell, copyCellValue]);

  const virtualizer = useVirtualizer({
    count: tableRows.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 32,
    overscan: 20,
  });

  const hasActiveColumnFilters = columnFilters.some((f) => f.value);

  return (
    <div className="flex flex-1 flex-col overflow-hidden">
      {/* Search / filter toolbar */}
      <div className="flex h-8 shrink-0 items-center gap-2 border-b border-border bg-muted/30 px-2">
        <div className="relative flex-1 max-w-xs">
          <Search size={11} className="absolute left-2 top-1/2 -translate-y-1/2 text-muted-foreground" />
          <input
            type="text"
            value={globalFilter}
            onChange={(e) => setGlobalFilter(e.target.value)}
            placeholder="Search results..."
            className="input h-6 w-full pl-6 pr-6 text-[11px]"
          />
          {globalFilter && (
            <button
              onClick={() => setGlobalFilter("")}
              className="absolute right-1.5 top-1/2 -translate-y-1/2 rounded p-0.5 text-muted-foreground hover:text-foreground"
            >
              <X size={10} />
            </button>
          )}
        </div>
        <button
          onClick={() => setShowColumnFilters((v) => !v)}
          className={cn(
            "flex items-center gap-1 rounded px-1.5 py-0.5 text-[11px] transition-colors",
            showColumnFilters || hasActiveColumnFilters
              ? "bg-primary/10 text-primary"
              : "text-muted-foreground hover:bg-accent hover:text-accent-foreground",
          )}
          title="Toggle column filters"
        >
          <Filter size={11} />
          <span>Filters</span>
        </button>
        {isFiltered && (
          <span className="text-[10px] text-muted-foreground">
            {filteredCount.toLocaleString()} of {totalRows.toLocaleString()}
          </span>
        )}
        {selectedCell && (
          <span className="text-[10px] text-muted-foreground ml-auto">
            Cell selected
          </span>
        )}
        {!selectedCell && selectedRows.size > 0 && (
          <span className="text-[10px] text-muted-foreground ml-auto">
            {selectedRows.size} row{selectedRows.size !== 1 && "s"} selected
          </span>
        )}
        {!selectedCell && selectedRows.size === 0 && selectedColumns.size > 0 && (
          <span className="text-[10px] text-muted-foreground ml-auto">
            {selectedColumns.size} column{selectedColumns.size !== 1 && "s"} selected · drag to editor for IN-list
          </span>
        )}
      </div>

      {/* Data grid */}
      <div ref={parentRef} className="flex-1 overflow-auto focus:outline-none" tabIndex={0}>
        <div className="min-w-fit text-xs">
          {/* Sticky header */}
          <div className="sticky top-0 z-10 bg-muted">
            {table.getHeaderGroups().map((headerGroup) => (
              <Fragment key={headerGroup.id}>
                {/* Header row */}
                <div className="grid" style={{ gridTemplateColumns }}>
                  {/* Row number column */}
                  <div className="border-b border-r border-border px-2 py-1.5 text-right text-[10px] font-normal text-muted-foreground">
                    #
                  </div>
                  {headerGroup.headers.map((header, hIdx) => {
                    const sorted = header.column.getIsSorted();
                    const meta = header.column.columnDef.meta as
                      | { typeName: string }
                      | undefined;
                    const isColSelected = selectedColumns.has(hIdx);
                    const sortHandler = header.column.getToggleSortingHandler();
                    return (
                      <div
                        key={header.id}
                        draggable
                        onDragStart={(e) => handleHeaderDragStart(e, hIdx)}
                        onClick={(e) => handleHeaderClick(e, hIdx, sortHandler)}
                        onContextMenu={(e) => handleHeaderContextMenu(e, hIdx)}
                        className={cn(
                          "relative cursor-pointer select-none border-b border-r border-border px-2 py-1.5 text-left",
                          isColSelected ? "bg-primary/15" : "hover:bg-accent/50",
                        )}
                        title={`${header.column.columnDef.header as string} — click to sort, Ctrl+click to select column, drag to insert as IN-list`}
                      >
                        <div className="flex items-center gap-1.5">
                          <div className="flex flex-col gap-0">
                            <span className="font-semibold leading-tight">
                              {header.column.columnDef.header as string}
                            </span>
                            {meta?.typeName && (
                              <span className="text-[9px] font-normal leading-tight opacity-40">
                                {meta.typeName}
                              </span>
                            )}
                          </div>
                          <span className="ml-auto shrink-0 opacity-50">
                            {sorted === "asc" ? (
                              <ArrowUp size={11} />
                            ) : sorted === "desc" ? (
                              <ArrowDown size={11} />
                            ) : (
                              <ArrowUpDown size={9} className="opacity-30" />
                            )}
                          </span>
                        </div>
                        {/* Resize handle */}
                        <div
                          draggable={false}
                          onDragStart={(e) => e.preventDefault()}
                          onMouseDown={(e) => onResizeStart(e, hIdx)}
                          className="absolute right-0 top-0 h-full w-1.5 cursor-col-resize hover:bg-primary/30 active:bg-primary/50"
                        />
                      </div>
                    );
                  })}
                </div>

                {/* Column filter row */}
                {showColumnFilters && (
                  <div className="grid" style={{ gridTemplateColumns }}>
                    <div className="border-b border-r border-border bg-muted/60 px-1 py-0.5" />
                    {headerGroup.headers.map((header) => (
                      <div
                        key={`${header.id}-filter`}
                        className="border-b border-r border-border bg-muted/60 px-1 py-0.5"
                      >
                        <ColumnFilterInput
                          value={(header.column.getFilterValue() as string) ?? ""}
                          onChange={(value) => header.column.setFilterValue(value || undefined)}
                          placeholder={`Filter ${header.column.columnDef.header as string}...`}
                        />
                      </div>
                    ))}
                  </div>
                )}
              </Fragment>
            ))}
          </div>

          {/* Virtualized body */}
          <div
            style={{
              height: `${virtualizer.getTotalSize()}px`,
              position: "relative",
            }}
          >
            {virtualizer.getVirtualItems().map((virtualRow) => {
              const row = tableRows[virtualRow.index];
              const isSelected = selectedRows.has(virtualRow.index);
              return (
                <div
                  key={row.id}
                  data-index={virtualRow.index}
                  ref={(node) => virtualizer.measureElement(node)}
                  className={cn(
                    "absolute left-0 w-full grid",
                    isSelected
                      ? "bg-primary/10"
                      : virtualRow.index % 2 === 0
                        ? "bg-background"
                        : "bg-muted/30",
                    !isSelected && "hover:bg-primary/5",
                  )}
                  style={{
                    transform: `translateY(${virtualRow.start}px)`,
                    gridTemplateColumns,
                  }}
                  onMouseDown={(e) => handleRowMouseDown(e, virtualRow.index)}
                  onMouseEnter={() => handleRowMouseEnter(virtualRow.index)}
                  onContextMenu={(e) => handleContextMenu(e, virtualRow.index)}
                >
                  {/* Row number */}
                  <div
                    className={cn(
                      "border-r border-border px-2 py-1 text-right text-[10px] tabular-nums",
                      isSelected
                        ? "text-primary font-medium"
                        : "text-muted-foreground/50",
                    )}
                  >
                    {virtualRow.index + 1}
                  </div>
                  {row.getVisibleCells().map((cell, cellIdx) => {
                    const isCellSelected =
                      selectedCell?.rowIdx === virtualRow.index &&
                      selectedCell?.colIdx === cellIdx;
                    const isColSelected = selectedColumns.has(cellIdx);
                    return (
                      <div
                        key={cell.id}
                        className={cn(
                          "truncate border-r border-border px-2 py-1",
                          isColSelected && !isSelected && "bg-primary/10",
                          isCellSelected && "ring-2 ring-inset ring-primary bg-primary/15",
                        )}
                        onDoubleClick={(e) => handleCellDoubleClick(e, virtualRow.index, cellIdx)}
                      >
                        {flexRender(cell.column.columnDef.cell, cell.getContext())}
                      </div>
                    );
                  })}
                </div>
              );
            })}
          </div>
        </div>
      </div>

      {/* Context menu */}
      {contextMenu && (selectedRows.size > 0 || selectedCell || selectedColumns.size > 0) && (
        <div
          ref={contextRef}
          className="fixed z-50 min-w-52 rounded-md border border-border bg-background py-1 shadow-lg text-xs"
          style={{ left: contextMenu.x, top: contextMenu.y }}
        >
          {selectedCell && (
            <>
              <CtxItem icon={Copy} label="Copy cell" shortcut="Ctrl+C" onClick={() => { copyCellValue(); setContextMenu(null); }} />
              <div className="my-1 border-t border-border" />
            </>
          )}
          {selectedColumns.size > 0 && (
            <>
              <CtxItem icon={Copy} label={`Copy column${selectedColumns.size !== 1 ? "s" : ""}`} shortcut="Ctrl+C" onClick={() => copyAs("tsv")} />
              <CtxItem icon={Parentheses} label="Copy as IN-list" onClick={copyColumnsAsInList} />
              <CtxItem icon={ClipboardList} label="Copy as CSV" onClick={() => copyAs("csv")} />
              <CtxItem icon={FileJson} label="Copy as JSON" onClick={() => copyAs("json")} />
              <CtxItem icon={Table2} label="Copy as Markdown" onClick={() => copyAs("markdown")} />
              <div className="my-1 border-t border-border" />
            </>
          )}
          {selectedRows.size > 0 && (
            <>
              <CtxItem icon={Copy} label={selectedCell ? "Copy row(s)" : "Copy"} shortcut={selectedCell ? undefined : "Ctrl+C"} onClick={() => copyAs("tsv")} />
              <CtxItem icon={ClipboardList} label="Copy as CSV" onClick={() => copyAs("csv")} />
              <CtxItem icon={FileJson} label="Copy as JSON" onClick={() => copyAs("json")} />
              <CtxItem icon={Table2} label="Copy as Markdown" onClick={() => copyAs("markdown")} />
              <div className="my-1 border-t border-border" />
            </>
          )}
          <CtxItem
            label={`Select all (${tableRows.length})`}
            onClick={() => {
              setSelectedCell(null);
              setSelectedColumns(new Set());
              selectAll();
              setContextMenu(null);
            }}
          />
        </div>
      )}
    </div>
  );
}

function CtxItem({
  label,
  onClick,
  icon: Icon,
  shortcut,
}: {
  label: string;
  onClick: () => void;
  icon?: React.ComponentType<{ size?: number; className?: string }>;
  shortcut?: string;
}) {
  return (
    <button
      onClick={onClick}
      className="flex w-full items-center gap-2 px-3 py-1 text-left text-xs hover:bg-accent hover:text-accent-foreground"
    >
      {Icon && <Icon size={12} className="shrink-0 opacity-60" />}
      <span className="flex-1">{label}</span>
      {shortcut && <span className="text-[10px] text-muted-foreground/50">{shortcut}</span>}
    </button>
  );
}

function ColumnFilterInput({
  value,
  onChange,
  placeholder,
}: {
  value: string;
  onChange: (value: string) => void;
  placeholder: string;
}) {
  return (
    <div className="relative">
      <input
        type="text"
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={placeholder}
        className="input h-5 w-full px-1.5 text-[10px] font-normal"
      />
      {value && (
        <button
          onClick={() => onChange("")}
          className="absolute right-0.5 top-1/2 -translate-y-1/2 rounded p-0.5 text-muted-foreground hover:text-foreground"
        >
          <X size={8} />
        </button>
      )}
    </div>
  );
}
