//! Inline edits applied through a real service on every engine.

use std::path::PathBuf;

use sqail_client::proto::{
    ConnectionInput, ConnectionParams, Engine, MssqlAuth, MssqlEncrypt, MssqlParams, PgSslMode,
    PostgresParams, QueryEvent, QueryRequest, SqliteParams,
};
use sqail_client::{Client, On, Target, Trust};
use sqail_ui::editing::{self, EditState, EditTx};
use sqail_ui::results::{Cell, Run};
use uuid::Uuid;

struct Svc {
    rt: tokio::runtime::Runtime,
    dir: tempfile::TempDir,
    _server: sqail_service::Server,
    client: Client,
}

fn svc() -> Svc {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = sqail_service::Config {
        data_dir: dir.path().join("service"),
        bind: "127.0.0.1:0".parse().unwrap(),
        docs_ui: false,
        ..Default::default()
    };
    cfg.sqlite.allowed_dirs = vec![
        dir.path().to_path_buf(),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../dev/data"),
    ];
    std::fs::create_dir_all(&cfg.data_dir).unwrap();
    let server = rt.block_on(sqail_service::start(cfg)).unwrap();
    let client = Client::new(&Target {
        url: format!("https://{}", server.addr),
        token: server.bootstrap_token.clone().unwrap(),
        trust: Trust::Pinned(server.fingerprint.clone()),
        identity: None,
    })
    .unwrap();
    Svc {
        rt,
        dir,
        _server: server,
        client,
    }
}

impl Svc {
    fn connect(&self, params: ConnectionParams, password: Option<&str>) -> Uuid {
        self.rt
            .block_on(self.client.create_connection(&ConnectionInput {
                name: "c".into(),
                params,
                password: password.map(String::from),
                ssl_client_key: None,
                read_only: false,
                color: None,
                environment: None,
                folder: None,
            }))
            .unwrap()
            .id
    }

    fn run(&self, conn: Uuid, sql: &str) -> Run {
        self.run_on(On::Connection(conn), sql)
    }

    fn run_on(&self, on: On, sql: &str) -> Run {
        let events = self
            .rt
            .block_on(self.client.query_all(on, &QueryRequest::new(sql)))
            .unwrap();
        let mut run = Run::new(1, sql.into());
        for e in events {
            if let QueryEvent::Error { message, .. } = &e {
                panic!("{sql}: {message}");
            }
            run.apply(e);
        }
        run
    }

    fn session(&self, conn: Uuid) -> Uuid {
        self.rt.block_on(self.client.open_session(conn)).unwrap().id
    }

    /// Start editing the single result of `select`, run on `session`, the
    /// way the editor does: the table is looked up on the session.
    fn edit(&self, session: Uuid, engine: Engine, select: &str) -> (Run, EditState) {
        let run = self.run_on(On::Session(session), select);
        let source = editing::source_table(select, engine).unwrap();
        let cols = self
            .rt
            .block_on(
                self.client
                    .session_columns(session, source.schema.as_deref(), &source.table),
            )
            .unwrap();
        let e = EditState::new(engine, source, &run.results[0].columns, &cols).unwrap();
        (run, e)
    }

    fn apply(
        &self,
        session: Uuid,
        engine: Engine,
        e: &EditState,
        run: &Run,
        tx: EditTx,
    ) -> anyhow::Result<usize> {
        self.rt.block_on(editing::apply(
            &self.client,
            session,
            engine,
            e.statements(&run.results[0])?,
            tx,
        ))
    }

    /// The single value of a one-row, one-column query.
    fn value(&self, on: On, sql: &str) -> String {
        let run = self.run_on(on, sql);
        let rs = &run.results[0];
        rs.raw_row(0)[0].display(rs.columns[0].logical).into_owned()
    }

    fn in_transaction(&self, session: Uuid) -> bool {
        self.rt
            .block_on(self.client.session(session))
            .unwrap()
            .in_transaction
    }
}

