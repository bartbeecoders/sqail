//! Which SQL the assistant may run on its own: a single read-only statement.
//!
//! This is the first of two barriers. The second is that every assistant
//! query runs in a transaction that is always rolled back (read-only on
//! Postgres), see [`super::mcp::ServiceBackend`].

use sqail_client::proto::Engine;

use crate::sql::lex::{Kind, tokenize};

/// Statements the assistant may start with.
const ALLOWED_FIRST: &[&str] = &["SELECT", "WITH", "VALUES", "TABLE", "SHOW"];

/// Words that can write or reach outside the database from inside an
/// otherwise allowed statement (data-modifying CTEs, `SELECT … INTO`,
/// `FOR UPDATE`, remote rowsets), anywhere outside strings, comments and
/// quoted names. Statements that *start* with DDL, DCL, `EXEC`, `PRAGMA`
/// and the like are already refused by [`ALLOWED_FIRST`]; common column names
/// such as `lock`, `release` or `load` stay usable.
const FORBIDDEN: &[&str] = &[
    "INSERT",
    "UPDATE",
    "DELETE",
    "MERGE",
    "UPSERT",
    "INTO",
    "EXEC",
    "EXECUTE",
    "BULK",
    "OPENROWSET",
    "OPENDATASOURCE",
    "OPENQUERY",
    "GO",
];

/// Functions with side effects, or that read files on the server.
const FORBIDDEN_FUNCTIONS: &[&str] = &[
    "nextval",
    "setval",
    "set_config",
    "pg_terminate_backend",
    "pg_cancel_backend",
    "pg_reload_conf",
    "pg_rotate_logfile",
    "pg_switch_wal",
    "pg_create_restore_point",
    "pg_promote",
    "pg_notify",
    "pg_logical_emit_message",
    "pg_read_file",
    "pg_read_binary_file",
    "pg_ls_dir",
    "pg_stat_file",
    "pg_ls_logdir",
    "pg_ls_waldir",
    "pg_ls_tmpdir",
    "lo_import",
    "lo_export",
    "lo_unlink",
    "lo_create",
    "lo_from_bytea",
    "lo_put",
    "dblink",
    "dblink_exec",
    "dblink_connect",
    "xp_cmdshell",
    "xp_regread",
    "xp_dirtree",
    "xp_fileexist",
    "sp_executesql",
    "sp_oacreate",
    "load_extension",
    "readfile",
    "writefile",
];

/// `Ok(())` when `sql` is one read-only statement, else why not.
pub fn check_read_only(sql: &str, engine: Engine) -> Result<(), String> {
    let tokens: Vec<_> = tokenize(sql, Some(engine))
        .into_iter()
        .filter(|t| !matches!(t.kind, Kind::Space | Kind::Comment))
        .collect();
    let Some(first) = tokens.first() else {
        return Err("the query is empty".into());
    };
    // One statement: nothing but a trailing `;` after the first `;`.
    if let Some(i) = tokens.iter().position(|t| t.kind == Kind::Semicolon)
        && tokens[i + 1..].iter().any(|t| t.kind != Kind::Semicolon)
    {
        return Err("run one statement at a time".into());
    }
    let word = |t: &crate::sql::lex::Token| sql[t.range.clone()].to_ascii_uppercase();
    let first_word = if first.kind == Kind::Word {
        word(first)
    } else {
        String::new()
    };
    if !ALLOWED_FIRST.contains(&first_word.as_str()) {
        return Err(format!(
            "only read-only queries are allowed (SELECT, WITH, VALUES); this one starts with {:?}",
            &sql[first.range.clone()]
        ));
    }
    for t in tokens.iter().filter(|t| t.kind == Kind::Word) {
        let w = word(t);
        if FORBIDDEN.contains(&w.as_str()) {
            return Err(format!(
                "{w} is not allowed: the assistant may only read data"
            ));
        }
        let lower = w.to_ascii_lowercase();
        if FORBIDDEN_FUNCTIONS.contains(&lower.as_str()) {
            return Err(format!("{lower}() is not allowed: it has side effects"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(sql: &str) {
        for e in [Engine::Postgres, Engine::Mssql, Engine::Sqlite] {
            assert_eq!(check_read_only(sql, e), Ok(()), "{e:?}: {sql}");
        }
    }

    fn rejected(sql: &str, engine: Engine) {
        assert!(
            check_read_only(sql, engine).is_err(),
            "{engine:?} allowed: {sql}"
        );
    }

    #[test]
    fn plain_reads_are_allowed() {
        ok("SELECT 1");
        ok("select * from sales.orders where status = 'new';");
        ok(
            "SELECT o.id,\n\n  c.name\nFROM sales.orders o\n\nJOIN sales.customers c ON c.id = o.customer_id",
        );
        ok("WITH t AS (SELECT 1 AS x) SELECT x FROM t");
        ok("-- top customers\nSELECT TOP 10 * FROM sales.customers /* c */");
        ok("VALUES (1), (2)");
        // Forbidden words inside strings, comments and quoted names are fine.
        ok("SELECT 'DELETE FROM x; DROP TABLE y' AS note -- UPDATE");
        ok("SELECT \"update\" FROM \"insert\"");
        ok("SELECT replace(name, 'a', 'b'), coalesce(x, 0) FROM t");
        ok("SELECT lock, release, load, status FROM jobs");
    }

    #[test]
    fn writes_and_side_effects_are_rejected() {
        for e in [Engine::Postgres, Engine::Mssql, Engine::Sqlite] {
            rejected("DELETE FROM sales.orders", e);
            rejected("UPDATE t SET a = 1", e);
            rejected("INSERT INTO t VALUES (1)", e);
            rejected("DROP TABLE t", e);
            rejected("SELECT * INTO backup FROM t", e);
            rejected("SELECT 1; DELETE FROM t", e);
            rejected("SELECT 1; SELECT 2", e);
            rejected("WITH d AS (DELETE FROM t RETURNING *) SELECT * FROM d", e);
            rejected("EXPLAIN ANALYZE DELETE FROM t", e);
            rejected("", e);
            rejected("  -- only a comment", e);
        }
        rejected("SELECT nextval('s')", Engine::Postgres);
        rejected("SELECT pg_read_file('/etc/passwd')", Engine::Postgres);
        rejected("SELECT pg_terminate_backend(123)", Engine::Postgres);
        rejected("SELECT * FROM t FOR UPDATE", Engine::Postgres);
        rejected("EXEC sp_who", Engine::Mssql);
        rejected("SELECT 1\nGO\nSELECT 2", Engine::Mssql);
        rejected(
            "SELECT * FROM OPENROWSET('SQLNCLI', 'x', 'SELECT 1')",
            Engine::Mssql,
        );
        rejected("SELECT load_extension('x')", Engine::Sqlite);
        rejected("PRAGMA writable_schema = 1", Engine::Sqlite);
    }
}
