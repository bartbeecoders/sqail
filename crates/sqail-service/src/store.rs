//! The service's own state in `service.db` (SQLite): API tokens, connection
//! profiles and the audit log.
//!
//! Access is serialized through one connection behind a mutex. Every operation
//! here is a sub-millisecond point query, so this is simpler than a pool and
//! fast enough for an interactive tool.

use std::path::Path;
use std::sync::Mutex;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{Connection as Db, OptionalExtension, Row, params};
use sqail_proto::{
    AuditEntry, AuditPage, Connection, ConnectionParams, CreatedToken, Scope, TokenInfo,
};
use uuid::Uuid;

use crate::crypto::sha256_hex;

/// Ordered schema migrations; `PRAGMA user_version` records how many ran.
const MIGRATIONS: &[&str] = &[
    r#"
    CREATE TABLE tokens (
        id           TEXT PRIMARY KEY,
        name         TEXT NOT NULL,
        scope        TEXT NOT NULL,
        hash         TEXT NOT NULL UNIQUE,
        created_at   TEXT NOT NULL,
        last_used_at TEXT,
        revoked      INTEGER NOT NULL DEFAULT 0
    );
    CREATE TABLE connections (
        id          TEXT PRIMARY KEY,
        name        TEXT NOT NULL,
        params      TEXT NOT NULL,
        secret      TEXT,
        read_only   INTEGER NOT NULL DEFAULT 0,
        color       TEXT,
        environment TEXT,
        folder      TEXT,
        created_at  TEXT NOT NULL,
        updated_at  TEXT NOT NULL
    );
    CREATE TABLE audit_log (
        id          INTEGER PRIMARY KEY AUTOINCREMENT,
        at          TEXT NOT NULL,
        actor       TEXT NOT NULL,
        action      TEXT NOT NULL,
        target      TEXT,
        detail      TEXT,
        duration_ms INTEGER,
        success     INTEGER NOT NULL
    );
"#,
    // PostgreSQL client private keys, encrypted the same way as `secret`.
    "ALTER TABLE connections ADD COLUMN ssl_key TEXT;",
];

pub struct Store {
    db: Mutex<Db>,
}

/// A connection profile plus its encrypted secret, as stored.
#[derive(Debug, Clone)]
pub struct StoredConnection {
    pub info: Connection,
    /// Encrypted password, when the profile has one.
    pub secret: Option<String>,
    /// Encrypted PostgreSQL client private key, when the profile has one.
    pub ssl_key: Option<String>,
}

/// Fields written on insert/update (secret already encrypted).
pub struct ConnectionRecord<'a> {
    pub name: &'a str,
    pub params: &'a ConnectionParams,
    pub secret: Option<&'a str>,
    /// Already encrypted.
    pub ssl_key: Option<&'a str>,
    pub read_only: bool,
    pub color: Option<&'a str>,
    pub environment: Option<&'a str>,
    pub folder: Option<&'a str>,
}