/// Price edit, a new row, a deleted row; then a stale edit that must fail.
fn exercise(s: &Svc, conn: Uuid, engine: Engine, schema: Option<&str>, table: &str) {
    let q = |t: &str| sqail_ui::sql::quote_ident(engine, t);
    let qualified = match schema {
        Some(sc) => format!("{}.{}", q(sc), q(table)),
        None => q(table),
    };
    let select = format!("SELECT id, name, price FROM {qualified} ORDER BY id");
    let session = s.session(conn);
    let (run, mut e) = s.edit(session, engine, &select);
    let rs = &run.results[0];
    e.set(0, 2, Some("12.34".into()), rs); // row id 1
    e.set(0, 1, None, rs); // name → NULL
    e.deleted.insert(1); // row id 2
    e.inserted.push(vec![
        Some(Some("100".into())),
        Some(Some("it's new".into())),
        Some(Some("0.5".into())),
    ]);
    let n = s.apply(session, engine, &e, &run, EditTx::Commit).unwrap();
    assert_eq!(n, 3);
    assert!(!s.in_transaction(session), "committed");

    let after = s.run(conn, &select);
    let rows: Vec<Vec<Cell>> = after.results[0].raw_rows().map(|r| r.to_vec()).collect();
    assert_eq!(rows.len(), 3, "{engine:?}: {rows:?}");
    assert_eq!(rows[0][0], Cell::Int(1));
    assert_eq!(rows[0][1], Cell::Null);
    assert_eq!(
        rows[0][2].display(after.results[0].columns[2].logical),
        "12.34"
    );
    assert_eq!(rows[1][0], Cell::Int(3));
    assert_eq!(rows[2][1], Cell::Text("it's new".into()));

    // Editing a row that no longer exists fails and changes nothing.
    let mut stale = s.edit(session, engine, &select).1;
    stale.set(1, 2, Some("1".into()), rs); // id 2, deleted above
    stale.set(0, 2, Some("99".into()), rs);
    let err = s
        .apply(session, engine, &stale, &run, EditTx::Commit)
        .unwrap_err();
    assert!(!s.in_transaction(session), "rolled back");
    assert!(format!("{err:#}").contains("matched no row"), "{err:#}");
    let again = s.run(conn, &select);
    assert_eq!(
        again.results[0].raw_row(0)[2].display(again.results[0].columns[2].logical),
        "12.34",
        "rolled back"
    );
}

