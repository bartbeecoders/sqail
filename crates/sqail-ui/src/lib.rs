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
