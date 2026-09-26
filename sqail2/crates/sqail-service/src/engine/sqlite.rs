//! SQLite via rusqlite (bundled), run on a blocking thread.
//!
//! Profiles may only open files inside the service's allowed directories.
//! Cancellation uses SQLite's interrupt handle.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use rusqlite::fallible_iterator::FallibleIterator;
use rusqlite::types::ValueRef;
use rusqlite::{Batch, InterruptHandle, OpenFlags};
use serde_json::Value;
use sqail_proto::{Column, Engine, LogicalType, Param, QueryEvent, SqliteParams};

use super::value::{bytes, float, int, text};
use super::{Cancel, Canceller, Conn, DbError, Driver, EventSink, ExecRequest, ROW_BATCH, Result};

pub struct SqliteDriver {
    path: PathBuf,
    read_only: bool,
    create: bool,
}

impl SqliteDriver {
    pub fn new(p: &SqliteParams, read_only: bool, allowed_dirs: &[PathBuf]) -> Result<Self> {
        let path = check_path(Path::new(&p.path), allowed_dirs)?;
        Ok(Self {
            path,
            read_only,
            create: p.create,
        })
    }
}

/// Resolve `path` and require it to be inside one of `allowed`.
pub fn check_path(path: &Path, allowed: &[PathBuf]) -> Result<PathBuf> {
    if allowed.is_empty() {
        return Err(DbError::Invalid(
            "SQLite is disabled on this service (no sqlite.allowed_dirs configured)".into(),
        ));
    }
    if !path.is_absolute() {
        return Err(DbError::Invalid("SQLite path must be absolute".into()));
    }
    let file_name = path
        .file_name()
        .ok_or_else(|| DbError::Invalid("SQLite path has no file name".into()))?;
    // Canonicalize the parent (the file itself may not exist yet), which
    // resolves `..` and symlinks before the prefix check.
    let parent = path
        .parent()
        .and_then(|p| p.canonicalize().ok())
        .ok_or_else(|| {
            DbError::Invalid(format!("directory of {} does not exist", path.display()))
        })?;
    let resolved = parent.join(file_name);
    let resolved = resolved.canonicalize().unwrap_or(resolved);
    let ok = allowed
        .iter()
        .filter_map(|d| d.canonicalize().ok())
        .any(|d| resolved.starts_with(&d));
    if ok {
        Ok(resolved)
    } else {
        Err(DbError::Invalid(format!(
            "{} is outside the service's allowed SQLite directories",
            path.display()
        )))
    }
}

#[async_trait]
impl Driver for SqliteDriver {
    async fn connect(&self) -> Result<Box<dyn Conn>> {
        let (path, read_only, create) = (self.path.clone(), self.read_only, self.create);
        tokio::task::spawn_blocking(move || {
            let mut flags = OpenFlags::SQLITE_OPEN_URI | OpenFlags::SQLITE_OPEN_NO_MUTEX;
            flags |= if read_only {
                OpenFlags::SQLITE_OPEN_READ_ONLY
            } else if create {
                OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE
            } else {
                OpenFlags::SQLITE_OPEN_READ_WRITE
            };
            let db = rusqlite::Connection::open_with_flags(&path, flags)
                .map_err(|e| DbError::Connect(format!("{}: {e}", path.display())))?;
            db.busy_timeout(std::time::Duration::from_secs(5))
                .map_err(db_err)?;
            db.pragma_update(None, "foreign_keys", "ON")
                .map_err(db_err)?;
            let interrupt = Arc::new(SqliteCancel(db.get_interrupt_handle()));
            Ok(Box::new(SqliteConn {
                db: Arc::new(Mutex::new(db)),
                interrupt,
            }) as Box<dyn Conn>)
        })
        .await
        .map_err(|e| DbError::Internal(e.to_string()))?
    }

    fn engine(&self) -> Engine {
        Engine::Sqlite
    }
}

struct SqliteConn {
    db: Arc<Mutex<rusqlite::Connection>>,
    interrupt: Arc<SqliteCancel>,
}

struct SqliteCancel(InterruptHandle);

#[async_trait]
impl Cancel for SqliteCancel {
    async fn cancel(&self) -> bool {
        self.0.interrupt();
        true
    }
}

impl SqliteConn {
    async fn blocking<T: Send + 'static>(
        &self,
        f: impl FnOnce(&rusqlite::Connection) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            let db = db.lock().unwrap_or_else(|e| e.into_inner());
            f(&db)
        })
        .await
        .map_err(|e| DbError::Internal(e.to_string()))?
    }
}

