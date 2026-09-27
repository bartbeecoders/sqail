//! Schema snapshot: introspect a connected DB, emit per-object DDL files
//! under <repo>/schemas/<schema>/<kind>/<name>.sql.
//!
//! The DDL we emit is synthesised from information_schema / pg_catalog / sys.*
//! — it's intentionally normalised rather than a round-trip pg_dump. The point
//! is to produce diff-friendly files that a human (or the AI migration flow)
//! can reason about.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::db::connections::Driver;
use crate::pool::DbPool;
use crate::schema::{self, ColumnInfo, IndexInfo};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotSummary {
    pub files_written: usize,
    pub written: Vec<String>,
    pub removed: Vec<String>,
}

pub async fn snapshot_to_repo(
    repo_path: &Path,
    pool: DbPool,
    driver: &Driver,
    include_routines: bool,
) -> Result<SnapshotSummary, String> {
    let schemas_root = repo_path.join("schemas");
    // Gather previously-written files so we can drop stale ones.
    let previous = list_existing_sql_files(&schemas_root);

    let mut written: Vec<String> = Vec::new();
    let schemas = schema::list_schemas(pool.clone(), driver).await?;

    for sch in schemas {
        // Tables & views
        let tables = schema::list_tables(pool.clone(), driver, &sch.name).await?;
        for t in &tables {
            let columns = schema::list_columns(pool.clone(), driver, &sch.name, &t.name).await?;
            let fks = schema::list_foreign_keys(pool.clone(), driver, &sch.name)
                .await
                .unwrap_or_default();
            let indexes = schema::list_indexes(pool.clone(), driver, &sch.name, &t.name)
                .await
                .unwrap_or_default();

            let rel_dir = format!("schemas/{}/{}s", sanitize(&sch.name), t.table_type);
            let rel = format!("{}/{}.sql", rel_dir, sanitize(&t.name));
            let body = if t.table_type == "view" {
                render_view(pool.clone(), driver, &sch.name, &t.name).await
            } else {
                render_table(driver, &sch.name, &t.name, &columns, &indexes, &filter_fks(&fks, &t.name))
            };
            write_file(repo_path, &rel, &body)?;
            written.push(rel);
        }

        if include_routines {
            let routines = schema::list_routines(pool.clone(), driver, &sch.name)
                .await
                .unwrap_or_default();
            for r in routines {
                let rel = format!(
                    "schemas/{}/{}s/{}.sql",
                    sanitize(&sch.name),
                    r.routine_type,
                    sanitize(&r.name)
                );
                let body = schema::get_routine_definition(
                    pool.clone(),
                    driver,
                    &r.schema,
                    &r.name,
                    &r.routine_type,
                )
                .await
                .unwrap_or_else(|e| format!("-- failed to fetch definition: {e}\n"));
                write_file(repo_path, &rel, &body)?;
                written.push(rel);
            }
        }
    }

    // Remove stale files that were present from a prior snapshot but are no
    // longer generated (dropped tables/views).
    let written_set: std::collections::HashSet<&String> = written.iter().collect();
    let mut removed: Vec<String> = Vec::new();
    for prev in previous {
        if !written_set.contains(&prev) {
            let full = repo_path.join(&prev);
            let _ = std::fs::remove_file(&full);
            removed.push(prev);
        }
    }

    Ok(SnapshotSummary {
        files_written: written.len(),
        written,
        removed,
    })
}

fn list_existing_sql_files(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    if !root.exists() {
        return out;
    }
    walk(root, root, &mut out);
    out
}

fn walk(base: &Path, dir: &Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return; };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(base, &path, out);
        } else if path.extension().and_then(|s| s.to_str()) == Some("sql") {
            if let Ok(rel) = path.strip_prefix(base.parent().unwrap_or(base)) {
                // We want paths relative to repo_path, which is base.parent().
                // `schemas/<name>/<kind>/<file>.sql`.
                out.push(rel.to_string_lossy().to_string());
            }
        }
    }
}