/// Edits on a table keyed by a binary column write real bytes: change a
/// value, insert a row, delete a row by its binary key. `len` is the
/// engine's byte-length function.
fn exercise_binary(s: &Svc, conn: Uuid, engine: Engine, schema: Option<&str>, table: &str) {
    let q = |t: &str| sqail_ui::sql::quote_ident(engine, t);
    let qualified = match schema {
        Some(sc) => format!("{}.{}", q(sc), q(table)),
        None => q(table),
    };
    let len = match engine {
        Engine::Postgres => "octet_length",
        Engine::Mssql => "DATALENGTH",
        _ => "length",
    };
    let select = format!("SELECT k, b FROM {qualified} ORDER BY k");
    let session = s.session(conn);
    let (run, mut e) = s.edit(session, engine, &select);
    let rs = &run.results[0];
    assert_eq!(
        rs.raw_row(0)[1].display(rs.columns[1].logical),
        "0xdeadbeef"
    );

    // Invalid hex never reaches the database.
    e.set(0, 1, Some("0xdeadbeefz".into()), rs);
    let err = e.statements(rs).unwrap_err().to_string();
    assert!(err.contains("is not a binary value"), "{err}");
    // In every transaction mode: nothing is sent, so no transaction opens.
    for tx in [EditTx::Commit, EditTx::Begin, EditTx::Join] {
        let err = s.apply(session, engine, &e, &run, tx).unwrap_err();
        assert!(
            format!("{err:#}").contains("is not a binary value"),
            "{err:#}"
        );
        assert!(!s.in_transaction(session), "{tx:?}: nothing sent");
    }

    e.set(0, 1, Some("0xDEADBEEE".into()), rs); // key 0x0102
    e.set(1, 1, Some("0x".into()), rs); // key 0x0a0b → empty
    e.deleted.insert(2); // key 0x0c0d
    e.inserted
        .push(vec![Some(Some("0xff00".into())), Some(Some("0x00".into()))]);
    let n = s.apply(session, engine, &e, &run, EditTx::Commit).unwrap();
    assert_eq!(n, 4);
    assert!(!s.in_transaction(session), "committed");

    let check = s.run(
        conn,
        &format!("SELECT {len}(k), k, {len}(b), b FROM {qualified} ORDER BY k"),
    );
    let r = &check.results[0];
    let rows: Vec<Vec<String>> = r
        .raw_rows()
        .map(|row| {
            row.iter()
                .zip(&r.columns)
                .map(|(c, col)| c.display(col.logical).into_owned())
                .collect()
        })
        .collect();
    let row = |a: &str, b: &str, c: &str, d: &str| -> Vec<String> {
        [a, b, c, d].iter().map(|s| s.to_string()).collect()
    };
    assert_eq!(
        rows,
        vec![
            row("2", "0x0102", "4", "0xdeadbeee"),
            row("2", "0x0a0b", "0", "0x"),
            row("2", "0xff00", "1", "0x00"),
        ],
        "{engine:?}"
    );
}

#[test]
fn sqlite_binary_edits_write_bytes() {
    let s = svc();
    let conn = s.connect(
        ConnectionParams::Sqlite(SqliteParams {
            path: s.dir.path().join("b.db").to_string_lossy().into(),
            create: true,
        }),
        None,
    );
    s.run(
        conn,
        "CREATE TABLE blobs (k BLOB PRIMARY KEY, b BLOB);
         INSERT INTO blobs VALUES (X'0102', X'DEADBEEF'), (X'0A0B', X'01'), (X'0C0D', NULL);
         SELECT 1",
    );
    exercise_binary(&s, conn, Engine::Sqlite, None, "blobs");
    let types = s.run(conn, "SELECT DISTINCT typeof(b) FROM blobs");
    assert_eq!(
        types.results[0].raw_row(0),
        &[Cell::Text("blob".into())],
        "stored as blobs, not text"
    );
}

#[test]
fn sqlite_edits_apply() {
    let s = svc();
    let conn = s.connect(
        ConnectionParams::Sqlite(SqliteParams {
            path: s.dir.path().join("e.db").to_string_lossy().into(),
            create: true,
        }),
        None,
    );
    s.run(
        conn,
        "CREATE TABLE products (id INTEGER PRIMARY KEY, name TEXT, price NUMERIC);
         INSERT INTO products VALUES (1, 'a', 1.5), (2, 'b', 2.5), (3, 'c', 3.5);
         SELECT 1",
    );
    exercise(&s, conn, Engine::Sqlite, None, "products");
}

fn postgres(s: &Svc) -> Uuid {
    s.connect(
        ConnectionParams::Postgres(PostgresParams {
            host: "127.0.0.1".into(),
            port: 55432,
            database: "sqail_test".into(),
            user: "sqail".into(),
            ssl_mode: PgSslMode::Disable,
            ssl_root_cert: None,
            ssl_client_cert: None,
        }),
        Some("sqail_dev_pw"),
    )
}

fn mssql(s: &Svc) -> Uuid {
    mssql_as(s, "sqail")
}

/// `sa` may create tables in `tempdb`, a second database to `USE`.
fn mssql_as(s: &Svc, user: &str) -> Uuid {
    s.connect(
        ConnectionParams::Mssql(MssqlParams {
            host: "127.0.0.1".into(),
            port: 51433,
            instance: None,
            database: Some("sqail_test".into()),
            auth: MssqlAuth::Sql { user: user.into() },
            encrypt: MssqlEncrypt::Required,
            trust_server_certificate: true,
        }),
        Some("Sqail2_dev!Passw0rd"),
    )
}

