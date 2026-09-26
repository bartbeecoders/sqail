//! Windows service mode: `sqail-service service install|uninstall|run`.
//!
//! `install` registers an auto-start service running as `LocalService` with
//! its data in `%ProgramData%\sqail2\service` (ACL'd to SYSTEM,
//! Administrators and LocalService only) and prints the first admin token.
//! The SCM then launches `sqail-service service run --data-dir <dir>`, which
//! logs to `<data-dir>\logs\sqail-service.log`.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::Subcommand;
use sqail_proto::Scope;
use sqail_service::Config;
use sqail_service::store::{AuditEvent, Store};
use tracing_subscriber::fmt::writer::BoxMakeWriter;
use windows_service::service::{
    ServiceAccess, ServiceControl, ServiceControlAccept, ServiceErrorControl, ServiceExitCode,
    ServiceInfo, ServiceStartType, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
use windows_service::{define_windows_service, service_dispatcher};

const NAME: &str = "sqail-service";
const DISPLAY_NAME: &str = "sqail2 database gateway";
const DESCRIPTION: &str = "HTTPS REST gateway between sqail2 and SQL databases.";

#[derive(Subcommand)]
pub enum ServiceCommand {
    /// Register and start the Windows service (run as Administrator).
    Install {
        /// Do not start the service after installing it.
        #[arg(long)]
        no_start: bool,
    },
    /// Stop and remove the Windows service. The data directory is kept.
    Uninstall,
    /// Entry point used by the service control manager.
    #[command(hide = true)]
    Run,
}

pub fn main(cmd: ServiceCommand, data_dir: Option<PathBuf>, config: Option<PathBuf>) -> Result<()> {
    match cmd {
        ServiceCommand::Install { no_start } => install(data_dir, config, !no_start),
        ServiceCommand::Uninstall => uninstall(),
        ServiceCommand::Run => {
            // The dispatcher calls back on its own thread without arguments,
            // so the resolved paths travel through a static.
            *ARGS.lock().expect("args lock") = Some((data_dir, config));
            service_dispatcher::start(NAME, ffi_service_main)
                .context("not started by the service control manager; use `serve` instead")
        }
    }
}

type RunArgs = (Option<PathBuf>, Option<PathBuf>);
static ARGS: Mutex<Option<RunArgs>> = Mutex::new(None);

fn default_data_dir() -> PathBuf {
    let base = std::env::var_os("ProgramData")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"));
    base.join("sqail2").join("service")
}

fn install(data_dir: Option<PathBuf>, config: Option<PathBuf>, start: bool) -> Result<()> {
    let data_dir = data_dir.unwrap_or_else(default_data_dir);
    std::fs::create_dir_all(&data_dir)
        .with_context(|| format!("creating {}", data_dir.display()))?;
    restrict_acl(&data_dir)?;

    // Create the first admin token here, on the operator's console, so the
    // service itself never has to hand one out.
    let cfg = Config::load(Some(data_dir.clone()), config.clone())?;
    let store = Store::open(&cfg.db_path()).context("opening service.db")?;
    let token = if store.active_admin_tokens()? == 0 {
        let created = store.token_create("windows-service-install", Scope::Admin)?;
        store.audit(AuditEvent {
            actor: "cli",
            action: "token.create",
            target: Some(created.info.id.to_string()),
            detail: Some("admin token created by service install".into()),
            duration_ms: None,
            success: true,
        });
        Some(created.token)
    } else {
        None
    };
    drop(store);

    let mut launch_arguments = vec![
        OsString::from("--data-dir"),
        data_dir.clone().into_os_string(),
    ];
    if let Some(c) = &config {
        launch_arguments.push("--config".into());
        launch_arguments.push(std::path::absolute(c)?.into_os_string());
    }
    launch_arguments.push("service".into());
    launch_arguments.push("run".into());

    let manager = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
    )
    .context("opening the service manager (run as Administrator)")?;
    let info = ServiceInfo {
        name: NAME.into(),
        display_name: DISPLAY_NAME.into(),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: std::env::current_exe()?,
        launch_arguments,
        dependencies: vec![],
        account_name: Some(r"NT AUTHORITY\LocalService".into()),
        account_password: None,
    };
    let service = manager
        .create_service(
            &info,
            ServiceAccess::CHANGE_CONFIG | ServiceAccess::START | ServiceAccess::QUERY_STATUS,
        )
        .context("creating the service (is it already installed?)")?;
    service.set_description(DESCRIPTION)?;
    println!("installed service `{NAME}` (data: {})", data_dir.display());

    if start {
        service.start::<&str>(&[]).context("starting the service")?;
        println!("started; listening on https://{}", cfg.bind);
    }
    if let Some(token) = token {
        println!(
            "\n  Admin token (shown once; store it now):\n\n    {token}\n\n  \
             Pin the certificate with:  sqail-service --data-dir \"{}\" fingerprint\n",
            data_dir.display()
        );
    }
    Ok(())
}

