//! sqail desktop app entry point.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() -> eframe::Result {
    // `sqail mcp`: the MCP server the AI assistant's CLI starts (no window).
    if std::env::args().nth(1).as_deref() == Some("mcp") {
        if let Err(e) = sqail_ui::assistant::mcp::run_from_env() {
            eprintln!("sqail mcp: {e:#}");
            std::process::exit(1);
        }
        return Ok(());
    }
    sqail_ui::run()
}
