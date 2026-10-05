//! The table designer's DDL applied through a real service on every engine:
//! create a table, reload it (no changes may show), alter it, reload, drop.

use std::path::PathBuf;

use sqail_client::proto::{
    ConnectionInput, ConnectionParams, Engine, MssqlAuth, MssqlEncrypt, MssqlParams, PgSslMode,
    PostgresParams, QueryEvent, QueryRequest, SqliteParams,
};
use sqail_client::{Client, On, Target, Trust};
use sqail_ui::designer::model::{Design, IndexPart, Plan, drop_table};
use sqail_ui::designer::{self};
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

    /// Run SQL and return the first result's rows as display strings.
    fn rows(&self, conn: Uuid, sql: &str) -> Vec<Vec<String>> {
        let events = self
            .rt
            .block_on(
                self.client
                    .query_all(On::Connection(conn), &QueryRequest::new(sql)),
            )
            .unwrap();
        let mut out = Vec::new();
        for e in events {
            match e {
                QueryEvent::Error { message, .. } => panic!("{sql}: {message}"),
                QueryEvent::Rows { index: 0, rows } => out.extend(rows.into_iter().map(|r| {
                    r.into_iter()
                        .map(|v| match v {
                            serde_json::Value::String(s) => s,
                            other => other.to_string(),
                        })
                        .collect()
                })),
                _ => {}
            }
        }
        out
    }

    fn load(&self, conn: Uuid, engine: Engine, schema: &str, table: &str) -> Design {
        self.rt
            .block_on(designer::fetch(&self.client, conn, engine, schema, table))
            .unwrap_or_else(|e| panic!("load {table}: {e}"))
            .design
    }

    fn apply(&self, conn: Uuid, engine: Engine, plan: Plan) {
        let script = plan.script(engine);
        self.rt
            .block_on(designer::apply(&self.client, conn, engine, plan))
            .unwrap_or_else(|e| panic!("{e:#}\n{script}"));
    }
}

fn column(d: &Design, name: &str) -> u64 {
    d.table
        .columns
        .iter()
        .find(|c| c.name == name)
        .unwrap_or_else(|| panic!("no column {name}"))
        .id
}

fn no_changes(d: &Design) {
    let plan = d.plan().unwrap();
    assert!(
        plan.is_empty(),
        "a freshly loaded table shows changes:\n{}",
        plan.script(d.engine)
    );
}