#[test]
#[ignore = "needs podman test databases (scripts/db.sh up)"]
fn edits_apply_on_postgres_and_mssql() {
    let s = svc();
    let t = format!("it_edit_{}", &Uuid::new_v4().simple().to_string()[..8]);
    let pg = postgres(&s);
    s.run(
        pg,
        &format!(
            "CREATE TABLE sales.{t} (id int PRIMARY KEY, name text, price numeric(10,2));
         INSERT INTO sales.{t} VALUES (1, 'a', 1.5), (2, 'b', 2.5), (3, 'c', 3.5); SELECT 1"
        ),
    );
    exercise(&s, pg, Engine::Postgres, Some("sales"), &t);
    s.run(pg, &format!("DROP TABLE sales.{t}; SELECT 1"));
    s.run(
        pg,
        &format!(
            "CREATE TABLE sales.{t} (k bytea PRIMARY KEY, b bytea);
         INSERT INTO sales.{t} VALUES ('\\x0102', '\\xdeadbeef'), ('\\x0a0b', '\\x01'), ('\\x0c0d', NULL);
         SELECT 1"
        ),
    );
    exercise_binary(&s, pg, Engine::Postgres, Some("sales"), &t);
    s.run(pg, &format!("DROP TABLE sales.{t}; SELECT 1"));

    let ms = mssql(&s);
    s.run(
        ms,
        &format!(
            "CREATE TABLE sales.{t} (id int PRIMARY KEY, name nvarchar(50), price decimal(10,2));
         INSERT INTO sales.{t} VALUES (1, 'a', 1.5), (2, 'b', 2.5), (3, 'c', 3.5); SELECT 1"
        ),
    );
    exercise(&s, ms, Engine::Mssql, Some("sales"), &t);
    s.run(ms, &format!("DROP TABLE sales.{t}; SELECT 1"));
    // binary(2): a fixed-length key compared with the varbinary parameter.
    s.run(
        ms,
        &format!(
            "CREATE TABLE sales.{t} (k binary(2) PRIMARY KEY, b varbinary(max));
         INSERT INTO sales.{t} VALUES (0x0102, 0xdeadbeef), (0x0a0b, 0x01), (0x0c0d, NULL);
         SELECT 1"
        ),
    );
    exercise_binary(&s, ms, Engine::Mssql, Some("sales"), &t);
    s.run(ms, &format!("DROP TABLE sales.{t}; SELECT 1"));
}

/// A table the tab's session sees under the same unqualified name as another
/// one: edits must hit the session's table (#162). On SQLite a temp table
/// shadows `main.products`; its columns differ, so the lookup must be on the
/// session too.
#[test]
fn sqlite_edits_hit_the_table_the_session_sees() {
    let s = svc();
    let conn = s.connect(
        ConnectionParams::Sqlite(SqliteParams {
            path: s.dir.path().join("shadow.db").to_string_lossy().into(),
            create: true,
        }),
        None,
    );
    s.run(
        conn,
        "CREATE TABLE products (id INTEGER PRIMARY KEY, name TEXT);
         INSERT INTO products VALUES (1, 'main'); SELECT 1",
    );
    let session = s.session(conn);
    let tab = On::Session(session);
    s.run_on(
        tab,
        "CREATE TEMP TABLE products (id INTEGER PRIMARY KEY, label TEXT);
         INSERT INTO products VALUES (1, 'temp'); SELECT 1",
    );
    let select = "SELECT id, label FROM products";
    let (run, mut e) = s.edit(session, Engine::Sqlite, select);
    e.set(0, 1, Some("edited".into()), &run.results[0]);
    s.apply(session, Engine::Sqlite, &e, &run, EditTx::Commit)
        .unwrap();
    assert_eq!(
        s.value(tab, "SELECT label FROM temp.products WHERE id = 1"),
        "edited"
    );
    assert_eq!(
        s.value(
            On::Connection(conn),
            "SELECT name FROM main.products WHERE id = 1"
        ),
        "main",
        "the other table is untouched"
    );
}

