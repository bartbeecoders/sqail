import { useEffect, useState, useRef, useCallback } from "react";
import {
  ChevronRight,
  ChevronDown,
  Table2,
  Eye,
  Columns3,
  Key,
  RefreshCw,
  Loader2,
  Search,
  FolderOpen,
  Folder,
  Cog,
  AlertCircle,
  Sparkles,
  FileEdit,
  ClipboardCopy,
  Download,
} from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { cn } from "../lib/utils";
import { useSchemaStore } from "../stores/schemaStore";
import { useConnectionStore } from "../stores/connectionStore";
import { useEditorStore } from "../stores/editorStore";
import { useMetadataStore } from "../stores/metadataStore";
import type { TableInfo, ColumnInfo, RoutineInfo } from "../types/schema";
import type { Driver } from "../types/connection";
import type { ObjectMetadata } from "../types/metadata";
import MetadataDetailModal from "./MetadataDetailModal";
import { collectExportObjects, exportSchema, type SchemaExportFormat } from "../lib/exportSchema";

/** Quote an identifier for the given dialect. */
function quoteId(name: string, driver: Driver): string {
  switch (driver) {
    case "mssql":
      return `[${name}]`;
    case "mysql":
      return `\`${name}\``;
    default: // postgres, sqlite
      return `"${name}"`;
  }
}

/** Build a qualified table reference: schema.table (or just table for sqlite). */
function qualifiedTable(schema: string, table: string, driver: Driver): string {
  if (driver === "sqlite") {
    return quoteId(table, driver);
  }
  return `${quoteId(schema, driver)}.${quoteId(table, driver)}`;
}

/** Generate a CREATE TABLE statement from loaded column info. */
function generateCreateTable(
  schema: string,
  table: string,
  cols: ColumnInfo[] | undefined,
  driver: Driver,
): string {
  const ref = qualifiedTable(schema, table, driver);
  if (!cols || cols.length === 0) {
    return `-- Could not generate DDL: columns not loaded\n-- CREATE TABLE ${ref} (\n-- );\n`;
  }
  const pks = cols.filter((c) => c.isPrimaryKey).map((c) => quoteId(c.name, driver));
  const colLines = cols.map((c) => {
    let line = `  ${quoteId(c.name, driver)} ${c.dataType}`;
    if (!c.isNullable) line += " NOT NULL";
    if (c.columnDefault) line += ` DEFAULT ${c.columnDefault}`;
    return line;
  });
  if (pks.length > 0) {
    colLines.push(`  PRIMARY KEY (${pks.join(", ")})`);
  }
  return `CREATE TABLE ${ref} (\n${colLines.join(",\n")}\n);`;
}

/** Generate an ALTER TABLE template. */
function generateAlterTable(
  schema: string,
  table: string,
  driver: Driver,
): string {
  const ref = qualifiedTable(schema, table, driver);
  const col = quoteId("column_name", driver);
  return (
    `-- ALTER TABLE: modify ${ref}\n\n` +
    `-- Add a column\nALTER TABLE ${ref}\n  ADD ${col} VARCHAR(255);\n\n` +
    `-- Drop a column\n-- ALTER TABLE ${ref}\n--   DROP COLUMN ${col};\n\n` +
    `-- Rename a column\n` +
    (driver === "mssql"
      ? `-- EXEC sp_rename '${schema}.${table}.${col}', 'new_name', 'COLUMN';\n`
      : `-- ALTER TABLE ${ref}\n--   RENAME COLUMN ${col} TO ${quoteId("new_name", driver)};\n`)
  );
}

/** Generate dialect-specific SQL snippets for the context menu. */
function dialectSql(schema: string, table: string, tableType: "table" | "view", driver: Driver) {
  const ref = qualifiedTable(schema, table, driver);

  const selectTop =
    driver === "mssql"
      ? `SELECT TOP 100 * FROM ${ref};`
      : `SELECT * FROM ${ref}\nLIMIT 100;`;

  const selectCount = `SELECT COUNT(*) FROM ${ref};`;

  const columnInfo = (() => {
    switch (driver) {
      case "mssql":
        return (
          `-- Columns of ${ref}\n` +
          `SELECT COLUMN_NAME, DATA_TYPE, IS_NULLABLE, COLUMN_DEFAULT\n` +
          `FROM INFORMATION_SCHEMA.COLUMNS\n` +
          `WHERE TABLE_SCHEMA = '${schema}' AND TABLE_NAME = '${table}'\n` +
          `ORDER BY ORDINAL_POSITION;`
        );
      case "mysql":
        return `DESCRIBE ${ref};`;
      case "sqlite":
        return `PRAGMA table_info(${quoteId(table, driver)});`;
      default: // postgres
        return (
          `-- Columns of ${ref}\n` +
          `SELECT column_name, data_type, is_nullable, column_default\n` +
          `FROM information_schema.columns\n` +
          `WHERE table_schema = '${schema}' AND table_name = '${table}'\n` +
          `ORDER BY ordinal_position;`
        );
    }
  })();

  const dropLabel = tableType === "view" ? "DROP VIEW" : "DROP TABLE";
  const dropSql =
    `-- ⚠️ DANGER: This will drop the ${tableType}!\n` +
    `-- ${dropLabel} ${ref};`;

  return { selectTop, selectCount, columnInfo, dropLabel, dropSql };
}

