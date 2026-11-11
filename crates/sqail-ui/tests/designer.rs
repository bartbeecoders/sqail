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
use sqail_ui::sql::drop::{DropItem, ObjectKind, Spot, text_for};
use sqail_ui::sql::format::Style;
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
        Engine::Sqlite | Engine::Unknown => ("INTEGER", "TEXT"),
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
        Engine::Sqlite | Engine::Unknown => "NUMERIC",
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

/// SQL Server: an index with a descending key, included columns and a
/// filter keeps all of them when a column it includes is dropped or its key
/// column is retyped, the nonclustered key stays nonclustered, and the
/// script for two changed defaults runs as one batch.
#[test]
#[ignore = "needs podman test databases (scripts/db.sh up)"]
fn mssql_designer_keeps_index_details() {
    let s = svc();
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
    let e = Engine::Mssql;
    let schema = format!("it172_{}", &Uuid::new_v4().simple().to_string()[..8]);
    s.rows(ms, &format!("CREATE SCHEMA {schema}"));
    s.rows(
        ms,
        &format!(
            "CREATE TABLE {schema}.parent (
                id int CONSTRAINT pk_{schema} PRIMARY KEY NONCLUSTERED,
                code nvarchar(20) NOT NULL DEFAULT N'', a int DEFAULT 0, b int DEFAULT 1,
                inc nvarchar(50), inc2 nvarchar(10));
             CREATE INDEX ix_code ON {schema}.parent (code DESC) INCLUDE (inc, inc2) WHERE a > 0;"
        ),
    );
    let facts = || {
        s.rows(
            ms,
            &format!(
                "SELECT i.name, i.type_desc, CAST(i.has_filter AS int), i.filter_definition,
                        (SELECT STRING_AGG(CONCAT(c.name, IIF(x.is_descending_key = 1, ' DESC', ''),
                                                  IIF(x.is_included_column = 1, ' included', '')), ', ')
                                WITHIN GROUP (ORDER BY x.index_column_id)
                         FROM sys.index_columns x
                         JOIN sys.columns c ON c.object_id = x.object_id AND c.column_id = x.column_id
                         WHERE x.object_id = i.object_id AND x.index_id = i.index_id)
                 FROM sys.indexes i
                 WHERE i.object_id = OBJECT_ID('{schema}.parent') AND i.name IS NOT NULL
                 ORDER BY i.name"
            ),
        )
    };
    let after = |include: &str| {
        vec![
            vec![
                "ix_code".to_string(),
                "NONCLUSTERED".into(),
                "1".into(),
                "([a]>(0))".into(),
                format!("code DESC, {include}"),
            ],
            vec![
                format!("pk_{schema}"),
                "NONCLUSTERED".into(),
                "0".into(),
                "null".into(),
                "id".into(),
            ],
        ]
    };
    assert_eq!(facts(), after("inc included, inc2 included"));

    let mut d = s.load(ms, e, &schema, "parent");
    no_changes(&d);
    d.remove_column(column(&d, "inc"));
    s.apply(ms, e, d.plan().unwrap());
    assert_eq!(facts(), after("inc2 included"));

    let mut d = s.load(ms, e, &schema, "parent");
    no_changes(&d);
    for (c, ty) in [("code", "nvarchar(40)"), ("id", "bigint")] {
        let c = column(&d, c);
        d.column_mut(c).unwrap().data_type = ty.into();
    }
    s.apply(ms, e, d.plan().unwrap());
    assert_eq!(facts(), after("inc2 included"));

    // "Open in editor": the whole script in one batch.
    let mut d = s.load(ms, e, &schema, "parent");
    no_changes(&d);
    for (c, v) in [("a", "5"), ("b", "6")] {
        let c = column(&d, c);
        d.column_mut(c).unwrap().default = v.into();
    }
    s.rows(ms, &d.plan().unwrap().script(e));
    let d = s.load(ms, e, &schema, "parent");
    no_changes(&d);
    let defaults: Vec<&str> = d.table.columns[2..4]
        .iter()
        .map(|c| c.default.as_str())
        .collect();
    assert_eq!(defaults, ["((5))", "((6))"]);

    s.rows(
        ms,
        &format!("DROP TABLE {schema}.parent; DROP SCHEMA {schema};"),
    );
}

