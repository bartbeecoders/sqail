//! The client against a real in-process sqail-service (SQLite only, so no
//! containers are needed).

use futures::StreamExt;
use sqail_client::proto::{
    ConnectionInput, ConnectionParams, QueryEvent, QueryRequest, SqliteParams,
};
use sqail_client::{Client, Error, On, Target, Trust};

struct Env {
    server: sqail_service::Server,
    token: String,
    dir: tempfile::TempDir,
}

async fn env() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = sqail_service::Config {
        data_dir: dir.path().to_path_buf(),
        bind: "127.0.0.1:0".parse().unwrap(),
        docs_ui: false,
        ..Default::default()
    };
    cfg.sqlite.allowed_dirs = vec![dir.path().to_path_buf()];
    let server = sqail_service::start(cfg).await.unwrap();
    let token = server.bootstrap_token.clone().unwrap();
    Env { server, token, dir }
}

impl Env {
    fn url(&self) -> String {
        format!("https://{}", self.server.addr)
    }

    fn client(&self) -> Client {
        Client::new(&Target {
            url: self.url(),
            token: self.token.clone(),
            trust: Trust::Pinned(self.server.fingerprint.clone()),
            identity: None,
        })
        .unwrap()
    }

    async fn sqlite(&self, c: &Client) -> uuid::Uuid {
        c.create_connection(&ConnectionInput {
            name: "t".into(),
            params: ConnectionParams::Sqlite(SqliteParams {
                path: self.dir.path().join("t.db").to_string_lossy().into(),
                create: true,
            }),
            password: None,
            read_only: false,
            color: None,
            environment: None,
            folder: None,
        })
        .await
        .unwrap()
        .id
    }
}

#[tokio::test]
async fn probe_reports_the_served_fingerprint() {
    let e = env().await;
    let (fp, health) = sqail_client::probe(&e.url()).await.unwrap();
    assert_eq!(fp, e.server.fingerprint);
    assert_eq!(health.status, "ok");
}

#[tokio::test]
async fn pinning_rejects_other_certificates() {
    let e = env().await;
    let wrong = Client::new(&Target {
        url: e.url(),
        token: e.token.clone(),
        trust: Trust::Pinned("00:".repeat(31) + "00"),
        identity: None,
    })
    .unwrap();
    assert!(matches!(
        wrong.info().await,
        Err(Error::CertificateMismatch)
    ));
    // System trust also refuses a self-signed certificate.
    let system = Client::new(&Target {
        url: e.url(),
        token: e.token.clone(),
        trust: Trust::System,
        identity: None,
    })
    .unwrap();
    assert!(system.info().await.is_err());
    assert_eq!(
        e.client().info().await.unwrap().scope,
        sqail_client::proto::Scope::Admin
    );
}

#[tokio::test]
async fn bad_token_is_an_auth_error() {
    let e = env().await;
    let c = Client::new(&Target {
        url: e.url(),
        token: "sq2_nope".into(),
        trust: Trust::Pinned(e.server.fingerprint.clone()),
        identity: None,
    })
    .unwrap();
    let err = c.connections().await.unwrap_err();
    assert!(sqail_client::is_auth_error(&err), "{err:?}");
}

#[tokio::test]
async fn streams_query_events() {
    let e = env().await;
    let c = e.client();
    let id = e.sqlite(&c).await;
    let events = c
        .query_all(
            On::Connection(id),
            &QueryRequest::new(
                "WITH RECURSIVE r(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM r LIMIT 1234) SELECT i FROM r",
            ),
        )
        .await
        .unwrap();
    let rows: usize = events
        .iter()
        .map(|e| match e {
            QueryEvent::Rows { rows, .. } => rows.len(),
            _ => 0,
        })
        .sum();
    assert_eq!(rows, 1234);
    assert!(matches!(events.last(), Some(QueryEvent::Done { .. })));
}