type ContextMenuTarget =
  | { kind: "table"; schemaName: string; name: string; tableType: "table" | "view" }
  | { kind: "routine"; schemaName: string; name: string; routineType: "function" | "procedure" }
  | { kind: "column"; schemaName: string; tableName: string; name: string; dataType: string; isPrimaryKey: boolean };

interface ContextMenu {
  x: number;
  y: number;
  target: ContextMenuTarget;
}

export default function SchemaTree() {
  const activeConnectionId = useConnectionStore((s) => s.activeConnectionId);
  const connections = useConnectionStore((s) => s.connections);
  const activeConnection = connections.find((c) => c.id === activeConnectionId);
  const driver: Driver = activeConnection?.driver ?? "postgres";
  const {
    schemas,
    tables,
    columns,
    indexes,
    routines,
    loading,
    error,
    loadSchemas,
    loadTables,
    loadColumns,
    loadRoutines,
    loadAllColumns,
  } = useSchemaStore();
  const metadataError = useMetadataStore((s) => s.error);
  const getMetadataForObject = useMetadataStore((s) => s.getForObject);

  // expanded state: top-level categories, schema nodes within a category, individual tables
  const [expandedCategories, setExpandedCategories] = useState<Set<string>>(new Set());
  const [expandedSchemaNodes, setExpandedSchemaNodes] = useState<Set<string>>(new Set());
  const [expandedTables, setExpandedTables] = useState<Set<string>>(new Set());
  const [filter, setFilter] = useState("");
  const [contextMenu, setContextMenu] = useState<ContextMenu | null>(null);
  const contextRef = useRef<HTMLDivElement>(null);
  const [metadataModalEntry, setMetadataModalEntry] = useState<ObjectMetadata | null>(null);
  const [showExportMenu, setShowExportMenu] = useState(false);
  const exportRef = useRef<HTMLDivElement>(null);

  const loadMetadata = useMetadataStore((s) => s.loadMetadata);

  // Track which connection's schemas have been requested (to avoid duplicate fetches)
  const schemaRequestedFor = useRef<string | null>(null);
  const storeConnectionId = useSchemaStore((s) => s.connectionId);

  // Load schemas when the active connection changes
  /* eslint-disable react-hooks/set-state-in-effect */
  useEffect(() => {
    if (!activeConnectionId) return;
    if (schemaRequestedFor.current === activeConnectionId) return;

    schemaRequestedFor.current = activeConnectionId;
    setExpandedCategories(new Set());
    setExpandedSchemaNodes(new Set());
    setExpandedTables(new Set());
    loadSchemas(activeConnectionId);
    loadMetadata(activeConnectionId);
  }, [activeConnectionId, loadSchemas, loadMetadata]);

  // When schemas arrive for the current connection, eagerly fetch tables + routines
  const tablesLoadedFor = useRef<string | null>(null);
  useEffect(() => {
    if (
      schemas.length > 0 &&
      activeConnectionId &&
      storeConnectionId === activeConnectionId &&
      tablesLoadedFor.current !== activeConnectionId
    ) {
      tablesLoadedFor.current = activeConnectionId;
      for (const schema of schemas) {
        loadTables(activeConnectionId, schema.name).then(() => {
          loadAllColumns(activeConnectionId!, schema.name);
        });
        loadRoutines(activeConnectionId, schema.name);
      }
      setExpandedCategories(new Set(["tables"]));
      if (schemas.length === 1) {
        setExpandedSchemaNodes(new Set([`tables:${schemas[0].name}`]));
      }
    }
  }, [schemas, activeConnectionId, storeConnectionId, loadTables, loadRoutines, loadAllColumns]);
  /* eslint-enable react-hooks/set-state-in-effect */

  // Close context menu / export menu on click outside
  useEffect(() => {
    const handler = (e: MouseEvent) => {
      setContextMenu(null);
      if (exportRef.current && !exportRef.current.contains(e.target as Node)) {
        setShowExportMenu(false);
      }
    };
    document.addEventListener("click", handler);
    return () => document.removeEventListener("click", handler);
  }, []);

  const toggleCategory = (category: string) => {
    setExpandedCategories((prev) => {
      const next = new Set(prev);
      if (next.has(category)) next.delete(category);
      else next.add(category);
      return next;
    });
  };

  const toggleSchemaNode = (key: string) => {
    setExpandedSchemaNodes((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  };

  const toggleTable = (schemaName: string, tableName: string) => {
    const key = `${schemaName}.${tableName}`;
    setExpandedTables((prev) => {
      const next = new Set(prev);
      if (next.has(key)) {
        next.delete(key);
      } else {
        next.add(key);
        if (activeConnectionId && !columns[key]) {
          loadColumns(activeConnectionId, schemaName, tableName);
        }
      }
      return next;
    });
  };

  const insertIntoEditor = useCallback((sql: string) => {
    const state = useEditorStore.getState();
    const tab = state.getActiveTab();
    if (tab) {
      const newContent = tab.content ? `${tab.content}\n${sql}` : sql;
      state.setContent(tab.id, newContent);
    }
  }, []);

  const handleDoubleClick = (schemaName: string, tableName: string) => {
    const ref = qualifiedTable(schemaName, tableName, driver);
    const sql =
      driver === "mssql"
        ? `SELECT TOP 100 * FROM ${ref};`
        : `SELECT * FROM ${ref}\nLIMIT 100;`;
    insertIntoEditor(sql);
  };

  const openContextMenu = (e: React.MouseEvent, target: ContextMenuTarget) => {
    e.preventDefault();
    e.stopPropagation();
    setContextMenu({ x: e.clientX, y: e.clientY, target });
  };

  const contextAction = (sql: string) => {
    insertIntoEditor(sql);
    setContextMenu(null);
  };

  const contextActionNewTab = async (title: string, contentOrPromise: string | Promise<string>) => {
    setContextMenu(null);
    const store = useEditorStore.getState();
    store.addTab();
    const tab = store.getActiveTab();
    if (!tab) return;
    store.renameTab(tab.id, title);
    const content = typeof contentOrPromise === "string" ? contentOrPromise : await contentOrPromise;
    store.setContent(tab.id, content);
  };

  const copyToClipboard = (text: string) => {
    navigator.clipboard.writeText(text);
    setContextMenu(null);
  };

  const handleRefresh = () => {
    if (activeConnectionId) {
      schemaRequestedFor.current = null;
      tablesLoadedFor.current = null;
      loadSchemas(activeConnectionId);
    }
  };

  const handleShowMetadata = useCallback((schemaName: string, objectName: string) => {
    const entry = useMetadataStore.getState().getForObject(schemaName, objectName);
    if (entry) setMetadataModalEntry(entry);
  }, []);

  const filterLower = filter.toLowerCase();

  const getTablesOfType = (type: string): { schema: string; items: TableInfo[] }[] => {
    const result: { schema: string; items: TableInfo[] }[] = [];
    for (const schema of schemas) {
      const all = tables[schema.name];
      if (!all) continue;
      let items = all.filter((t) => t.tableType === type);
      const schemaMatches = filterLower && schema.name.toLowerCase().includes(filterLower);
      if (filterLower && !schemaMatches) {
        items = items.filter((t) => t.name.toLowerCase().includes(filterLower));
      }
      if (items.length > 0) {
        result.push({ schema: schema.name, items });
      }
    }
    return result;
  };

  const getRoutines = (): { schema: string; items: RoutineInfo[] }[] => {
    const result: { schema: string; items: RoutineInfo[] }[] = [];
    for (const schema of schemas) {
      const all = routines[schema.name];
      if (!all) continue;
      let items = all;
      const schemaMatches = filterLower && schema.name.toLowerCase().includes(filterLower);
      if (filterLower && !schemaMatches) {
        items = items.filter((r) => r.name.toLowerCase().includes(filterLower));
      }
      if (items.length > 0) {
        result.push({ schema: schema.name, items });
      }
    }
    return result;
  };

  const totalCount = (groups: { items: unknown[] }[]): number =>
    groups.reduce((sum, g) => sum + g.items.length, 0);

  const handleExport = async (format: SchemaExportFormat) => {
    setShowExportMenu(false);
    const tGroups = getTablesOfType("table");
    const vGroups = getTablesOfType("view");
    const rGroups = getRoutines();
    const objects = collectExportObjects(tGroups, vGroups, rGroups, columns, indexes, getMetadataForObject);
    const connName = activeConnection?.name ?? "database";
    try {
      await exportSchema(format, objects, `${connName}-schema`);
    } catch (e) {
      console.error("Schema export failed:", e);
    }
  };

  if (!activeConnectionId) {
    return (
      <div className="px-2 py-4 text-center text-[11px] text-muted-foreground opacity-60">
        Connect to a database to browse schema
      </div>
    );
  }

  const tableGroups = getTablesOfType("table");
  const viewGroups = getTablesOfType("view");
  const routineGroups = getRoutines();
  const hasMultipleSchemas = schemas.length > 1;

  return (
    <div className="flex flex-col min-h-0 h-full text-xs">
      {/* Header with refresh + export */}
      <div className="flex items-center justify-between px-2 pb-1 pt-2">
        <span className="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
          Database
        </span>
        <div className="flex items-center gap-0.5">
          <div ref={exportRef} className="relative">
            <button
              onClick={(e) => { e.stopPropagation(); setShowExportMenu((v) => !v); }}
              className="rounded p-0.5 text-muted-foreground hover:bg-accent hover:text-accent-foreground"
              title="Export schema"
            >
              <Download size={12} />
            </button>
            {showExportMenu && (
              <div className="absolute right-0 top-full z-50 mt-1 min-w-36 rounded-md border border-border bg-background py-1 shadow-lg text-xs">
                <button
                  onClick={() => handleExport("markdown")}
                  className="flex w-full items-center gap-2 px-3 py-1 text-left hover:bg-accent hover:text-accent-foreground"
                >
                  Export as Markdown
                </button>
                <button
                  onClick={() => handleExport("excel")}
                  className="flex w-full items-center gap-2 px-3 py-1 text-left hover:bg-accent hover:text-accent-foreground"
                >
                  Export as Excel
                </button>
              </div>
            )}
          </div>
          <button
            onClick={handleRefresh}
            disabled={loading}
            className="rounded p-0.5 text-muted-foreground hover:bg-accent hover:text-accent-foreground disabled:opacity-40"
            title="Refresh schema"
          >
            {loading ? <Loader2 size={12} className="animate-spin" /> : <RefreshCw size={12} />}
          </button>
        </div>
      </div>

      {/* Error display */}
      {error && (
        <div className="mx-2 mb-1 flex items-start gap-1.5 rounded border border-destructive/30 bg-destructive/10 px-2 py-1.5 text-[11px] text-destructive">
          <AlertCircle size={12} className="mt-0.5 shrink-0" />
          <span className="break-all">{error}</span>
        </div>
      )}

      {/* Metadata generation error */}
      {metadataError && (
        <div className="mx-2 mb-1 flex items-start gap-1.5 rounded border border-destructive/30 bg-destructive/10 px-2 py-1.5 text-[11px] text-destructive">
          <AlertCircle size={12} className="mt-0.5 shrink-0" />
          <div className="min-w-0 flex-1">
            <span className="break-all">{metadataError}</span>
            <button
              onClick={() => useMetadataStore.setState({ error: null })}
              className="ml-1 text-[10px] underline opacity-70 hover:opacity-100"
            >
              dismiss
            </button>
          </div>
        </div>
      )}

      {/* Filter */}
      <div className="relative px-2 pb-1">
        <Search size={10} className="absolute left-3.5 top-1/2 -translate-y-1/2 text-muted-foreground" />
        <input
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          placeholder="Filter..."
          className="input py-1 pl-6 text-[11px]"
        />
      </div>

      {/* Tree: Type → Schema → Items */}
      <div className="overflow-y-auto pl-1 pr-2.5">
        {/* ── Tables ── */}
        <CategoryNode
          label="Tables"
          icon={Table2}
          count={totalCount(tableGroups)}
          isExpanded={expandedCategories.has("tables")}
          onToggle={() => toggleCategory("tables")}
        >
          {tableGroups.length === 0 && (
            <EmptyMessage filter={filter} label="tables" />
          )}
          {tableGroups.map(({ schema, items }) => (
            <SchemaGroupNode
              key={schema}
              schemaName={schema}
              showSchema={hasMultipleSchemas}
              isExpanded={expandedSchemaNodes.has(`tables:${schema}`)}
              onToggle={() => toggleSchemaNode(`tables:${schema}`)}
              connectionId={activeConnectionId}
            >
              {items.map((table) => (
                <TableNode
                  key={table.name}
                  table={table}
                  schemaName={schema}
                  isExpanded={expandedTables.has(`${schema}.${table.name}`)}
                  columns={columns[`${schema}.${table.name}`]}
                  onToggle={() => toggleTable(schema, table.name)}
                  onDoubleClick={() => handleDoubleClick(schema, table.name)}
                  onContextMenu={(e) => openContextMenu(e, { kind: "table", schemaName: schema, name: table.name, tableType: table.tableType })}
                  onColumnContextMenu={(e, col) => openContextMenu(e, { kind: "column", schemaName: schema, tableName: table.name, name: col.name, dataType: col.dataType, isPrimaryKey: col.isPrimaryKey })}
                  connectionId={activeConnectionId}
                  onShowMetadata={handleShowMetadata}
                />
              ))}
            </SchemaGroupNode>
          ))}
        </CategoryNode>

        {/* ── Views ── */}
        <CategoryNode
          label="Views"
          icon={Eye}
          count={totalCount(viewGroups)}
          isExpanded={expandedCategories.has("views")}
          onToggle={() => toggleCategory("views")}
        >
          {viewGroups.length === 0 && (
            <EmptyMessage filter={filter} label="views" />
          )}
          {viewGroups.map(({ schema, items }) => (
            <SchemaGroupNode
              key={schema}
              schemaName={schema}
              showSchema={hasMultipleSchemas}
              isExpanded={expandedSchemaNodes.has(`views:${schema}`)}
              onToggle={() => toggleSchemaNode(`views:${schema}`)}
              connectionId={activeConnectionId}
            >
              {items.map((view) => (
                <TableNode
                  key={view.name}
                  table={view}
                  schemaName={schema}
                  isExpanded={expandedTables.has(`${schema}.${view.name}`)}
                  columns={columns[`${schema}.${view.name}`]}
                  onToggle={() => toggleTable(schema, view.name)}
                  onDoubleClick={() => handleDoubleClick(schema, view.name)}
                  onContextMenu={(e) => openContextMenu(e, { kind: "table", schemaName: schema, name: view.name, tableType: view.tableType })}
                  onColumnContextMenu={(e, col) => openContextMenu(e, { kind: "column", schemaName: schema, tableName: view.name, name: col.name, dataType: col.dataType, isPrimaryKey: col.isPrimaryKey })}
                  connectionId={activeConnectionId}
                  onShowMetadata={handleShowMetadata}
                />
              ))}
            </SchemaGroupNode>
          ))}
        </CategoryNode>

        {/* ── Procedures ── */}
        <CategoryNode
          label="Procedures"
          icon={Cog}
          count={totalCount(routineGroups)}
          isExpanded={expandedCategories.has("procedures")}
          onToggle={() => toggleCategory("procedures")}
        >
          {routineGroups.length === 0 && (
            <EmptyMessage filter={filter} label="procedures" />
          )}
          {routineGroups.map(({ schema, items }) => (
            <SchemaGroupNode
              key={schema}
              schemaName={schema}
              showSchema={hasMultipleSchemas}
              isExpanded={expandedSchemaNodes.has(`procedures:${schema}`)}
              onToggle={() => toggleSchemaNode(`procedures:${schema}`)}
              connectionId={activeConnectionId}
            >
              {items.map((routine) => (
                <RoutineNode
                  key={routine.name}
                  routine={routine}
                  onContextMenu={(e) => openContextMenu(e, {
                    kind: "routine",
                    schemaName: routine.schema,
                    name: routine.name,
                    routineType: routine.routineType,
                  })}
                />
              ))}
            </SchemaGroupNode>
          ))}
        </CategoryNode>
      </div>

      {/* Unified context menu */}
      {contextMenu && (
        <div
          ref={contextRef}
          className="fixed z-50 min-w-48 rounded-md border border-border bg-background py-1 shadow-lg text-xs"
          style={{ left: contextMenu.x, top: contextMenu.y }}
        >
          {contextMenu.target.kind === "table" && (() => {
            const { schemaName, name, tableType } = contextMenu.target;
            const sql = dialectSql(schemaName, name, tableType, driver);
            const ref = qualifiedTable(schemaName, name, driver);
            const colKey = `${schemaName}.${name}`;
            return (
              <>
                <ContextItem
                  label={driver === "mssql" ? "SELECT TOP 100 *" : "SELECT * LIMIT 100"}
                  onClick={() => contextAction(sql.selectTop)}
                />
                <ContextItem
                  label="SELECT COUNT(*)"
                  onClick={() => contextAction(sql.selectCount)}
                />
                <ContextItem
                  label={driver === "mysql" ? "DESCRIBE" : driver === "sqlite" ? "PRAGMA table_info" : "Column Info"}
                  onClick={() => contextAction(sql.columnInfo)}
                />
                <ContextSeparator />
                {tableType === "table" ? (
                  <>
                    <ContextItem
                      label="CREATE TABLE"
                      icon={FileEdit}
                      onClick={() => contextActionNewTab(
                        `CREATE ${name}`,
                        generateCreateTable(schemaName, name, columns[colKey], driver),
                      )}
                    />
                    <ContextItem
                      label="ALTER TABLE"
                      icon={FileEdit}
                      onClick={() => contextActionNewTab(
                        `ALTER ${name}`,
                        generateAlterTable(schemaName, name, driver),
                      )}
                    />
                  </>
                ) : (
                  <ContextItem
                    label="View Definition"
                    icon={FileEdit}
                    onClick={() => {
                      if (!activeConnectionId) return;
                      contextActionNewTab(
                        name,
                        invoke<string>("get_view_definition", {
                          connectionId: activeConnectionId,
                          schemaName,
                          viewName: name,
                        }),
                      );
                    }}
                  />
                )}
                <ContextSeparator />
                <ContextItem
                  label="Copy Name"
                  icon={ClipboardCopy}
                  onClick={() => copyToClipboard(ref)}
                />
                <ContextItem
                  label="Insert Name"
                  onClick={() => contextAction(ref)}
                />
                <ContextSeparator />
                <ContextItem
                  label="Generate Metadata"
                  icon={Sparkles}
                  onClick={() => {
                    if (activeConnectionId) {
                      useMetadataStore.getState().generateSingle(
                        activeConnectionId,
                        schemaName,
                        name,
                        tableType,
                      );
                    }
                    setContextMenu(null);
                  }}
                />
                <ContextSeparator />
                <ContextItem
                  label={sql.dropLabel}
                  className="text-destructive"
                  onClick={() => contextAction(sql.dropSql)}
                />
              </>
            );
          })()}

          {contextMenu.target.kind === "routine" && (() => {
            const { schemaName, name, routineType } = contextMenu.target;
            const ref = qualifiedTable(schemaName, name, driver);
            const typeLabel = routineType === "procedure" ? "Procedure" : "Function";

            const execSql = (() => {
              if (routineType === "procedure") {
                switch (driver) {
                  case "mssql":
                    return `EXEC ${ref};`;
                  case "mysql":
                    return `CALL ${ref}();`;
                  default:
                    return `CALL ${ref}();`;
                }
              }
              // function
              return `SELECT ${ref}();`;
            })();

            const dropSql =
              `-- ⚠️ DANGER: This will drop the ${routineType}!\n` +
              `-- DROP ${routineType.toUpperCase()} ${ref};`;

            return (
              <>
                <ContextItem
                  label="Open Definition"
                  icon={FileEdit}
                  onClick={() => {
                    if (!activeConnectionId) return;
                    contextActionNewTab(
                      name,
                      invoke<string>("get_routine_definition", {
                        connectionId: activeConnectionId,
                        schemaName,
                        routineName: name,
                        routineType,
                      }),
                    );
                  }}
                />
                <ContextItem
                  label={routineType === "procedure" ? (driver === "mssql" ? "EXEC" : "CALL") : "SELECT (execute)"}
                  onClick={() => contextAction(execSql)}
                />
                <ContextSeparator />
                <ContextItem
                  label="Copy Name"
                  icon={ClipboardCopy}
                  onClick={() => copyToClipboard(ref)}
                />
                <ContextItem
                  label="Insert Name"
                  onClick={() => contextAction(ref)}
                />
                <ContextSeparator />
                <ContextItem
                  label="Generate Metadata"
                  icon={Sparkles}
                  onClick={() => {
                    if (activeConnectionId) {
                      useMetadataStore.getState().generateSingle(
                        activeConnectionId,
                        schemaName,
                        name,
                        routineType,
                      );
                    }
                    setContextMenu(null);
                  }}
                />
                <ContextSeparator />
                <ContextItem
                  label={`DROP ${typeLabel.toUpperCase()}`}
                  className="text-destructive"
                  onClick={() => contextAction(dropSql)}
                />
              </>
            );
          })()}

          {contextMenu.target.kind === "column" && (() => {
            const { schemaName, tableName, name, dataType, isPrimaryKey } = contextMenu.target;
            const tableRef = qualifiedTable(schemaName, tableName, driver);
            const colRef = quoteId(name, driver);

            const selectCol =
              driver === "mssql"
                ? `SELECT TOP 100 ${colRef} FROM ${tableRef};`
                : `SELECT ${colRef} FROM ${tableRef}\nLIMIT 100;`;

            const selectDistinct =
              driver === "mssql"
                ? `SELECT DISTINCT TOP 100 ${colRef} FROM ${tableRef};`
                : `SELECT DISTINCT ${colRef} FROM ${tableRef}\nLIMIT 100;`;

            const whereClause = `SELECT * FROM ${tableRef}\nWHERE ${colRef} = ;`;

            const countDistinct = `SELECT COUNT(DISTINCT ${colRef}) FROM ${tableRef};`;

            return (
              <>
                <ContextItem
                  label="SELECT Column"
                  onClick={() => contextAction(selectCol)}
                />
                <ContextItem
                  label="SELECT DISTINCT"
                  onClick={() => contextAction(selectDistinct)}
                />
                <ContextItem
                  label="COUNT DISTINCT"
                  onClick={() => contextAction(countDistinct)}
                />
                <ContextItem
                  label="WHERE Clause"
                  onClick={() => contextAction(whereClause)}
                />
                <ContextSeparator />
                <ContextItem
                  label="Copy Name"
                  icon={ClipboardCopy}
                  onClick={() => copyToClipboard(name)}
                />
                <ContextItem
                  label="Insert Name"
                  onClick={() => contextAction(colRef)}
                />
                <ContextSeparator />
                <ContextItem
                  label={`Type: ${dataType}${isPrimaryKey ? " (PK)" : ""}`}
                  className="text-muted-foreground cursor-default"
                  onClick={() => setContextMenu(null)}
                />
              </>
            );
          })()}
        </div>
      )}

      {metadataModalEntry && activeConnectionId && (
        <MetadataDetailModal
          entry={metadataModalEntry}
          connectionId={activeConnectionId}
          onClose={() => setMetadataModalEntry(null)}
        />
      )}
    </div>
  );
}

/* ── Category (top-level: Tables / Views / Procedures) ── */

function CategoryNode({
  label,
  icon: Icon,
  count,
  isExpanded,
  onToggle,
  children,
}: {
  label: string;
  icon: React.ComponentType<{ size?: number; className?: string }>;
  count: number;
  isExpanded: boolean;
  onToggle: () => void;
  children: React.ReactNode;
}) {
  return (
    <div>
      <button
        onClick={onToggle}
        className="flex w-full items-center gap-1 rounded px-1 py-0.5 font-medium text-muted-foreground hover:bg-accent hover:text-accent-foreground"
      >
        {isExpanded ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
        <Icon size={12} className="shrink-0 opacity-60" />
        <span>{label}</span>
        <span className="ml-auto text-[9px] font-normal opacity-40">{count}</span>
      </button>
      {isExpanded && <div className="ml-3">{children}</div>}
    </div>
  );
}

/* ── Schema group (second level, only shown when multiple schemas) ── */

function SchemaGroupNode({
  schemaName,
  showSchema,
  isExpanded,
  onToggle,
  children,
  connectionId,
}: {
  schemaName: string;
  showSchema: boolean;
  isExpanded: boolean;
  onToggle: () => void;
  children: React.ReactNode;
  connectionId: string | null;
}) {
  const isSchemaGenerating = useMetadataStore((s) => s.isGenerating(schemaName));

  const handleGenerateSchema = (e: React.MouseEvent) => {
    e.stopPropagation();
    if (connectionId) {
      useMetadataStore.getState().generateSchema(connectionId, schemaName);
    }
  };

  if (!showSchema) {
    // Single schema — skip the schema grouping level, render children directly
    return <>{children}</>;
  }

  const FolderIcon = isExpanded ? FolderOpen : Folder;

  return (
    <div>
      <div className="group flex w-full items-center rounded px-1 py-0.5 text-muted-foreground hover:bg-accent hover:text-accent-foreground">
        <button
          onClick={onToggle}
          className="flex flex-1 items-center gap-1 min-w-0"
        >
          {isExpanded ? <ChevronDown size={10} /> : <ChevronRight size={10} />}
          <FolderIcon size={11} className="shrink-0 opacity-60" />
          <span className="truncate">{schemaName}</span>
        </button>
        {isSchemaGenerating ? (
          <Loader2 size={11} className="shrink-0 animate-spin text-muted-foreground" />
        ) : (
          <button
            onClick={handleGenerateSchema}
            className="shrink-0 rounded p-0.5 text-muted-foreground/40 hover:text-muted-foreground"
            title={`Generate metadata for all objects in ${schemaName}`}
          >
            <Sparkles size={11} />
          </button>
        )}
      </div>
      {isExpanded && <div className="ml-3">{children}</div>}
    </div>
  );
}

/* ── Table / View node ── */

function TableNode({
  table,
  schemaName,
  isExpanded,
  columns,
  onToggle,
  onDoubleClick,
  onContextMenu,
  onColumnContextMenu,
  connectionId,
  onShowMetadata,
}: {
  table: TableInfo;
  schemaName: string;
  isExpanded: boolean;
  columns?: ColumnInfo[];
  onToggle: () => void;
  onDoubleClick: () => void;
  onContextMenu: (e: React.MouseEvent) => void;
  onColumnContextMenu: (e: React.MouseEvent, col: ColumnInfo) => void;
  connectionId: string | null;
  onShowMetadata: (schemaName: string, objectName: string) => void;
}) {
  const Icon = table.tableType === "view" ? Eye : Table2;
  const isObjectGenerating = useMetadataStore((s) => s.isGenerating(schemaName, table.name));
  const hasMetadata = useMetadataStore((s) => !!s.getForObject(schemaName, table.name));

  const handleGenerateMetadata = (e: React.MouseEvent) => {
    e.stopPropagation();
    if (hasMetadata) {
      onShowMetadata(schemaName, table.name);
    } else if (connectionId) {
      useMetadataStore.getState().generateSingle(
        connectionId,
        schemaName,
        table.name,
        table.tableType,
      );
    }
  };

  const handleDragStart = (e: React.DragEvent) => {
    e.dataTransfer.setData(
      "application/sqlai-table",
      JSON.stringify({ schemaName, tableName: table.name }),
    );
    e.dataTransfer.effectAllowed = "copy";
  };

  return (
    <div>
      <div className="group flex w-full items-center rounded px-1 py-0.5 text-muted-foreground hover:bg-accent hover:text-accent-foreground">
        <button
          draggable
          onDragStart={handleDragStart}
          onClick={onToggle}
          onDoubleClick={onDoubleClick}
          onContextMenu={onContextMenu}
          className="flex flex-1 items-center gap-1 min-w-0 cursor-grab active:cursor-grabbing"
          title={`${schemaName}.${table.name} (${table.tableType})`}
        >
          {isExpanded ? <ChevronDown size={10} /> : <ChevronRight size={10} />}
          <Icon size={11} className="shrink-0 opacity-60" />
          <span className="truncate">{table.name}</span>
        </button>
        {isObjectGenerating ? (
          <Loader2 size={11} className="shrink-0 animate-spin text-muted-foreground" />
        ) : (
          <button
            onClick={handleGenerateMetadata}
            className={cn(
              "shrink-0 rounded p-0.5",
              hasMetadata
                ? "text-emerald-500 hover:text-emerald-400"
                : "text-muted-foreground/40 hover:text-muted-foreground",
            )}
            title={hasMetadata ? `View metadata for ${table.name}` : `Generate metadata for ${table.name}`}
          >
            <Sparkles size={11} />
          </button>
        )}
      </div>

      {isExpanded && columns && (
        <div className="ml-5">
          {columns.map((col) => (
            <ColumnNode
              key={col.name}
              column={col}
              onContextMenu={(e) => onColumnContextMenu(e, col)}
            />
          ))}
        </div>
      )}
    </div>
  );
}

/* ── Routine node ── */

function RoutineNode({
  routine,
  onContextMenu,
}: {
  routine: RoutineInfo;
  onContextMenu: (e: React.MouseEvent) => void;
}) {
  return (
    <div
      className="flex items-center gap-1 rounded px-1 py-0.5 text-muted-foreground hover:bg-accent hover:text-accent-foreground"
      title={`${routine.schema}.${routine.name} (${routine.routineType})`}
      onContextMenu={onContextMenu}
    >
      <Cog size={11} className="shrink-0 opacity-60" />
      <span className="truncate">{routine.name}</span>
      <span className="ml-auto shrink-0 text-[9px] opacity-40">{routine.routineType}</span>
    </div>
  );
}

/* ── Column node ── */

function ColumnNode({ column, onContextMenu }: { column: ColumnInfo; onContextMenu: (e: React.MouseEvent) => void }) {
  return (
    <div
      className="flex items-center gap-1 rounded px-1 py-px text-[11px] text-muted-foreground hover:bg-accent hover:text-accent-foreground"
      title={`${column.name} ${column.dataType}${column.isNullable ? "" : " NOT NULL"}${column.columnDefault ? ` DEFAULT ${column.columnDefault}` : ""}`}
      onContextMenu={onContextMenu}
    >
      {column.isPrimaryKey ? (
        <Key size={10} className="shrink-0 text-amber-500" />
      ) : (
        <Columns3 size={10} className="shrink-0 opacity-40" />
      )}
      <span className={cn("truncate", column.isPrimaryKey && "font-medium")}>{column.name}</span>
      <span className="ml-auto shrink-0 text-[9px] opacity-40">{column.dataType}</span>
    </div>
  );
}

/* ── Empty state ── */

function EmptyMessage({ filter, label }: { filter: string; label: string }) {
  return (
    <div className="px-2 py-1 text-[10px] text-muted-foreground/50">
      {filter ? "No matches" : `No ${label}`}
    </div>
  );
}

/* ── Context menu helpers ── */

function ContextSeparator() {
  return <div className="my-1 border-t border-border" />;
}

function ContextItem({
  label,
  onClick,
  className,
  icon: Icon,
}: {
  label: string;
  onClick: () => void;
  className?: string;
  icon?: React.ComponentType<{ size?: number; className?: string }>;
}) {
  return (
    <button
      onClick={onClick}
      className={cn(
        "flex w-full items-center gap-2 px-3 py-1 text-left text-xs hover:bg-accent hover:text-accent-foreground",
        className,
      )}
    >
      {Icon && <Icon size={12} className="shrink-0 opacity-60" />}
      {label}
    </button>
  );
}
