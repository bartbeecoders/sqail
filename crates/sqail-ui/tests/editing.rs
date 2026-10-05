//! Inline edits applied through a real service on every engine.

use std::path::PathBuf;

use sqail_client::proto::{
    ConnectionInput, ConnectionParams, Engine, MssqlAuth, MssqlEncrypt, MssqlParams, PgSslMode,
    PostgresParams, QueryEvent, QueryRequest, SqliteParams,
};
use sqail_client::{Client, On, Target, Trust};
use sqail_ui::editing::{self, EditState};
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
        let events = self
            .rt
            .block_on(
                self.client
                    .query_all(On::Connection(conn), &QueryRequest::new(sql)),
            )
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
}

/// Price edit, a new row, a deleted row; then a stale edit that must fail.
fn exercise(s: &Svc, conn: Uuid, engine: Engine, schema: Option<&str>, table: &str) {
    let q = |t: &str| sqail_ui::sql::quote_ident(engine, t);
    let qualified = match schema {
        Some(sc) => format!("{}.{}", q(sc), q(table)),
        None => q(table),
    };
    let select = format!("SELECT id, name, price FROM {qualified} ORDER BY id");
    let run = s.run(conn, &select);
    let rs = &run.results[0];
    let (sch, tbl) = editing::source_table(&select, engine).unwrap();
    let cols =
        s.rt.block_on(s.client.columns(conn, sch.as_deref(), &tbl))
            .unwrap();
    let mut e = EditState::new(engine, sch, tbl, &rs.columns, &cols).unwrap();
    e.set(0, 2, Some("12.34".into()), rs); // row id 1
    e.set(0, 1, None, rs); // name → NULL
    e.deleted.insert(1); // row id 2
    e.inserted.push(vec![
        Some(Some("100".into())),
        Some(Some("it's new".into())),
        Some(Some("0.5".into())),
    ]);
    let n =
        s.rt.block_on(editing::apply(&s.client, conn, engine, e.statements(rs)))
            .unwrap();
    assert_eq!(n, 3);

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
    let mut stale = EditState::new(
        engine,
        e.schema.clone(),
        e.table.clone(),
        &rs.columns,
        &cols,
    )
    .unwrap();
    stale.set(1, 2, Some("1".into()), rs); // id 2, deleted above
    stale.set(0, 2, Some("99".into()), rs);
    let err =
        s.rt.block_on(editing::apply(
            &s.client,
            conn,
            engine,
            stale.statements(rs),
        ))
        .unwrap_err();
    assert!(format!("{err:#}").contains("matched no row"), "{err:#}");
    let again = s.run(conn, &select);
    assert_eq!(
        again.results[0].raw_row(0)[2].display(again.results[0].columns[2].logical),
        "12.34",
        "rolled back"
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

#[test]
#[ignore = "needs podman test databases (scripts/db.sh up)"]
fn edits_apply_on_postgres_and_mssql() {
    let s = svc();
    let t = format!("it_edit_{}", &Uuid::new_v4().simple().to_string()[..8]);
    let pg = s.connect(
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
    );
    s.run(
        pg,
        &format!(
            "CREATE TABLE sales.{t} (id int PRIMARY KEY, name text, price numeric(10,2));
         INSERT INTO sales.{t} VALUES (1, 'a', 1.5), (2, 'b', 2.5), (3, 'c', 3.5); SELECT 1"
        ),
    );
    exercise(&s, pg, Engine::Postgres, Some("sales"), &t);
    s.run(pg, &format!("DROP TABLE sales.{t}; SELECT 1"));

    let ms = s.connect(
        ConnectionParams::Mssql(MssqlParams {
            host: "127.0.0.1".into(),
            port: 51433,
            instance: None,
            database: Some("sqail_test".into()),
            auth: MssqlAuth::Sql {
                user: "sqail".into(),
            },
            encrypt: MssqlEncrypt::Required,
            trust_server_certificate: true,
        }),
        Some("Sqail2_dev!Passw0rd"),
    );
    s.run(
        ms,
        &format!(
            "CREATE TABLE sales.{t} (id int PRIMARY KEY, name nvarchar(50), price decimal(10,2));
         INSERT INTO sales.{t} VALUES (1, 'a', 1.5), (2, 'b', 2.5), (3, 'c', 3.5); SELECT 1"
        ),
    );
    exercise(&s, ms, Engine::Mssql, Some("sales"), &t);
    s.run(ms, &format!("DROP TABLE sales.{t}; SELECT 1"));
}
