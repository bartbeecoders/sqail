import { invoke } from "@tauri-apps/api/core";

export interface ValidationResult {
  ok: boolean;
  error?: string | null;
  /** Present when validation was skipped (DDL, unsupported driver, etc.). */
  note?: string | null;
}

/**
 * Ask the database to parse/prepare the SQL without executing it.
 *
 * Postgres: PREPARE / DEALLOCATE
 * MySQL:    PREPARE / DEALLOCATE PREPARE
 * SQLite:   EXPLAIN
 * MSSQL:    SET PARSEONLY ON / OFF
 *
 * DDL and multi-statement scripts may be skipped with `ok: true, note: ...`.
 * Requires an active connection — callers must guard with a connection check.
 */
export async function validateQuery(
  connectionId: string,
  sql: string,
): Promise<ValidationResult> {
  return invoke<ValidationResult>("validate_query", { connectionId, sql });
}
