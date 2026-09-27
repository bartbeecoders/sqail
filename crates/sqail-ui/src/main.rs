//! sqail2 desktop app entry point.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() -> eframe::Result {
    sqail_ui::run()
}
