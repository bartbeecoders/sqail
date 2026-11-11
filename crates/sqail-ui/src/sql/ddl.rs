//! Whether SQL run in the editor may have changed the schema, so the
//! sidebar's cached tree is reloaded afterwards.

use sqail_client::proto::Engine;

use super::lex::{Kind, tokenize};
use super::statements;

/// Words that change the schema wherever they appear outside strings,
/// comments and quoted names (`IF … CREATE TABLE` in T-SQL, `ALTER TABLE …
/// DROP COLUMN`, …). A `GRANT CREATE …` also counts: a needless reload is
/// cheap, a stale tree is not.
const ANYWHERE: &[&str] = &["CREATE", "ALTER", "DROP"];

/// Whether running `sql` on `engine` may have changed what the schema tree
/// shows: `CREATE` / `ALTER` / `DROP`, `SELECT … INTO` (SQL Server and
/// PostgreSQL), `sp_rename` (SQL Server), `IMPORT FOREIGN SCHEMA`
/// (PostgreSQL), `ATTACH` / `DETACH` (SQLite). Statements hidden in strings
/// (`EXEC('…')`, a function body) are not seen.
pub fn changes_schema(sql: &str, engine: Engine) -> bool {
    statements::split(sql, Some(engine))
        .into_iter()
        .any(|r| statement_changes_schema(&sql[r], engine))
}

fn statement_changes_schema(stmt: &str, engine: Engine) -> bool {
    let words: Vec<String> = tokenize(stmt, Some(engine))
        .into_iter()
        .filter(|t| t.kind == Kind::Word)
        .map(|t| stmt[t.range].to_ascii_uppercase())
        .collect();
    let has = |w: &str| words.iter().any(|x| x == w);
    if ANYWHERE.iter().any(|w| has(w)) {
        return true;
    }
    let first = words.first().map(String::as_str);
    match engine {
        Engine::Mssql | Engine::Postgres => {
            // `SELECT … INTO new_table`: not `INSERT INTO` or `MERGE INTO`
            // in the same batch.
            if matches!(first, Some("SELECT" | "WITH"))
                && has("INTO")
                && !has("INSERT")
                && !has("MERGE")
            {
                return true;
            }
        }
        Engine::Sqlite | Engine::Unknown => {}
    }
    match engine {
        Engine::Mssql => has("SP_RENAME"),
        Engine::Postgres => first == Some("IMPORT"),
        Engine::Sqlite => matches!(first, Some("ATTACH" | "DETACH")),
        Engine::Unknown => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ddl_is_seen() {
        for (sql, engine) in [
            ("CREATE TABLE t (id int)", Engine::Postgres),
            ("create or replace view v as select 1", Engine::Postgres),
            ("SELECT 1;\nDROP TABLE t;", Engine::Sqlite),
            ("ALTER TABLE t ADD c int", Engine::Mssql),
            (
                "IF OBJECT_ID('t') IS NULL\n    CREATE TABLE t (id int)",
                Engine::Mssql,
            ),
            ("SELECT * INTO dbo.copy FROM dbo.t", Engine::Mssql),
            ("select a, b into new_t from t", Engine::Postgres),
            (
                "WITH x AS (SELECT 1 AS a) SELECT a INTO t2 FROM x",
                Engine::Mssql,
            ),
            ("EXEC sp_rename 'dbo.t', 't2'", Engine::Mssql),
            (
                "exec sys.sp_rename N'dbo.t.c', N'd', N'COLUMN'",
                Engine::Mssql,
            ),
            (
                "IMPORT FOREIGN SCHEMA s FROM SERVER x INTO local",
                Engine::Postgres,
            ),
            ("ATTACH DATABASE 'x.db' AS x", Engine::Sqlite),
            ("SELECT 1\nGO\nDROP VIEW v\nGO", Engine::Mssql),
        ] {
            assert!(changes_schema(sql, engine), "{engine:?}: {sql}");
        }
    }

    #[test]
    fn other_statements_are_not() {
        for (sql, engine) in [
            ("SELECT * FROM t", Engine::Postgres),
            ("INSERT INTO t SELECT * FROM u", Engine::Mssql),
            ("INSERT INTO t (a) VALUES (1)", Engine::Postgres),
            (
                "WITH x AS (SELECT 1 AS a) INSERT INTO t SELECT a FROM x",
                Engine::Mssql,
            ),
            (
                "MERGE INTO t USING u ON t.id = u.id WHEN MATCHED THEN DELETE",
                Engine::Mssql,
            ),
            ("UPDATE t SET created = 1", Engine::Sqlite),
            // In strings, comments and quoted names.
            ("SELECT 'DROP TABLE t'", Engine::Postgres),
            ("-- create table t\nSELECT 1", Engine::Mssql),
            ("SELECT \"create\" FROM t", Engine::Postgres),
            ("SELECT [drop] FROM t", Engine::Mssql),
            // `SELECT … INTO` only on engines where it creates a table.
            ("SELECT 1 INTO x", Engine::Sqlite),
            ("EXEC dbo.do_things", Engine::Mssql),
        ] {
            assert!(!changes_schema(sql, engine), "{engine:?}: {sql}");
        }
    }
}