#[async_trait]
impl Conn for SqliteConn {
    async fn execute(&mut self, req: &ExecRequest, sink: &EventSink) -> Result<()> {
        // The blocking thread needs owned data and a sink it can use; bridge
        // through a channel and forward on this task.
        let (tx, mut rx) = tokio::sync::mpsc::channel::<QueryEvent>(16);
        let sql = req.sql.clone();
        let params = req.params.clone();
        let max_rows = req.max_rows;
        let db = self.db.clone();
        let worker = tokio::task::spawn_blocking(move || {
            let db = db.lock().unwrap_or_else(|e| e.into_inner());
            run(&db, &sql, &params, max_rows, &EventSink::Channel(tx))
        });
        let mut forward_err = None;
        while let Some(ev) = rx.recv().await {
            if let Err(e) = sink.send(ev).await {
                // Client gone: stop the worker, keep draining so it can exit.
                self.interrupt.0.interrupt();
                forward_err = Some(e);
                break;
            }
        }
        drop(rx);
        let res = worker.await.map_err(|e| DbError::Internal(e.to_string()))?;
        match forward_err {
            Some(e) => Err(e),
            None => res,
        }
    }

    fn canceller(&self) -> Canceller {
        self.interrupt.clone()
    }

    async fn in_transaction(&mut self) -> Result<bool> {
        self.blocking(|db| Ok(!db.is_autocommit())).await
    }

    async fn ping(&mut self) -> Result<()> {
        Ok(())
    }

    async fn server_version(&mut self) -> Result<String> {
        Ok(format!("SQLite {}", rusqlite::version()))
    }

    fn engine(&self) -> Engine {
        Engine::Sqlite
    }
}

/// Runs on the blocking thread.
fn run(
    db: &rusqlite::Connection,
    sql: &str,
    params: &[Param],
    max_rows: u64,
    sink: &EventSink,
) -> Result<()> {
    let mut index = 0u32;
    if params.is_empty() {
        let mut batch = Batch::new(db, sql);
        while let Some(mut stmt) = batch.next().map_err(db_err)? {
            run_stmt(db, &mut stmt, &[], max_rows, &mut index, sink)?;
        }
    } else {
        let mut stmt = db.prepare(sql).map_err(db_err)?;
        run_stmt(db, &mut stmt, params, max_rows, &mut index, sink)?;
    }
    Ok(())
}

fn run_stmt(
    db: &rusqlite::Connection,
    stmt: &mut rusqlite::Statement<'_>,
    params: &[Param],
    max_rows: u64,
    index: &mut u32,
    sink: &EventSink,
) -> Result<()> {
    let values: Vec<rusqlite::types::Value> = params.iter().map(to_sqlite).collect();
    let params = rusqlite::params_from_iter(values.iter());
    if stmt.column_count() == 0 {
        // `changes()` is not reset by statements that change nothing (DDL,
        // BEGIN/ROLLBACK), so measure this statement's own contribution.
        let before = db.total_changes();
        stmt.execute(params).map_err(db_err)?;
        return sink.send_blocking(QueryEvent::RowsAffected {
            count: db.total_changes().saturating_sub(before),
        });
    }
    let columns: Vec<Column> = stmt
        .columns()
        .iter()
        .map(|c| {
            let decl = c.decl_type().unwrap_or("");
            Column {
                name: c.name().to_string(),
                type_name: decl.to_string(),
                logical: logical(decl),
            }
        })
        .collect();
    let logicals: Vec<LogicalType> = columns.iter().map(|c| c.logical).collect();
    let my_index = *index;
    *index += 1;
    sink.send_blocking(QueryEvent::ResultStart {
        index: my_index,
        columns,
    })?;
    let mut rows = stmt.query(params).map_err(db_err)?;
    let mut batch = Vec::with_capacity(ROW_BATCH);
    let mut count = 0u64;
    let mut truncated = false;
    while let Some(row) = rows.next().map_err(db_err)? {
        if count >= max_rows {
            truncated = true;
            break; // SQLite can simply stop stepping.
        }
        count += 1;
        let cells = logicals
            .iter()
            .enumerate()
            .map(|(i, &l)| row.get_ref(i).map(|v| cell(l, v)).map_err(db_err))
            .collect::<Result<Vec<_>>>()?;
        batch.push(cells);
        if batch.len() >= ROW_BATCH {
            sink.send_blocking(QueryEvent::Rows {
                index: my_index,
                rows: std::mem::replace(&mut batch, Vec::with_capacity(ROW_BATCH)),
            })?;
        }
    }
    if !batch.is_empty() {
        sink.send_blocking(QueryEvent::Rows {
            index: my_index,
            rows: batch,
        })?;
    }
    sink.send_blocking(QueryEvent::ResultEnd {
        index: my_index,
        row_count: count,
        truncated,
    })
}

