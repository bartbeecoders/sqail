import type { Driver } from "../types/connection";
import type { ColumnInfo } from "../types/schema";

/**
 * Quote an identifier according to the database driver's convention.
 *
 *  - mssql:    [identifier]
 *  - mysql:    `identifier`
 *  - postgres: "identifier"
 *  - sqlite:   "identifier"
 */
function quoteIdent(name: string, driver: Driver): string {
  switch (driver) {
    case "mssql":
    case "dbservice":
      return `[${name.replace(/]/g, "]]")}]`;
    case "mysql":
      return `\`${name.replace(/`/g, "``")}\``;
    case "postgres":
    case "sqlite":
      return `"${name.replace(/"/g, '""')}"`;
  }
}

/** Quote a string alias according to the driver's convention. */
function quoteAlias(name: string, driver: Driver): string {
  switch (driver) {
    case "mssql":
    case "dbservice":
    case "mysql":
      // Single-quoted alias: 'Column Name'
      return `'${name.replace(/'/g, "''")}'`;
    case "postgres":
    case "sqlite":
      // Double-quoted alias
      return `"${name.replace(/"/g, '""')}"`;
  }
}

/** Convert snake_case / camelCase column name to a readable alias label. */
function toAliasLabel(name: string): string {
  return name
    // insert space before capitals in camelCase
    .replace(/([a-z])([A-Z])/g, "$1 $2")
    // replace underscores / hyphens with space
    .replace(/[_-]/g, " ")
    // title-case each word
    .replace(/\b\w/g, (c) => c.toUpperCase());
}

/**
 * Build a formatted SELECT statement with all columns aliased.
 *
 * Example (mssql):
 *   SELECT
 *       [first_name] AS 'First Name',
 *       [last_name] AS 'Last Name'
 *   FROM [dbo].[users]
 */
export function buildSelectStatement(
  schemaName: string,
  tableName: string,
  cols: ColumnInfo[],
  driver: Driver,
): string {
  const qualifiedTable = `${quoteIdent(schemaName, driver)}.${quoteIdent(tableName, driver)}`;

  if (cols.length === 0) {
    return `SELECT *\nFROM ${qualifiedTable}`;
  }

  const lines = cols.map((col, i) => {
    const ident = quoteIdent(col.name, driver);
    const alias = quoteAlias(toAliasLabel(col.name), driver);
    const comma = i < cols.length - 1 ? "," : "";
    return `    ${ident} AS ${alias}${comma}`;
  });

  return `SELECT\n${lines.join("\n")}\nFROM ${qualifiedTable}`;
}
