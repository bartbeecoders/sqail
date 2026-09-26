//! sqail-service command line.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use sqail_proto::Scope;
use sqail_service::store::{AuditEvent, Store};
use sqail_service::{Config, tls};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::writer::BoxMakeWriter;

#[cfg(windows)]
mod winsvc;

#[derive(Parser)]
#[command(
    name = "sqail-service",
    version,
    about = "HTTPS REST gateway between sqail2 and SQL databases"
)]
struct Cli {
    /// Data directory (service.db, master.key, dev certificate).
    #[arg(long, global = true, env = "SQAIL_DATA_DIR")]
    data_dir: Option<PathBuf>,
    /// Config file (default: <data-dir>/sqail-service.toml if present).
    #[arg(long, global = true, env = "SQAIL_CONFIG")]
    config: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Run the server (default).
    Serve {
        /// Do not create an admin token when none is active (for callers
        /// that provision tokens with `token create`).
        #[arg(long)]
        no_bootstrap_token: bool,
    },
    /// Manage API tokens directly in service.db.
    Token {
        #[command(subcommand)]
        command: TokenCommand,
    },
    /// Print the TLS certificate fingerprint clients should pin.
    Fingerprint,
    /// Write a consistent copy of service.db (safe while running). PATH is
    /// a new file, or a folder to create service-<timestamp>.db in.
    /// Back up master.key separately: without it stored passwords are lost.
    Backup { path: PathBuf },
    /// Install, remove or run as a Windows service.
    #[cfg(windows)]
    Service {
        #[command(subcommand)]
        command: winsvc::ServiceCommand,
    },
}

#[derive(Subcommand)]
enum TokenCommand {
    /// Create a token and print its secret (shown once).
    Create {
        #[arg(long)]
        name: String,
        /// read, query or admin.
        #[arg(long, default_value = "query")]
        scope: Scope,
    },
    List,
    Revoke {
        id: uuid::Uuid,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let command = cli.command.unwrap_or(Command::Serve {
        no_bootstrap_token: false,
    });
    #[cfg(windows)]
    let command = match command {
        // Runs before a runtime exists: the SCM dispatcher owns this thread.
        Command::Service { command } => return winsvc::main(command, cli.data_dir, cli.config),
        other => other,
    };
    let mut config = Config::load(cli.data_dir, cli.config)?;
    init_tracing(
        &config,
        BoxMakeWriter::new(std::io::stderr),
        std::io::IsTerminal::is_terminal(&std::io::stderr()),
    );
    let rt = tokio::runtime::Runtime::new()?;

    match command {
        Command::Serve { no_bootstrap_token } => {
            config.bootstrap_admin_token = !no_bootstrap_token;
            rt.block_on(serve(config, shutdown_signal()))
        }
        Command::Fingerprint => {
            println!("{}", tls::load(&config)?.fingerprint);
            Ok(())
        }
        Command::Token { command } => token(&config, command),
        Command::Backup { path } => {
            let file = if path.is_dir() {
                path.join(format!(
                    "service-{}.db",
                    chrono::Utc::now().format("%Y%m%d-%H%M%S")
                ))
            } else {
                path
            };
            let store = Store::open(&config.db_path()).context("opening service.db")?;
            store.backup_to(&file)?;
            println!("wrote {}", file.display());
            println!(
                "master.key is not included: keep a copy of {} (or SQAIL_MASTER_KEY) somewhere else",
                config.data_dir.join("master.key").display()
            );
            Ok(())
        }
        #[cfg(windows)]
        Command::Service { .. } => unreachable!("handled above"),
    }
}

/// Run the server until `shutdown` resolves.
pub(crate) async fn serve(config: Config, shutdown: impl Future<Output = ()>) -> Result<()> {
    let data_dir = config.data_dir.clone();
    let sqlite_dirs = config.sqlite.allowed_dirs.clone();
    let server = sqail_service::start(config).await?;
    tracing::info!(
        addr = %server.addr,
        data_dir = %data_dir.display(),
        fingerprint = %server.fingerprint,
        "sqail-service {} listening on https://{}",
        env!("CARGO_PKG_VERSION"),
        server.addr
    );
    if sqlite_dirs.is_empty() {
        tracing::info!("SQLite disabled (set sqlite.allowed_dirs or SQAIL_SQLITE_DIRS to enable)");
    }
    if let Some(token) = &server.bootstrap_token {
        show_bootstrap_token(&data_dir, token)?;
    }
    shutdown.await;
    tracing::info!("shutting down");
    server.shutdown();
    server.wait().await
}

/// Hand the first-start admin token to the operator without logging it: on
/// an interactive terminal it is printed; under a service manager (where
/// stderr ends up in the journal or nowhere) it goes to a private file.
fn show_bootstrap_token(data_dir: &std::path::Path, token: &str) -> Result<()> {
    use std::io::IsTerminal;
    if std::io::stderr().is_terminal() {
        eprintln!(
            "\n  First start: created an admin token. Store it now; it is not shown again.\n\n    {token}\n"
        );
    } else {
        let path = data_dir.join(BOOTSTRAP_TOKEN_FILE);
        let _ = std::fs::remove_file(&path);
        sqail_service::crypto::write_private_file(&path, format!("{token}\n").as_bytes())?;
        tracing::warn!(
            path = %path.display(),
            "first start: admin token written to this file; store it and delete the file"
        );
    }
    Ok(())
}

const BOOTSTRAP_TOKEN_FILE: &str = "bootstrap-admin-token.txt";

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        if let Ok(mut s) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            s.recv().await;
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {}
        _ = term => {}
    }
}

fn token(config: &Config, cmd: TokenCommand) -> Result<()> {
    let store = Store::open(&config.db_path()).context("opening service.db")?;
    match cmd {
        TokenCommand::Create { name, scope } => {
            let created = store.token_create(&name, scope)?;
            store.audit(AuditEvent {
                actor: "cli",
                action: "token.create",
                target: Some(created.info.id.to_string()),
                detail: Some(format!("{name} ({})", scope.as_str())),
                duration_ms: None,
                success: true,
            });
            println!("{}", created.token);
        }
        TokenCommand::List => {
            for t in store.token_list()? {
                println!(
                    "{}  {:<6} {:<8} {}{}",
                    t.id,
                    t.scope.as_str(),
                    t.last_used_at
                        .map(|d| d.format("%Y-%m-%d").to_string())
                        .unwrap_or_else(|| "never".into()),
                    t.name,
                    if t.revoked { "  (revoked)" } else { "" }
                );
            }
        }
        TokenCommand::Revoke { id } => {
            if store.token_revoke(id)? {
                store.audit(AuditEvent {
                    actor: "cli",
                    action: "token.revoke",
                    target: Some(id.to_string()),
                    detail: None,
                    duration_ms: None,
                    success: true,
                });
                println!("revoked {id}");
            } else {
                anyhow::bail!("no active token {id}");
            }
        }
    }
    Ok(())
}

pub(crate) fn init_tracing(config: &Config, writer: BoxMakeWriter, ansi: bool) {
    let filter = EnvFilter::try_from_env("SQAIL_LOG").unwrap_or_else(|_| {
        EnvFilter::new("info,tower_http=info,tiberius=warn,tokio_postgres=warn")
    });
    let builder = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(ansi)
        .with_writer(writer);
    if config.log_format == "json" {
        builder.json().init();
    } else {
        builder.init();
    }
}
