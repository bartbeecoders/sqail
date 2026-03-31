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
import { ArrowUp, ArrowDown, ArrowUpDown, Search, X, Filter, Check } from "lucide-react";
import { cn } from "../lib/utils";
import type { QueryColumn } from "../types/query";

/** Flash a "Copied" tooltip near the click position. */
function useCopyFeedback() {
  const [copiedId, setCopiedId] = useState<string | null>(null);
  const timeoutRef = useRef<ReturnType<typeof setTimeout>>(undefined);

  const showCopied = useCallback((id: string) => {
    if (timeoutRef.current) clearTimeout(timeoutRef.current);
    setCopiedId(id);
    timeoutRef.current = setTimeout(() => setCopiedId(null), 1200);
  }, []);

  return { copiedId, showCopied };
}

type CellValue = string | number | boolean | null;

interface DataGridProps {
  columns: QueryColumn[];
  rows: CellValue[][];
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

export default function DataGrid({ columns, rows }: DataGridProps) {
  const [sorting, setSorting] = useState<SortingState>([]);
  const [columnFilters, setColumnFilters] = useState<ColumnFiltersState>([]);
  const [globalFilter, setGlobalFilter] = useState("");
  const [showColumnFilters, setShowColumnFilters] = useState(false);
  const parentRef = useRef<HTMLDivElement>(null);
  const { copiedId, showCopied } = useCopyFeedback();

  const copyCell = useCallback(
    (rowIdx: number, colIdx: number) => {
      const value = rows[rowIdx]?.[colIdx];
      const text = value === null ? "NULL" : String(value);
      navigator.clipboard.writeText(text);
      showCopied(`cell-${rowIdx}-${colIdx}`);
    },
    [rows, showCopied],
  );

  const copyRow = useCallback(
    (rowIdx: number) => {
      const row = rows[rowIdx];
      if (!row) return;
      const text = row.map((v) => (v === null ? "NULL" : String(v))).join("\t");
      navigator.clipboard.writeText(text);
      showCopied(`row-${rowIdx}`);
    },
    [rows, showCopied],
  );

  const [colWidths, setColWidths] = useState<number[]>(() =>
    columns.map(() => 150),
  );

  // Reset widths when columns change
  useEffect(() => {
    setColWidths(columns.map(() => 150));
  }, [columns]);

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
      </div>

      {/* Data grid */}
      <div ref={parentRef} className="flex-1 overflow-auto">
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
                    return (
                      <div
                        key={header.id}
                        onClick={header.column.getToggleSortingHandler()}
                        className="relative cursor-pointer select-none border-b border-r border-border px-2 py-1.5 text-left hover:bg-accent/50"
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
              return (
                <div
                  key={row.id}
                  data-index={virtualRow.index}
                  ref={(node) => virtualizer.measureElement(node)}
                  className={cn(
                    "absolute left-0 w-full grid",
                    virtualRow.index % 2 === 0 ? "bg-background" : "bg-muted/30",
                    "hover:bg-primary/5",
                  )}
                  style={{
                    transform: `translateY(${virtualRow.start}px)`,
                    gridTemplateColumns,
                  }}
                >
                  {/* Row number — click to copy entire row */}
                  <div
                    onClick={() => copyRow(row.index)}
                    className="relative cursor-pointer border-r border-border px-2 py-1 text-right text-[10px] text-muted-foreground/50 tabular-nums hover:bg-primary/10 hover:text-primary"
                    title="Click to copy row"
                  >
                    {copiedId === `row-${row.index}` ? (
                      <Check size={10} className="inline text-success" />
                    ) : (
                      virtualRow.index + 1
                    )}
                  </div>
                  {row.getVisibleCells().map((cell, cellIdx) => (
                    <div
                      key={cell.id}
                      onClick={() => copyCell(row.index, cellIdx)}
                      className="cursor-pointer truncate border-r border-border px-2 py-1 hover:bg-primary/5"
                      title="Click to copy cell"
                    >
                      {copiedId === `cell-${row.index}-${cellIdx}` ? (
                        <span className="text-[10px] text-success">Copied</span>
                      ) : (
                        flexRender(cell.column.columnDef.cell, cell.getContext())
                      )}
                    </div>
                  ))}
                </div>
              );
            })}
          </div>
        </div>
      </div>
    </div>
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