/// Column affinity from the declared type (SQLite §3.1 rules, plus hints).
fn logical(decl: &str) -> LogicalType {
    let d = decl.to_ascii_uppercase();
    if d.is_empty() {
        LogicalType::Other
    } else if d.contains("BOOL") {
        LogicalType::Bool
    } else if d.contains("INT") {
        LogicalType::Int
    } else if d.contains("CHAR") || d.contains("CLOB") || d.contains("TEXT") {
        LogicalType::Text
    } else if d.contains("BLOB") {
        LogicalType::Bytes
    } else if d.contains("REAL") || d.contains("FLOA") || d.contains("DOUB") {
        LogicalType::Float
    } else if d.contains("DATETIME") || d.contains("TIMESTAMP") {
        LogicalType::Timestamp
    } else if d.contains("DATE") {
        LogicalType::Date
    } else if d.contains("JSON") {
        LogicalType::Json
    } else if d.contains("NUM") || d.contains("DEC") {
        LogicalType::Decimal
    } else {
        LogicalType::Other
    }
}

/// SQLite is dynamically typed: encode what is stored, nudged by the column's
/// declared type where the two agree.
fn cell(logical: LogicalType, v: ValueRef<'_>) -> Value {
    match v {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(i) => match logical {
            LogicalType::Bool if i == 0 || i == 1 => Value::Bool(i == 1),
            LogicalType::Decimal => text(i.to_string()),
            _ => int(i),
        },
        ValueRef::Real(f) => match logical {
            LogicalType::Decimal => text(f.to_string()),
            _ => float(f),
        },
        ValueRef::Text(t) => text(String::from_utf8_lossy(t)),
        ValueRef::Blob(b) => bytes(b),
    }
}

fn to_sqlite(p: &Param) -> rusqlite::types::Value {
    use rusqlite::types::Value as V;
    match p {
        Param::Null => V::Null,
        Param::Bool(b) => V::Integer(i64::from(*b)),
        Param::Int(i) => V::Integer(*i),
        Param::Float(f) => V::Real(*f),
        Param::Text(s) => V::Text(s.clone()),
    }
}

fn db_err(e: rusqlite::Error) -> DbError {
    match &e {
        rusqlite::Error::SqliteFailure(err, msg) => {
            if err.code == rusqlite::ErrorCode::OperationInterrupted {
                return DbError::Cancelled;
            }
            DbError::Database {
                code: Some(err.extended_code.to_string()),
                message: msg.clone().unwrap_or_else(|| err.to_string()),
            }
        }
        _ => DbError::Database {
            code: None,
            message: e.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_must_be_inside_allowed_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let allowed = vec![dir.path().to_path_buf()];
        assert!(check_path(&dir.path().join("a.db"), &allowed).is_ok());
        assert!(check_path(&dir.path().join("../escape.db"), &allowed).is_err());
        assert!(check_path(Path::new("relative.db"), &allowed).is_err());
        assert!(check_path(&dir.path().join("a.db"), &[]).is_err());
    }

    #[tokio::test]
    async fn runs_scripts_and_streams_results() {
        let dir = tempfile::tempdir().unwrap();
        let p = SqliteParams {
            path: dir.path().join("t.db").to_string_lossy().into(),
            create: true,
        };
        let driver = SqliteDriver::new(&p, false, &[dir.path().to_path_buf()]).unwrap();
        let mut conn = driver.connect().await.unwrap();
        let rows = super::super::fetch_rows(
            conn.as_mut(),
            "CREATE TABLE t (id INTEGER, ok BOOLEAN, b BLOB); INSERT INTO t VALUES (1, 1, x'ff'), (2, 0, NULL); SELECT * FROM t ORDER BY id",
            vec![],
        )
        .await
        .unwrap();
        assert_eq!(
            rows,
            vec![
                vec![
                    serde_json::json!(1),
                    Value::Bool(true),
                    serde_json::json!("ff")
                ],
                vec![serde_json::json!(2), Value::Bool(false), Value::Null],
            ]
        );
        let rows = super::super::fetch_rows(
            conn.as_mut(),
            "SELECT count(*) FROM t WHERE id > ?1",
            vec![Param::Int(1)],
        )
        .await
        .unwrap();
        assert_eq!(rows, vec![vec![serde_json::json!(1)]]);
        assert!(!conn.in_transaction().await.unwrap());
    }
}
