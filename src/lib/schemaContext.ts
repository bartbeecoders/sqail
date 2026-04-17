import { useSchemaStore } from "../stores/schemaStore";
import { useMetadataStore } from "../stores/metadataStore";
import { useConnectionStore } from "../stores/connectionStore";

/**
 * Build a schema-context block for the AI flows.
 *
 * This is `async` so that tables whose columns haven't been cached yet
 * (because the user hasn't expanded their row in the sidebar) still end
 * up in the context. Without this lazy load the model sees a list of
 * table names with no column lists and falls back to hallucinating the
 * columns it "expects" that kind of table to have.
 */
export async function buildSchemaContext(): Promise<string> {
  const schemaStore = useSchemaStore.getState();
  const { schemas, tables } = schemaStore;
  const metadataEntries = useMetadataStore.getState().entries;
  const connectionId = useConnectionStore.getState().activeConnectionId;
  if (schemas.length === 0) return "";

  // Lazily load columns for every table in every schema. `loadAllColumns`
  // skips tables whose columns are already cached, so this is cheap on
  // repeat calls.
  if (connectionId) {
    await Promise.allSettled(
      schemas.map(async (s) => {
        if (!tables[s.name]) {
          await schemaStore.loadTables(connectionId, s.name);
        }
        await useSchemaStore
          .getState()
          .loadAllColumns(connectionId, s.name);
      }),
    );
  }

  const { tables: refreshedTables, columns } = useSchemaStore.getState();

  const lines: string[] = [];

  for (const schema of schemas) {
    lines.push(`Schema: ${schema.name}`);
    const schemaTables = refreshedTables[schema.name] ?? [];
    for (const table of schemaTables) {
      const key = `${schema.name}.${table.name}`;
      const cols = columns[key] ?? [];
      if (cols.length > 0) {
        const colDefs = cols
          .map((c) => {
            let def = `${c.name} ${c.dataType}`;
            if (c.isPrimaryKey) def += " PK";
            if (!c.isNullable) def += " NOT NULL";
            return def;
          })
          .join(", ");
        lines.push(`  ${table.tableType === "view" ? "View" : "Table"}: ${table.name} (${colDefs})`);
      } else {
        lines.push(`  ${table.tableType === "view" ? "View" : "Table"}: ${table.name}`);
      }

      // Enrich with metadata description if available
      const meta = metadataEntries.find(
        (m) => m.schemaName === schema.name && m.objectName === table.name,
      );
      if (meta) {
        lines.push(`    -- ${meta.metadata.description}`);
      }
    }
  }

  return lines.join("\n");
}
