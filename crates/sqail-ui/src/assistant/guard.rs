//! Which SQL the assistant may run on its own: a single read-only statement.
//!
//! This is the first of three barriers. The second is that the service is
//! asked to refuse anything but a single statement
//! (`QueryRequest::single_statement`), so a boundary this lexer misreads
//! cannot smuggle in a second statement. The third is that every assistant
//! query runs in a transaction that is always rolled back (read-only on
//! Postgres), see [`super::mcp::ServiceBackend`]. None of them replaces a
//! low-privilege login for the assistant's connection (docs/security.md).

use sqail_client::proto::Engine;

use crate::sql::lex::{Kind, Token, tokenize_checked};

/// Statements the assistant may start with.
const ALLOWED_FIRST: &[&str] = &["SELECT", "WITH", "VALUES", "TABLE", "SHOW"];

/// Words refused anywhere outside strings, comments and quoted names: they
/// write (data-modifying CTEs, `SELECT … INTO`, `FOR UPDATE`), reach outside
/// the database, control the transaction, change the schema, the session or
/// the server, or start another statement. Common column names such as
/// `lock`, `release`, `load`, `start` or `comment` stay usable; the
/// statements they begin need a statement separator, which is refused.
const FORBIDDEN: &[&str] = &[
    // Writes and remote access.
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
    // Transaction control.
    "BEGIN",
    "COMMIT",
    "ROLLBACK",
    "ABORT",
    "SAVE",
    "SAVEPOINT",
    "PREPARE",
    // Schema and permissions.
    "CREATE",
    "ALTER",
    "DROP",
    "TRUNCATE",
    "GRANT",
    "REVOKE",
    "DENY",
    // Session, server and maintenance.
    "SET",
    "RESET",
    "DISCARD",
    "USE",
    "DECLARE",
    "KILL",
    "SHUTDOWN",
    "DBCC",
    "BACKUP",
    "RESTORE",
    "WAITFOR",
    "RECONFIGURE",
    "CHECKPOINT",
    "ATTACH",
    "DETACH",
    "PRAGMA",
    "VACUUM",
    "REINDEX",
    "ANALYZE",
    "ANALYSE",
    "CLUSTER",
    "REFRESH",
    "COPY",
    "CALL",
    "DO",
    "NOTIFY",
    "LISTEN",
    "UNLISTEN",
    "IMPORT",
];

