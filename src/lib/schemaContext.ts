import { useSchemaStore } from "../stores/schemaStore";

export function buildSchemaContext(): string {
  const { schemas, tables, columns } = useSchemaStore.getState();
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
    }
  }

  return lines.join("\n");
}
