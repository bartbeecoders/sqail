import { useSchemaStore } from "../stores/schemaStore";
import { useMetadataStore } from "../stores/metadataStore";

export function buildSchemaContext(): string {
  const { schemas, tables, columns } = useSchemaStore.getState();
  const metadataEntries = useMetadataStore.getState().entries;
  if (schemas.length === 0) return "";

  const lines: string[] = [];

  for (const schema of schemas) {
    lines.push(`Schema: ${schema.name}`);
    const schemaTables = tables[schema.name] ?? [];
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