/// In a manual-commit tab the edits become part of its transaction: they see
/// its uncommitted rows and don't wait on its locks (#81), stay uncommitted,
/// and a batch that fails undoes only itself. `table` holds ids 1-3 named
/// a, b, c.
fn edits_join_the_open_transaction(s: &Svc, conn: Uuid, engine: Engine, table: &str) {
    let begin = if engine == Engine::Mssql {
        "BEGIN TRANSACTION"
    } else {
        "BEGIN"
    };
    let session = s.session(conn);
    let tab = On::Session(session);
    let select = format!("SELECT id, name FROM {table} ORDER BY id");
    let name_of = |id: i32| s.value(tab, &format!("SELECT name FROM {table} WHERE id = {id}"));
    let count = |on: On| s.value(on, &format!("SELECT count(*) FROM {table}"));

    // Manual commit, no transaction open yet: the edits open one.
    let (run, mut e) = s.edit(session, engine, &select);
    e.set(0, 1, Some("begun".into()), &run.results[0]);
    s.apply(session, engine, &e, &run, EditTx::Begin).unwrap();
    assert!(s.in_transaction(session), "left open");
    assert_eq!(name_of(1), "begun");
    s.run_on(tab, "ROLLBACK");
    assert_eq!(name_of(1), "a", "rolled back with the transaction");

    // An open transaction holding a row lock, and an uncommitted row.
    s.run_on(tab, begin);
    s.run_on(
        tab,
        &format!("UPDATE {table} SET name = 'locked' WHERE id = 1"),
    );
    s.run_on(
        tab,
        &format!("INSERT INTO {table} (id, name) VALUES (4, 'new')"),
    );
    let (run, mut e) = s.edit(session, engine, &select);
    let rs = &run.results[0];
    e.set(0, 1, Some("edited".into()), rs); // id 1, locked by the tab itself
    e.set(3, 1, Some("seen".into()), rs); // id 4, not committed
    assert_eq!(s.apply(session, engine, &e, &run, EditTx::Join).unwrap(), 2);
    assert!(s.in_transaction(session), "not committed");
    assert_eq!(name_of(1), "edited");
    assert_eq!(name_of(4), "seen");

    // A batch that fails half-way undoes itself and nothing else.
    let (stale_run, mut stale) = s.edit(session, engine, &select);
    s.run_on(tab, &format!("DELETE FROM {table} WHERE id = 3"));
    stale.deleted.insert(1); // id 2: works
    stale.set(2, 1, Some("gone".into()), &stale_run.results[0]); // id 3: deleted
    let err = s
        .apply(session, engine, &stale, &stale_run, EditTx::Join)
        .unwrap_err();
    assert!(format!("{err:#}").contains("matched no row"), "{err:#}");
    assert!(s.in_transaction(session), "the transaction survives");
    assert_eq!(count(tab), "3", "id 2 is back, id 3 stays deleted");
    assert_eq!(name_of(4), "seen");

    s.run_on(tab, "ROLLBACK");
    assert!(!s.in_transaction(session));
    assert_eq!(count(On::Connection(conn)), "3");
    assert_eq!(name_of(1), "a");
}

#[test]
fn sqlite_edits_join_the_open_transaction() {
    let s = svc();
    let conn = s.connect(
        ConnectionParams::Sqlite(SqliteParams {
            path: s.dir.path().join("tx.db").to_string_lossy().into(),
            create: true,
        }),
        None,
    );
    s.run(
        conn,
        "CREATE TABLE products (id INTEGER PRIMARY KEY, name TEXT);
         INSERT INTO products VALUES (1, 'a'), (2, 'b'), (3, 'c'); SELECT 1",
    );
    edits_join_the_open_transaction(&s, conn, Engine::Sqlite, "products");
}