/// T-SQL statements that need no separator: refused anywhere on SQL Server,
/// where they are reserved words and so never a bare column name.
const FORBIDDEN_MSSQL: &[&str] = &[
    "IF",
    "WHILE",
    "GOTO",
    "RETURN",
    "BREAK",
    "CONTINUE",
    "PRINT",
    "RAISERROR",
    "THROW",
    "OPEN",
    "CLOSE",
    "DEALLOCATE",
    "SETUSER",
    "REVERT",
    "READTEXT",
    "WRITETEXT",
    "UPDATETEXT",
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
    "query_to_xml",
    "query_to_xmlschema",
    "query_to_xml_and_xmlschema",
    "cursor_to_xml",
    "cursor_to_xmlschema",
    "lo_import",
    "lo_export",
    "lo_unlink",
    "lo_create",
    "lo_from_bytea",
    "lo_put",
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

/// Function name prefixes refused like [`FORBIDDEN_FUNCTIONS`]: advisory
/// locks (session-level ones outlive the rollback) and remote queries.
const FORBIDDEN_PREFIXES: &[&str] = &["pg_advisory", "pg_try_advisory", "dblink"];

/// Words before a top-level `SELECT` that keep it in the same statement.
const SET_OPERATORS: &[&str] = &["UNION", "ALL", "EXCEPT", "INTERSECT", "DISTINCT"];

/// `Ok(())` when `sql` is one read-only statement, else why not.
pub fn check_read_only(sql: &str, engine: Engine) -> Result<(), String> {
    let (tokens, unterminated) = tokenize_checked(sql, Some(engine));
    if unterminated {
        return Err("the query ends inside an unterminated string, name or comment".into());
    }
    let tokens: Vec<Token> = tokens
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
    let word = |t: &Token| sql[t.range.clone()].to_ascii_uppercase();
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
    let mssql = engine == Engine::Mssql;
    for t in tokens.iter().filter(|t| t.kind == Kind::Word) {
        let w = word(t);
        if FORBIDDEN.contains(&w.as_str()) || (mssql && FORBIDDEN_MSSQL.contains(&w.as_str())) {
            return Err(format!(
                "{w} is not allowed: the assistant may only read data"
            ));
        }
        let lower = w.to_ascii_lowercase();
        if FORBIDDEN_FUNCTIONS.contains(&lower.as_str())
            || FORBIDDEN_PREFIXES.iter().any(|p| lower.starts_with(p))
        {
            return Err(format!("{lower}() is not allowed: it has side effects"));
        }
    }
    // T-SQL needs no `;` between statements: a second top-level SELECT that
    // does not follow a set operator starts a new one.
    if mssql {
        let mut depth = 0i32;
        let mut selects = 0;
        let mut prev: Option<String> = None;
        for t in &tokens {
            match t.kind {
                Kind::Open => depth += 1,
                Kind::Close => depth -= 1,
                Kind::Word => {
                    let w = word(t);
                    if depth <= 0
                        && w == "SELECT"
                        && !prev.as_deref().is_some_and(|p| SET_OPERATORS.contains(&p))
                    {
                        selects += 1;
                        if selects > 1 {
                            return Err("run one statement at a time".into());
                        }
                    }
                    prev = Some(w);
                    continue;
                }
                _ => {}
            }
            prev = None;
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

    #[test]
    fn transaction_control_and_ddl_are_rejected_anywhere() {
        for e in [Engine::Postgres, Engine::Mssql, Engine::Sqlite] {
            for sql in [
                "SELECT 1 COMMIT",
                "SELECT 1 ROLLBACK",
                "SELECT 1 FROM t WHERE 1 = 1 COMMIT TRANSACTION",
                "SELECT 1 BEGIN TRAN",
                "SELECT 1 SAVE TRANSACTION x",
                "SELECT 1 DROP TABLE t",
                "SELECT 1 CREATE TABLE t (a int)",
                "SELECT 1 ALTER TABLE t ADD b int",
                "SELECT 1 TRUNCATE TABLE t",
                "SELECT 1 GRANT SELECT ON t TO public",
                "SELECT 1 DECLARE @x int",
                "SELECT 1 SET x = 1",
                "SELECT 1 USE master",
                "SELECT 1 WAITFOR DELAY '00:00:10'",
                "SELECT 1 ATTACH DATABASE 'x' AS y",
                "SELECT 1 VACUUM",
                "SELECT 1 COPY t TO STDOUT",
                "WITH x AS (SELECT 1) SELECT * FROM x ROLLBACK",
                "SELECT 1 /* x */ COMMIT -- y",
            ] {
                rejected(sql, e);
            }
        }
    }

    #[test]
    fn unterminated_input_is_rejected() {
        for e in [Engine::Postgres, Engine::Mssql, Engine::Sqlite] {
            rejected("SELECT 'abc", e);
            rejected("SELECT 1 /* x", e);
            rejected("SELECT \"abc", e);
        }
        rejected("SELECT $$ x", Engine::Postgres);
        rejected("SELECT [x", Engine::Mssql);
    }

    #[test]
    fn adversarial_postgres() {
        let pg = Engine::Postgres;
        // Backslash escapes in E'' strings end where Postgres ends them.
        rejected(r"SELECT E'\''; DELETE FROM t; SELECT ''", pg);
        rejected(r"SELECT E'a\\'; COMMIT; SELECT 1", pg);
        // `$` inside a name does not open a dollar-quoted string.
        rejected("SELECT 1 AS a$b$; COMMIT; SELECT $b$", pg);
        rejected("SELECT 1; END", pg);
        rejected("SELECT pg_advisory_lock(1)", pg);
        rejected("SELECT pg_try_advisory_lock_shared(1)", pg);
        rejected("SELECT * FROM dblink('x', 'y') AS t(a int)", pg);
        rejected("SELECT query_to_xml('select 1', true, true, '')", pg);
        rejected("SELECT setval('s', 1)", pg);
        rejected("SELECT 1 DO $$ BEGIN END $$", pg);
        rejected("SELECT 1 FOR SHARE NOWAIT; LOCK t", pg);
        assert_eq!(check_read_only(r"SELECT E'it\'s', 'a\'", pg), Ok(()));
        assert_eq!(check_read_only("SELECT a$b FROM t", pg), Ok(()));
        assert_eq!(check_read_only("TABLE sales.orders", pg), Ok(()));
        assert_eq!(check_read_only("SHOW search_path", pg), Ok(()));
    }

    #[test]
    fn adversarial_sql_server() {
        let ms = Engine::Mssql;
        // Statements need no `;` in T-SQL.
        rejected("SELECT 1 SELECT 2", ms);
        rejected("SELECT * FROM t\nSELECT * FROM u", ms);
        rejected("WITH c AS (SELECT 1 AS a) SELECT a FROM c SELECT 2", ms);
        rejected("SELECT 1 IF 1 = 1 SELECT 2", ms);
        rejected("SELECT 1 WHILE 1 = 1 BREAK", ms);
        rejected("SELECT 1 PRINT 'x'", ms);
        rejected("SELECT 1 RAISERROR('x', 16, 1)", ms);
        rejected("SELECT 1 KILL 55", ms);
        rejected("SELECT 1 DBCC CHECKDB", ms);
        rejected("SELECT 1 SHUTDOWN", ms);
        rejected("SELECT 1 BACKUP DATABASE d TO DISK = 'x'", ms);
        rejected("SELECT 1 DENY SELECT ON t TO u", ms);
        rejected("SELECT 1\nGO 2\nSELECT 2", ms);
        rejected("SELECT 'a''' COMMIT", ms);
        rejected("SELECT [a]]] COMMIT", ms);
        rejected("SELECT N'x' ROLLBACK", ms);
        rejected("SELECT 1 /* /* */ */ COMMIT", ms);
        for sql in [
            "SELECT 1 UNION SELECT 2",
            "SELECT 1 UNION ALL SELECT 2 EXCEPT SELECT 3 INTERSECT SELECT 4",
            "WITH c AS (SELECT 1 AS a), d AS (SELECT 2 AS b) SELECT * FROM c, d",
            "SELECT * FROM (SELECT 1 AS a) x WHERE a IN (SELECT 1)",
            "SELECT TOP 5 WITH TIES * FROM t WITH (NOLOCK) ORDER BY a",
            "SELECT a FROM t ORDER BY a OFFSET 0 ROWS FETCH NEXT 5 ROWS ONLY",
            "SELECT IIF(a > 1, 'x', 'y') FROM t",
        ] {
            assert_eq!(check_read_only(sql, ms), Ok(()), "{sql}");
        }
    }

    #[test]
    fn adversarial_sqlite() {
        let lite = Engine::Sqlite;
        // SQLite block comments do not nest: the first `*/` ends them.
        rejected("SELECT 1 /* /* */ ; DELETE FROM t; /* */", lite);
        rejected("SELECT 1 /* /* */ ; COMMIT; /* */", lite);
        // No backslash escapes in SQLite strings.
        rejected(r"SELECT 'a\'; DELETE FROM t; SELECT '", lite);
        rejected(r"SELECT E'a\'; COMMIT; SELECT '", lite);
        rejected("SELECT 1 DETACH DATABASE x", lite);
        rejected("SELECT 1 REINDEX", lite);
        rejected("SELECT 1 ANALYZE", lite);
        assert_eq!(check_read_only("SELECT [a b] FROM `t`", lite), Ok(()));
        assert_eq!(check_read_only("SELECT iif(a, 1, 2) FROM t", lite), Ok(()));
    }
}
