//! sqail: a native SQL editor that talks to databases through sqail-service.
//!
//! The binary (`src/main.rs`) only calls [`run`]; everything lives here so the
//! UI can be tested headlessly (see `tests/ui.rs`).

pub mod app;
pub mod assistant;
pub mod commands;
pub mod designer;
pub mod dialogs;
pub mod editing;
pub mod editor;
pub mod find;
pub mod folders;
pub mod grid;
pub mod local;
pub mod local_service;
pub mod palette;
pub mod results;
pub mod schema;
pub mod secrets;
pub mod settings;
pub mod settings_ui;
pub mod sidebar;
pub mod sql;
pub mod textfile;
pub mod theme;
pub mod transfer;
pub mod worker;

pub use app::SqailApp;

/// Start the desktop app.
pub fn run() -> eframe::Result {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("SQAIL_UI_LOG")
                .unwrap_or_else(|_| "warn,sqail_ui=info".into()),
        )
        .init();
    settings::migrate_from_sqail2();

    // One sqail per config directory: a second one would overwrite the
    // first one's tabs and settings. Held until `run` returns.
    let _lock = match local::InstanceLock::acquire(&settings::config_dir()) {
        Ok(lock) => Some(lock),
        Err(local::LockError::Held) => return already_running(),
        Err(local::LockError::Unavailable(e)) => {
            tracing::warn!(error = %e, "no single-instance lock; continuing without one");
            None
        }
    };

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("sqail")
            .with_app_id("sqail")
            .with_inner_size([1400.0, 900.0])
            .with_min_inner_size([720.0, 480.0])
            .with_icon(
                eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon-256.png"))
                    .unwrap_or_default(),
            ),
        ..Default::default()
    };
    eframe::run_native(
        "sqail",
        options,
        Box::new(|cc| Ok(Box::new(app::SqailApp::new(cc)))),
    )
}

/// A small window saying sqail is already running, instead of a second
/// editor that would overwrite the first one's tabs and settings.
fn already_running() -> eframe::Result {
    struct AlreadyRunning;
    impl eframe::App for AlreadyRunning {
        fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
            egui::CentralPanel::default().show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(12.0);
                    ui.heading("sqail is already running");
                    ui.add_space(8.0);
                    ui.label(format!(
                        "Use the open sqail window: two windows would overwrite each \
                         other's tabs and settings. (Config directory: {})",
                        settings::config_dir().display()
                    ));
                    ui.add_space(12.0);
                    if ui.button("Close").clicked() {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
            });
        }
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("sqail")
            .with_app_id("sqail")
            .with_inner_size([460.0, 180.0])
            .with_resizable(false),
        persist_window: false,
        ..Default::default()
    };
    eframe::run_native(
        "sqail-already-running",
        options,
        Box::new(|_| Ok(Box::new(AlreadyRunning))),
    )
}
