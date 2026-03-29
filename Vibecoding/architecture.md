Top Recommendation: Tauri 2 + SvelteKit (or React/Vue) + Rust backendThis combo gives you a lightweight, blazing-fast, modern desktop app with excellent performance for a SQL tool (query execution, result grids, syntax highlighting, etc.).Why Tauri is perfect for this project:Extremely lightweight and fast — Uses the system's native WebView (no bundled Chromium like Electron). Apps often start under 10-20 MB and use far less RAM/CPU. Ideal for a "fast" SQL editor where you want snappy query results and smooth tables.
Cross-platform out of the box — Builds native installers/executables for Linux (AppImage/DEB/RPM), Windows (.exe/MSI), and macOS (.dmg/app).
Modern web frontend — You write the UI in HTML/CSS/JS with any framework you like (Svelte for minimal overhead and great DX, React for ecosystem, or even vanilla + Tailwind).
Rust core — Handles heavy lifting securely and efficiently (e.g., database connections via sqlx or tauri-plugin-sql, file I/O, multi-threading for long queries). Rust is fast and memory-safe—great for a tool that might connect to PostgreSQL, MySQL, SQLite, etc.
Security & performance edge — Better than Electron for resource usage; modern feel without bloat.
Vibe coding friendly — You can prototype the UI quickly with web tech (hot reload is excellent), then add Rust commands as needed. AI tools love generating Svelte + Tauri code.

Existing similar apps: Tools like Beekeeper Studio (modern open-source SQL client) show that web-tech desktop apps work great for this use case, but Tauri makes it lighter and faster than pure Electron apps.