/// Replace inherited permissions (ProgramData grants Users read access)
/// with full control for SYSTEM and Administrators and modify for the
/// LocalService account the service runs as. SIDs keep this locale-proof.
fn restrict_acl(dir: &Path) -> Result<()> {
    let status = std::process::Command::new("icacls")
        .arg(dir)
        .args([
            "/inheritance:r",
            "/grant:r",
            "*S-1-5-18:(OI)(CI)F",
            "/grant:r",
            "*S-1-5-32-544:(OI)(CI)F",
            "/grant:r",
            "*S-1-5-19:(OI)(CI)M",
            "/T",
            "/Q",
        ])
        .status()
        .context("running icacls")?;
    if !status.success() {
        bail!("icacls could not restrict {}", dir.display());
    }
    Ok(())
}

fn uninstall() -> Result<()> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .context("opening the service manager (run as Administrator)")?;
    let service = manager
        .open_service(
            NAME,
            ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE,
        )
        .context("opening the service (is it installed?)")?;
    if service.query_status()?.current_state != ServiceState::Stopped {
        service.stop()?;
        for _ in 0..30 {
            std::thread::sleep(Duration::from_millis(500));
            if service.query_status()?.current_state == ServiceState::Stopped {
                break;
            }
        }
    }
    service.delete()?;
    println!("removed service `{NAME}`; the data directory was kept");
    Ok(())
}

define_windows_service!(ffi_service_main, service_main);

fn service_main(_args: Vec<OsString>) {
    if let Err(e) = run_service() {
        // Tracing may not be up yet; the event log gets the exit code.
        tracing::error!(error = %format!("{e:#}"), "service failed");
    }
}

fn run_service() -> Result<()> {
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let tx = Mutex::new(Some(tx));
    let handle = service_control_handler::register(NAME, move |control| match control {
        ServiceControl::Stop | ServiceControl::Shutdown => {
            if let Some(tx) = tx.lock().ok().and_then(|mut t| t.take()) {
                let _ = tx.send(());
            }
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    })?;
    let status = |state, exit| ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: state,
        controls_accepted: if state == ServiceState::Running {
            ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN
        } else {
            ServiceControlAccept::empty()
        },
        exit_code: ServiceExitCode::Win32(exit),
        checkpoint: 0,
        wait_hint: Duration::from_secs(15),
        process_id: None,
    };
    handle.set_service_status(status(ServiceState::StartPending, 0))?;

    let result = (|| -> Result<()> {
        let (data_dir, config) = ARGS.lock().expect("args lock").take().unwrap_or_default();
        let config = Config::load(data_dir, config)?;
        let log_dir = config.data_dir.join("logs");
        std::fs::create_dir_all(&log_dir)?;
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_dir.join("sqail-service.log"))?;
        crate::init_tracing(&config, BoxMakeWriter::new(Mutex::new(log)), false);

        let rt = tokio::runtime::Runtime::new()?;
        handle.set_service_status(status(ServiceState::Running, 0))?;
        rt.block_on(crate::serve(config, async {
            let _ = rx.await;
        }))
    })();

    let exit = if result.is_ok() { 0 } else { 1 };
    handle.set_service_status(status(ServiceState::Stopped, exit))?;
    result
}