#[tokio::test]
async fn cancel_and_sessions() {
    let e = env().await;
    let c = e.client();
    let id = e.sqlite(&c).await;
    let mut q = c
        .query(
            On::Connection(id),
            &QueryRequest::new("WITH RECURSIVE r(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM r) SELECT count(*) FROM r"),
        )
        .await
        .unwrap();
    let c2 = c.clone();
    let qid = q.query_id;
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        c2.cancel(qid).await.unwrap();
    });
    let mut cancelled = false;
    while let Some(ev) = q.events.next().await {
        if let QueryEvent::Done { cancelled: c, .. } = ev.unwrap() {
            cancelled = c;
        }
    }
    assert!(cancelled);

    let s = c.open_session(id).await.unwrap();
    c.query_all(
        On::Session(s.id),
        &QueryRequest::new("CREATE TABLE t (a); BEGIN; INSERT INTO t VALUES (1)"),
    )
    .await
    .unwrap();
    assert!(c.session(s.id).await.unwrap().in_transaction);
    c.close_session(s.id).await.unwrap();
    c.close_session(s.id).await.unwrap(); // idempotent

    let tables = c.tables(id, None).await.unwrap();
    assert!(tables.iter().any(|t| t.name == "t"));
    let cols = c.columns(id, None, "t").await.unwrap();
    assert_eq!(cols[0].name, "a");
}

/// A CA plus a client certificate it signed, as PEM (cert, key).
fn mtls_material(dir: &std::path::Path) -> (std::path::PathBuf, sqail_client::Identity) {
    use rcgen::{
        BasicConstraints, CertificateParams, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair,
    };
    let ca_key = KeyPair::generate().unwrap();
    let mut ca_params = CertificateParams::new(Vec::<String>::new()).unwrap();
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let ca_cert = ca_params.self_signed(&ca_key).unwrap();
    let issuer = Issuer::from_params(&ca_params, &ca_key);
    let client_key = KeyPair::generate().unwrap();
    let mut client_params = CertificateParams::new(vec!["sqail-client".to_string()]).unwrap();
    client_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
    let client_cert = client_params.signed_by(&client_key, &issuer).unwrap();
    let ca_path = dir.join("client-ca.pem");
    std::fs::write(&ca_path, ca_cert.pem()).unwrap();
    (
        ca_path,
        sqail_client::Identity {
            cert_pem: client_cert.pem().into_bytes(),
            key_pem: client_key.serialize_pem().into_bytes(),
        },
    )
}

#[tokio::test]
async fn mutual_tls_requires_a_client_certificate() {
    let dir = tempfile::tempdir().unwrap();
    let (ca, identity) = mtls_material(dir.path());
    let mut cfg = sqail_service::Config {
        data_dir: dir.path().join("svc"),
        bind: "127.0.0.1:0".parse().unwrap(),
        docs_ui: false,
        ..Default::default()
    };
    cfg.tls.client_ca = Some(ca);
    std::fs::create_dir_all(&cfg.data_dir).unwrap();
    let server = sqail_service::start(cfg).await.unwrap();
    let target = |identity| Target {
        url: format!("https://{}", server.addr),
        token: server.bootstrap_token.clone().unwrap(),
        trust: Trust::Pinned(server.fingerprint.clone()),
        identity,
    };
    let without = Client::new(&target(None)).unwrap();
    assert!(
        matches!(without.info().await, Err(Error::Transport(_))),
        "handshake must fail without a certificate"
    );
    let with = Client::new(&target(Some(identity))).unwrap();
    assert_eq!(
        with.info().await.unwrap().scope,
        sqail_client::proto::Scope::Admin
    );
    // A certificate from another CA is refused too.
    let other_dir = dir.path().join("other");
    std::fs::create_dir_all(&other_dir).unwrap();
    let (_, stranger) = mtls_material(&other_dir);
    let other = Client::new(&target(Some(stranger))).unwrap();
    assert!(other.info().await.is_err());
}