/// `grantee`: a role that exists (Postgres, SQL Server) for the grant checks.
fn exercise(s: &Svc, conn: Uuid, engine: Engine, schema: &str, grantee: Option<&str>) {
    let base = format!("dz_{}", &Uuid::new_v4().simple().to_string()[..8]);
    let (int, text) = match engine {
        Engine::Postgres => ("integer", "text"),
        Engine::Mssql => ("int", "nvarchar(100)"),
        Engine::Sqlite => ("INTEGER", "TEXT"),
    };
    let q = |n: &str| sqail_ui::sql::qualified_name(engine, Some(schema), n);

    // Create: id key, name with an index, qty with a default, and a grant.
    let mut d = Design::new_table(engine, Some(schema));
    d.table.name = base.clone();
    let name = d.add_column();
    d.column_mut(name).unwrap().name = "name".into();
    d.column_mut(name).unwrap().data_type = text.into();
    let qty = d.add_column();
    d.column_mut(qty).unwrap().name = "qty".into();
    d.column_mut(qty).unwrap().data_type = int.into();
    d.column_mut(qty).unwrap().nullable = false;
    d.column_mut(qty).unwrap().default = "0".into();
    let ix = d.add_index();
    d.index_mut(ix).unwrap().name = format!("ix_{base}_name");
    d.index_mut(ix).unwrap().parts = vec![IndexPart::Column(name)];
    if let Some(g) = grantee {
        d.set_grant(g, "SELECT", true);
    }
    s.apply(conn, engine, d.plan().unwrap());
    s.rows(
        conn,
        &format!(
            "INSERT INTO {} (id, name) VALUES (1, 'one'), (2, 'two')",
            q(&base)
        ),
    );

    let mut d = s.load(conn, engine, schema, &base);
    no_changes(&d);
    assert_eq!(d.table.columns.len(), 3);
    assert_eq!(d.table.primary_key.columns, vec![column(&d, "id")]);
    assert_eq!(d.table.indexes.len(), 1);
    if let Some(g) = grantee {
        assert!(d.has_grant(g, "SELECT"), "{:?}", d.table.grants);
    }

    // Alter: rename the table and a column, retype, drop, add, re-key,
    // unique index, change the grants.
    let renamed = format!("{base}_v2");
    d.table.name = renamed.clone();
    let name = column(&d, "name");
    d.column_mut(name).unwrap().name = "title".into();
    let qty = column(&d, "qty");
    d.column_mut(qty).unwrap().data_type = match engine {
        Engine::Postgres => "bigint",
        Engine::Mssql => "bigint",
        Engine::Sqlite => "NUMERIC",
    }
    .into();
    d.column_mut(qty).unwrap().default = "5".into();
    let note = d.add_column();
    d.column_mut(note).unwrap().name = "note".into();
    d.column_mut(note).unwrap().data_type = text.into();
    d.toggle_pk(name); // key becomes (id, title)
    let ix = d.table.indexes[0].id;
    d.index_mut(ix).unwrap().unique = true;
    d.index_mut(ix).unwrap().parts = vec![IndexPart::Column(name), IndexPart::Column(qty)];
    if let Some(g) = grantee {
        d.set_grant(g, "SELECT", false);
        d.set_grant(g, "INSERT", true);
    }
    s.apply(conn, engine, d.plan().unwrap());

    let mut d = s.load(conn, engine, schema, &renamed);
    no_changes(&d);
    let names: Vec<&str> = d.table.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["id", "title", "qty", "note"], "{engine:?}");
    assert_eq!(
        d.table.primary_key.columns,
        vec![column(&d, "id"), column(&d, "title")]
    );
    assert!(d.table.indexes[0].unique);
    assert_eq!(d.table.indexes[0].parts.len(), 2);
    if let Some(g) = grantee {
        assert!(!d.has_grant(g, "SELECT"));
        assert!(d.has_grant(g, "INSERT"));
    }
    let rows = s.rows(
        conn,
        &format!("SELECT id, title, qty FROM {} ORDER BY id", q(&renamed)),
    );
    assert_eq!(rows.len(), 2, "rows survive: {rows:?}");
    assert_eq!(rows[0][1], "one");

    // Drop a column and the index, remove the key.
    let note = column(&d, "note");
    d.remove_column(note);
    let ix = d.table.indexes[0].id;
    d.remove_index(ix);
    d.table.primary_key.columns.clear();
    s.apply(conn, engine, d.plan().unwrap());
    let d = s.load(conn, engine, schema, &renamed);
    no_changes(&d);
    assert_eq!(d.table.columns.len(), 3);
    assert!(d.table.indexes.is_empty());
    assert!(d.table.primary_key.columns.is_empty());

    s.apply(
        conn,
        engine,
        Plan {
            body: vec![drop_table(engine, schema, &renamed, false)],
            ..Default::default()
        },
    );
    let err =
        s.rt.block_on(designer::fetch(&s.client, conn, engine, schema, &renamed))
            .err()
            .expect("dropped");
    assert!(err.contains("not found"), "{err}");
}

#[test]
fn sqlite_table_design_round_trip() {
    let s = svc();
    let conn = s.connect(
        ConnectionParams::Sqlite(SqliteParams {
            path: s.dir.path().join("d.db").to_string_lossy().into(),
            create: true,
        }),
        None,
    );
    exercise(&s, conn, Engine::Sqlite, "main", None);
}

#[test]
#[ignore = "needs podman test databases (scripts/db.sh up)"]
fn table_design_round_trip_on_postgres_and_mssql() {
    let s = svc();
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
    s.rows(
        pg,
        "DO $$ BEGIN
           IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'sqail_it_reader') THEN
             CREATE ROLE sqail_it_reader NOLOGIN;
           END IF;
         END $$",
    );
    exercise(&s, pg, Engine::Postgres, "sales", Some("sqail_it_reader"));

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
    s.rows(
        ms,
        "IF DATABASE_PRINCIPAL_ID('sqail_it_reader') IS NULL CREATE USER sqail_it_reader WITHOUT LOGIN",
    );
    exercise(&s, ms, Engine::Mssql, "sales", Some("sqail_it_reader"));
}