/// An audit event to record.
pub struct AuditEvent<'a> {
    pub actor: &'a str,
    pub action: &'a str,
    pub target: Option<String>,
    pub detail: Option<String>,
    pub duration_ms: Option<i64>,
    pub success: bool,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        let db = Db::open(path).with_context(|| format!("opening {}", path.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        }
        Self::init(db)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Db::open_in_memory()?)
    }

    fn init(mut db: Db) -> Result<Self> {
        db.pragma_update(None, "journal_mode", "WAL")?;
        db.pragma_update(None, "foreign_keys", "ON")?;
        db.busy_timeout(std::time::Duration::from_secs(5))?;
        let version: i64 = db.pragma_query_value(None, "user_version", |r| r.get(0))?;
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(version as usize) {
            let tx = db.transaction()?;
            tx.execute_batch(sql)
                .with_context(|| format!("migration {}", i + 1))?;
            tx.pragma_update(None, "user_version", (i + 1) as i64)?;
            tx.commit()?;
        }
        Ok(Self { db: Mutex::new(db) })
    }

    /// Write a consistent copy of the whole database to `dest` (which must
    /// not exist). Safe while the service is running.
    pub fn backup_to(&self, dest: &Path) -> Result<()> {
        if dest.exists() {
            anyhow::bail!("{} already exists", dest.display());
        }
        self.db()
            .execute("VACUUM INTO ?1", [dest.to_string_lossy()])
            .with_context(|| format!("writing {}", dest.display()))?;
        Ok(())
    }

    fn db(&self) -> std::sync::MutexGuard<'_, Db> {
        // A panic while holding the lock cannot leave SQLite inconsistent
        // (statements are atomic), so recovering from poisoning is safe.
        self.db.lock().unwrap_or_else(|e| e.into_inner())
    }

    // ------------------------------------------------------------ tokens --

    pub fn token_create(&self, name: &str, scope: Scope) -> Result<CreatedToken> {
        let secret = generate_token();
        let info = TokenInfo {
            id: Uuid::new_v4(),
            name: name.to_string(),
            scope,
            created_at: Utc::now(),
            last_used_at: None,
            revoked: false,
        };
        self.db().execute(
            "INSERT INTO tokens (id, name, scope, hash, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                info.id.to_string(),
                info.name,
                scope.as_str(),
                sha256_hex(&secret),
                ts(info.created_at)
            ],
        )?;
        Ok(CreatedToken {
            info,
            token: secret,
        })
    }

    /// Look up a presented bearer token. Returns `None` for unknown tokens.
    pub fn token_by_secret(&self, secret: &str) -> Result<Option<TokenInfo>> {
        let hash = sha256_hex(secret);
        let db = self.db();
        let found = db
            .query_row(
                &format!("SELECT {TOKEN_COLS}, hash FROM tokens WHERE hash = ?1"),
                [&hash],
                |r| Ok((token_from_row(r)?, r.get::<_, String>(6)?)),
            )
            .optional()?;
        // The index lookup already matched; compare again in constant time so
        // correctness never depends on SQLite's string comparison.
        Ok(found.and_then(|(info, stored)| {
            use subtle::ConstantTimeEq;
            bool::from(stored.as_bytes().ct_eq(hash.as_bytes())).then_some(info)
        }))
    }

    pub fn token_touch(&self, id: Uuid) -> Result<()> {
        self.db().execute(
            "UPDATE tokens SET last_used_at = ?2 WHERE id = ?1",
            params![id.to_string(), ts(Utc::now())],
        )?;
        Ok(())
    }

    pub fn token_list(&self) -> Result<Vec<TokenInfo>> {
        let db = self.db();
        let mut stmt = db.prepare(&format!(
            "SELECT {TOKEN_COLS} FROM tokens ORDER BY created_at"
        ))?;
        let rows = stmt.query_map([], token_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn token_revoke(&self, id: Uuid) -> Result<bool> {
        Ok(self.db().execute(
            "UPDATE tokens SET revoked = 1 WHERE id = ?1 AND revoked = 0",
            [id.to_string()],
        )? == 1)
    }

    pub fn active_admin_tokens(&self) -> Result<i64> {
        Ok(self.db().query_row(
            "SELECT count(*) FROM tokens WHERE scope = 'admin' AND revoked = 0",
            [],
            |r| r.get(0),
        )?)
    }

    // ------------------------------------------------------- connections --

    pub fn connection_list(&self) -> Result<Vec<Connection>> {
        let db = self.db();
        let mut stmt = db.prepare(&format!(
            "SELECT {CONN_COLS} FROM connections ORDER BY folder, name"
        ))?;
        let rows = stmt.query_map([], stored_from_row)?;
        rows.map(|r| Ok(r?.info)).collect()
    }

    pub fn connection_get(&self, id: Uuid) -> Result<Option<StoredConnection>> {
        Ok(self
            .db()
            .query_row(
                &format!("SELECT {CONN_COLS} FROM connections WHERE id = ?1"),
                [id.to_string()],
                stored_from_row,
            )
            .optional()?)
    }

    pub fn connection_insert(&self, rec: &ConnectionRecord<'_>) -> Result<Uuid> {
        let id = Uuid::new_v4();
        let now = ts(Utc::now());
        self.db().execute(
            "INSERT INTO connections (id, name, params, secret, read_only, color, environment, folder, created_at, updated_at, ssl_key)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9, ?10)",
            params![
                id.to_string(),
                rec.name,
                serde_json::to_string(rec.params)?,
                rec.secret,
                rec.read_only,
                rec.color,
                rec.environment,
                rec.folder,
                now,
                rec.ssl_key
            ],
        )?;
        Ok(id)
    }

    pub fn connection_update(&self, id: Uuid, rec: &ConnectionRecord<'_>) -> Result<bool> {
        Ok(self.db().execute(
            "UPDATE connections SET name = ?2, params = ?3, secret = ?4, read_only = ?5, color = ?6,
                    environment = ?7, folder = ?8, updated_at = ?9, ssl_key = ?10
             WHERE id = ?1",
            params![
                id.to_string(),
                rec.name,
                serde_json::to_string(rec.params)?,
                rec.secret,
                rec.read_only,
                rec.color,
                rec.environment,
                rec.folder,
                ts(Utc::now()),
                rec.ssl_key
            ],
        )? == 1)
    }

    pub fn connection_delete(&self, id: Uuid) -> Result<bool> {
        Ok(self
            .db()
            .execute("DELETE FROM connections WHERE id = ?1", [id.to_string()])?
            == 1)
    }

    // ------------------------------------------------------------- audit --

    pub fn audit(&self, ev: AuditEvent<'_>) {
        let res = self.db().execute(
            "INSERT INTO audit_log (at, actor, action, target, detail, duration_ms, success)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                ts(Utc::now()),
                ev.actor,
                ev.action,
                ev.target,
                ev.detail,
                ev.duration_ms,
                ev.success
            ],
        );
        // Losing an audit row must never fail the user's request, but it must
        // be visible to the operator.
        if let Err(e) = res {
            tracing::error!(error = %e, action = ev.action, "failed to write audit log");
        }
    }

    pub fn audit_list(&self, before: Option<i64>, limit: u32) -> Result<AuditPage> {
        let db = self.db();
        let mut stmt = db.prepare(
            "SELECT id, at, actor, action, target, detail, duration_ms, success FROM audit_log
             WHERE ?1 IS NULL OR id < ?1 ORDER BY id DESC LIMIT ?2",
        )?;
        let items: Vec<AuditEntry> = stmt
            .query_map(params![before, limit], |r| {
                Ok(AuditEntry {
                    id: r.get(0)?,
                    at: parse_ts(r, 1)?,
                    actor: r.get(2)?,
                    action: r.get(3)?,
                    target: r.get(4)?,
                    detail: r.get(5)?,
                    duration_ms: r.get(6)?,
                    success: r.get(7)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        let next_before = (items.len() == limit as usize)
            .then(|| items.last().map(|e| e.id))
            .flatten();
        Ok(AuditPage { items, next_before })
    }
}

const TOKEN_COLS: &str = "id, name, scope, created_at, last_used_at, revoked";
const CONN_COLS: &str = "id, name, params, secret, read_only, color, environment, folder, created_at, updated_at, ssl_key";

fn token_from_row(r: &Row<'_>) -> rusqlite::Result<TokenInfo> {
    Ok(TokenInfo {
        id: parse_uuid(r, 0)?,
        name: r.get(1)?,
        scope: r
            .get::<_, String>(2)?
            .parse()
            .map_err(|e: String| conv_err(2, e))?,
        created_at: parse_ts(r, 3)?,
        last_used_at: r
            .get::<_, Option<String>>(4)?
            .map(|s| {
                s.parse()
                    .map_err(|e: chrono::ParseError| conv_err(4, e.to_string()))
            })
            .transpose()?,
        revoked: r.get(5)?,
    })
}

fn stored_from_row(r: &Row<'_>) -> rusqlite::Result<StoredConnection> {
    let params: ConnectionParams =
        serde_json::from_str(&r.get::<_, String>(2)?).map_err(|e| conv_err(2, e.to_string()))?;
    let secret: Option<String> = r.get(3)?;
    let ssl_key: Option<String> = r.get(10)?;
    Ok(StoredConnection {
        info: Connection {
            id: parse_uuid(r, 0)?,
            name: r.get(1)?,
            engine: params.engine(),
            params,
            has_password: secret.is_some(),
            has_ssl_client_key: ssl_key.is_some(),
            read_only: r.get(4)?,
            color: r.get(5)?,
            environment: r.get(6)?,
            folder: r.get(7)?,
            created_at: parse_ts(r, 8)?,
            updated_at: parse_ts(r, 9)?,
        },
        secret,
        ssl_key,
    })
}

fn ts(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn parse_ts(r: &Row<'_>, i: usize) -> rusqlite::Result<DateTime<Utc>> {
    r.get::<_, String>(i)?
        .parse()
        .map_err(|e: chrono::ParseError| conv_err(i, e.to_string()))
}

fn parse_uuid(r: &Row<'_>, i: usize) -> rusqlite::Result<Uuid> {
    r.get::<_, String>(i)?
        .parse()
        .map_err(|e: uuid::Error| conv_err(i, e.to_string()))
}

fn conv_err(i: usize, msg: String) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(i, rusqlite::types::Type::Text, msg.into())
}

/// `sq2_` + 43 base62 characters (≈256 bits of entropy).
fn generate_token() -> String {
    const ALPHABET: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
    let mut out = String::from("sq2_");
    for _ in 0..43 {
        out.push(ALPHABET[rand::random_range(0..ALPHABET.len())] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backup_is_a_complete_copy() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("service.db")).unwrap();
        let created = store.token_create("t", Scope::Query).unwrap();
        let copy = dir.path().join("copy.db");
        store.backup_to(&copy).unwrap();
        let restored = Store::open(&copy).unwrap();
        assert_eq!(restored.token_list().unwrap()[0].id, created.info.id);
        assert!(store.backup_to(&copy).is_err(), "never overwrites");
    }
    use sqail_proto::SqliteParams;

    #[test]
    fn token_lifecycle() {
        let s = Store::open_in_memory().unwrap();
        let created = s.token_create("ci", Scope::Query).unwrap();
        assert!(created.token.starts_with("sq2_") && created.token.len() == 47);
        let found = s.token_by_secret(&created.token).unwrap().unwrap();
        assert_eq!(found.id, created.info.id);
        assert!(s.token_by_secret("sq2_nope").unwrap().is_none());
        assert!(s.token_revoke(found.id).unwrap());
        assert!(s.token_by_secret(&created.token).unwrap().unwrap().revoked);
        assert!(
            !s.token_revoke(found.id).unwrap(),
            "second revoke is a no-op"
        );
    }

    #[test]
    fn connection_crud() {
        let s = Store::open_in_memory().unwrap();
        let params = ConnectionParams::Sqlite(SqliteParams {
            path: "/tmp/x.db".into(),
            create: false,
        });
        let mut rec = ConnectionRecord {
            name: "local",
            params: &params,
            secret: Some("v1:abc"),
            ssl_key: None,
            read_only: false,
            color: None,
            environment: Some("dev"),
            folder: None,
        };
        let id = s.connection_insert(&rec).unwrap();
        let got = s.connection_get(id).unwrap().unwrap();
        assert_eq!(got.info.name, "local");
        assert!(got.info.has_password);
        assert!(!got.info.has_ssl_client_key);
        rec.name = "renamed";
        rec.secret = None;
        rec.ssl_key = Some("v1:key");
        assert!(s.connection_update(id, &rec).unwrap());
        let got = s.connection_get(id).unwrap().unwrap();
        assert_eq!(got.info.name, "renamed");
        assert!(!got.info.has_password);
        assert!(got.info.has_ssl_client_key);
        assert_eq!(got.ssl_key.as_deref(), Some("v1:key"));
        assert_eq!(s.connection_list().unwrap().len(), 1);
        assert!(s.connection_delete(id).unwrap());
        assert!(s.connection_get(id).unwrap().is_none());
    }

    #[test]
    fn migration_adds_the_ssl_key_column() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("service.db");
        {
            let db = rusqlite::Connection::open(&path).unwrap();
            db.execute_batch(MIGRATIONS[0]).unwrap();
            db.pragma_update(None, "user_version", 1).unwrap();
        }
        let s = Store::open(&path).unwrap();
        let params = ConnectionParams::Sqlite(SqliteParams {
            path: "/tmp/x.db".into(),
            create: false,
        });
        let id = s
            .connection_insert(&ConnectionRecord {
                name: "local",
                params: &params,
                secret: None,
                ssl_key: Some("v1:key"),
                read_only: false,
                color: None,
                environment: None,
                folder: None,
            })
            .unwrap();
        let got = s.connection_get(id).unwrap().unwrap();
        assert!(got.info.has_ssl_client_key);
        assert_eq!(got.ssl_key.as_deref(), Some("v1:key"));
    }

    #[test]
    fn audit_paging() {
        let s = Store::open_in_memory().unwrap();
        for i in 0..5 {
            s.audit(AuditEvent {
                actor: "t",
                action: "test",
                target: Some(i.to_string()),
                detail: None,
                duration_ms: None,
                success: true,
            });
        }
        let p1 = s.audit_list(None, 3).unwrap();
        assert_eq!(p1.items.len(), 3);
        assert_eq!(p1.items[0].target.as_deref(), Some("4"));
        let p2 = s.audit_list(p1.next_before, 3).unwrap();
        assert_eq!(p2.items.len(), 2);
        assert!(p2.next_before.is_none());
    }
}