fn write_file(repo_path: &Path, rel: &str, body: &str) -> Result<(), String> {
    let full: PathBuf = repo_path.join(rel);
    if let Some(parent) = full.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create dir: {e}"))?;
    }
    // Skip rewriting if content is byte-identical — keeps mtime stable and
    // keeps git diff clean across repeated snapshots with no DB changes.
    if let Ok(existing) = std::fs::read_to_string(&full) {
        if existing == body {
            return Ok(());
        }
    }
    std::fs::write(&full, body).map_err(|e| format!("write file: {e}"))
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            _ => c,
        })
        .collect()
}

fn filter_fks(
    fks: &[schema::ForeignKeyInfo],
    table: &str,
) -> Vec<schema::ForeignKeyInfo> {
    fks.iter()
        .filter(|f| f.source_table == table)
        .cloned()
        .collect()
}

async fn render_view(
    pool: DbPool,
    driver: &Driver,
    schema_name: &str,
    name: &str,
) -> String {
    match schema::get_view_definition(pool, driver, schema_name, name).await {
        Ok(def) => {
            let qualified = qualify(driver, schema_name, name);
            format!("CREATE OR REPLACE VIEW {qualified} AS\n{def}\n")
        }
        Err(e) => format!("-- failed to fetch view definition: {e}\n"),
    }
}

fn render_table(
    driver: &Driver,
    schema_name: &str,
    table: &str,
    columns: &[ColumnInfo],
    indexes: &[IndexInfo],
    fks: &[schema::ForeignKeyInfo],
) -> String {
    let qualified = qualify(driver, schema_name, table);
    let mut out = String::new();
    out.push_str(&format!("CREATE TABLE {qualified} (\n"));

    let mut rows: Vec<String> = Vec::new();
    for c in columns {
        let nullable = if c.is_nullable { "" } else { " NOT NULL" };
        let default = c
            .column_default
            .as_ref()
            .map(|d| format!(" DEFAULT {d}"))
            .unwrap_or_default();
        rows.push(format!(
            "    {} {}{}{}",
            quote_ident(driver, &c.name),
            c.data_type,
            nullable,
            default
        ));
    }
    // PK constraint
    let pk_cols: Vec<String> = columns
        .iter()
        .filter(|c| c.is_primary_key)
        .map(|c| quote_ident(driver, &c.name))
        .collect();
    if !pk_cols.is_empty() {
        rows.push(format!("    PRIMARY KEY ({})", pk_cols.join(", ")));
    }
    out.push_str(&rows.join(",\n"));
    out.push_str("\n);\n");

    // Indexes (skip ones that look like the primary key)
    for idx in indexes {
        if idx.columns == pk_cols
            || idx
                .columns
                .iter()
                .map(|c| quote_ident(driver, c))
                .collect::<Vec<_>>()
                == pk_cols
        {
            continue;
        }
        let unique = if idx.is_unique { "UNIQUE " } else { "" };
        let cols = idx
            .columns
            .iter()
            .map(|c| quote_ident(driver, c))
            .collect::<Vec<_>>()
            .join(", ");
        out.push_str(&format!(
            "CREATE {unique}INDEX {} ON {qualified} ({cols});\n",
            quote_ident(driver, &idx.name)
        ));
    }

    // Foreign keys as separate ALTER TABLE so each FK diffs in isolation.
    for fk in fks {
        let target = qualify(driver, &fk.target_schema, &fk.target_table);
        out.push_str(&format!(
            "ALTER TABLE {qualified} ADD CONSTRAINT {name} FOREIGN KEY ({src}) REFERENCES {target} ({tgt});\n",
            name = quote_ident(driver, &fk.constraint_name),
            src = quote_ident(driver, &fk.source_column),
            tgt = quote_ident(driver, &fk.target_column),
        ));
    }

    out
}

fn qualify(driver: &Driver, schema: &str, name: &str) -> String {
    match driver {
        Driver::Sqlite => quote_ident(driver, name),
        _ => format!(
            "{}.{}",
            quote_ident(driver, schema),
            quote_ident(driver, name)
        ),
    }
}

fn quote_ident(driver: &Driver, s: &str) -> String {
    match driver {
        Driver::Mysql => format!("`{}`", s.replace('`', "``")),
        Driver::Mssql => format!("[{}]", s.replace(']', "]]")),
        _ => format!("\"{}\"", s.replace('"', "\"\"")),
    }
}