/// PostgreSQL: changing an index keeps its sort order, INCLUDE columns and
/// WHERE clause, with a renamed column renamed in the clause.
#[test]
#[ignore = "needs podman test databases (scripts/db.sh up)"]
fn postgres_designer_keeps_index_details() {
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
    let e = Engine::Postgres;
    let schema = format!("it172_{}", &Uuid::new_v4().simple().to_string()[..8]);
    s.rows(
        pg,
        &format!(
            "CREATE SCHEMA {schema};
             CREATE TABLE {schema}.t (id int PRIMARY KEY, code text, a int, inc text);
             CREATE INDEX ix_t ON {schema}.t (code DESC) INCLUDE (inc) WHERE a > 0;"
        ),
    );
    let mut d = s.load(pg, e, &schema, "t");
    no_changes(&d);
    let a = column(&d, "a");
    d.column_mut(a).unwrap().name = "qty".into();
    let ix = d.table.indexes[0].id;
    d.index_mut(ix).unwrap().unique = true;
    s.apply(pg, e, d.plan().unwrap());
    let def = s.rows(
        pg,
        &format!("SELECT pg_get_indexdef('{schema}.ix_t'::regclass)"),
    );
    assert_eq!(
        def[0][0],
        format!(
            "CREATE UNIQUE INDEX ix_t ON {schema}.t USING btree (code DESC) INCLUDE (inc) WHERE (qty > 0)"
        )
    );
    no_changes(&s.load(pg, e, &schema, "t"));
    s.rows(pg, &format!("DROP SCHEMA {schema} CASCADE"));
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

/// A table whose column names the engine reserves, built and reloaded by the
/// designer with a key and an index on them, then queried through the
/// drag-and-drop SELECT: every name must come out quoted and still match.
fn reserved_names(s: &Svc, conn: Uuid, engine: Engine, schema: &str, names: [&str; 3]) {
    let table = format!("rw_{}", &Uuid::new_v4().simple().to_string()[..8]);
    let text = if engine == Engine::Postgres {
        "text"
    } else {
        "TEXT"
    };
    let mut d = Design::new_table(engine, Some(schema));
    d.table.name = table.clone();
    let key = d.table.columns[0].id;
    d.column_mut(key).unwrap().name = names[0].into();
    d.column_mut(key).unwrap().data_type = text.into();
    let mut ids = vec![key];
    for n in &names[1..] {
        let c = d.add_column();
        d.column_mut(c).unwrap().name = (*n).into();
        d.column_mut(c).unwrap().data_type = text.into();
        ids.push(c);
    }
    let ix = d.add_index();
    d.index_mut(ix).unwrap().name = format!("ix_{table}");
    d.index_mut(ix).unwrap().parts = vec![IndexPart::Column(ids[1]), IndexPart::Column(ids[2])];
    s.apply(conn, engine, d.plan().unwrap());

    let d = s.load(conn, engine, schema, &table);
    no_changes(&d);
    assert_eq!(d.table.primary_key.columns, vec![column(&d, names[0])]);
    assert_eq!(
        d.table.indexes[0].parts,
        vec![
            IndexPart::Column(column(&d, names[1])),
            IndexPart::Column(column(&d, names[2]))
        ]
    );

    let q = sqail_ui::sql::qualified_name(engine, Some(schema), &table);
    s.rows(conn, &format!("INSERT INTO {q} VALUES ('alice', 'b', 'c')"));
    let item = DropItem {
        engine,
        schema: schema.into(),
        name: table.clone(),
        kind: ObjectKind::Table,
    };
    let cols: Vec<String> = names.iter().map(|n| n.to_string()).collect();
    let style = Style {
        uppercase: true,
        indent: 2,
    };
    let select = text_for(&item, Spot::Line, Some(&cols), &style);
    assert_eq!(s.rows(conn, &select), [["alice", "b", "c"]], "{select}");
    s.apply(
        conn,
        engine,
        Plan {
            body: vec![drop_table(engine, schema, &table, false)],
            ..Default::default()
        },
    );
}

#[test]
fn sqlite_reserved_words_are_quoted() {
    let s = svc();
    let conn = s.connect(
        ConnectionParams::Sqlite(SqliteParams {
            path: s.dir.path().join("rw.db").to_string_lossy().into(),
            create: true,
        }),
        None,
    );
    let names = ["current_date", "notnull", "raise"];
    reserved_names(&s, conn, Engine::Sqlite, "main", names);
}

/// `PG_MUST_QUOTE` against the server's own keyword list, so a server that
/// reserves more words fails here instead of getting SQL that misreads.
#[test]
#[ignore = "needs podman test databases (scripts/db.sh up)"]
fn postgres_reserved_words_are_quoted() {
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
    let version = s.rows(pg, "SHOW server_version").remove(0).remove(0);
    let reserved: Vec<String> = s
        .rows(
            pg,
            "SELECT word FROM pg_get_keywords() WHERE catcode <> 'U' ORDER BY word",
        )
        .into_iter()
        .map(|mut r| r.remove(0))
        .collect();
    let missing: Vec<&String> = reserved
        .iter()
        .filter(|w| !sqail_ui::sql::pg_must_quote(w))
        .collect();
    assert!(
        missing.is_empty(),
        "PostgreSQL {version} reserves words PG_MUST_QUOTE lacks: {missing:?}"
    );
    let extra: Vec<&&str> = sqail_ui::sql::PG_MUST_QUOTE
        .iter()
        .filter(|w| !reserved.iter().any(|r| r == **w))
        .collect();
    assert!(
        extra.is_empty(),
        "PostgreSQL {version} doesn't reserve {extra:?}: keep them in PG_MUST_QUOTE only \
         if a supported version still does"
    );

    let schema = format!("it170_{}", &Uuid::new_v4().simple().to_string()[..8]);
    s.rows(pg, &format!("CREATE SCHEMA {schema}"));
    let names = ["user", "leading", "position"];
    reserved_names(&s, pg, Engine::Postgres, &schema, names);
    s.rows(pg, &format!("DROP SCHEMA {schema} CASCADE"));
}