/// The tab changed its search path (PostgreSQL) or database (SQL Server):
/// edits hit that table, not the same-named one elsewhere.
#[test]
#[ignore = "needs podman test databases (scripts/db.sh up)"]
fn edits_follow_the_sessions_schema_and_database() {
    let s = svc();
    let id = &Uuid::new_v4().simple().to_string()[..8];
    let (t, sc) = (format!("it_edit_t_{id}"), format!("it_edit_s_{id}"));

    let pg = postgres(&s);
    s.run(
        pg,
        &format!(
            "CREATE SCHEMA {sc};
             CREATE TABLE {sc}.{t} (id int PRIMARY KEY, name text);
             CREATE TABLE sales.{t} (id int PRIMARY KEY, name text);
             INSERT INTO {sc}.{t} VALUES (1, 'a'), (2, 'b'), (3, 'c');
             INSERT INTO sales.{t} VALUES (1, 'a'), (2, 'b'), (3, 'c'); SELECT 1"
        ),
    );
    for (path, other) in [(sc.as_str(), "sales"), ("sales", sc.as_str())] {
        let session = s.session(pg);
        s.run_on(On::Session(session), &format!("SET search_path TO {path}"));
        let select = format!("SELECT id, name FROM {t}");
        let (run, mut e) = s.edit(session, Engine::Postgres, &select);
        e.set(0, 1, Some(format!("in {path}")), &run.results[0]);
        s.apply(session, Engine::Postgres, &e, &run, EditTx::Commit)
            .unwrap();
        let name = |schema: &str| {
            s.value(
                On::Connection(pg),
                &format!("SELECT name FROM {schema}.{t} WHERE id = 1"),
            )
        };
        assert_eq!(name(path), format!("in {path}"));
        assert_eq!(name(other), "a", "wrong table edited");
        s.run(pg, &format!("UPDATE {path}.{t} SET name = 'a'; SELECT 1"));
    }
    edits_join_the_open_transaction(&s, pg, Engine::Postgres, &format!("{sc}.{t}"));
    s.run(
        pg,
        &format!("DROP SCHEMA {sc} CASCADE; DROP TABLE sales.{t}; SELECT 1"),
    );

    let ms = mssql_as(&s, "sa");
    s.run(
        ms,
        &format!(
            "CREATE TABLE dbo.{t} (id int PRIMARY KEY, name nvarchar(50));
             INSERT INTO dbo.{t} VALUES (1, 'a'), (2, 'b'), (3, 'c');
             CREATE TABLE tempdb.dbo.{t} (id int PRIMARY KEY, label nvarchar(50));
             INSERT INTO tempdb.dbo.{t} VALUES (1, 'a'); SELECT 1"
        ),
    );
    let session = s.session(ms);
    s.run_on(On::Session(session), "USE tempdb");
    let select = format!("SELECT id, label FROM {t}");
    let (run, mut e) = s.edit(session, Engine::Mssql, &select);
    e.set(0, 1, Some("in tempdb".into()), &run.results[0]);
    s.apply(session, Engine::Mssql, &e, &run, EditTx::Commit)
        .unwrap();
    let value = |sql: String| s.value(On::Connection(ms), &sql);
    assert_eq!(
        value(format!("SELECT label FROM tempdb.dbo.{t} WHERE id = 1")),
        "in tempdb"
    );
    assert_eq!(
        value(format!("SELECT name FROM dbo.{t} WHERE id = 1")),
        "a",
        "wrong table edited"
    );
    edits_join_the_open_transaction(&s, ms, Engine::Mssql, &format!("dbo.{t}"));
    s.run(
        ms,
        &format!("DROP TABLE tempdb.dbo.{t}; DROP TABLE dbo.{t}; SELECT 1"),
    );
}
