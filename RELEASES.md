# Releases

> The authoritative changelog lives in `releases.json` (consumed by both the in-app About tab and the portal). This file tracks selected highlights.

## Unreleased

## v0.5.0

### Inline AI completion
- Ghost-text SQL completion powered by a local llama.cpp sidecar — no cloud calls, nothing leaves your machine
- Opt-in: flip the toggle in **Settings → Inline AI** and pick a model
- Three quantised GGUF models in the catalog: Qwen2.5-Coder-1.5B (low-end), Qwen2.5-Coder-3B (default), DeepSeek-Coder-V2-Lite 16B MoE (performance)
- Token-budget-aware schema context keeps prompts lean for ~6 ms first-token latency on GPU
- `Tab` to accept, `Esc` to dismiss; cursor moves and tab/connection switches cancel in-flight requests
- Toolbar indicator shows sidecar state at a glance; click to open the Inline AI settings tab
- Optional latency telemetry pane for debugging
- Bundled `llama-server` binaries for Windows/macOS are a follow-up; today the feature runs against any OpenAI-compatible local endpoint (Ollama, LM Studio) or a user-supplied `llama-server` pointed at via `SQAIL_LLAMA_SERVER_PATH`

## v0.4.2

### Auto-Update
- In-app auto-update: checks for new versions on startup, shows a banner when an update is available
- One-click download, install, and restart from the update banner
- Manual "Check for Updates" button in Settings > About
- Download progress bar with percentage indicator

### CI / Build
- CI pipeline signs update bundles with TAURI_SIGNING_PRIVATE_KEY
- New GitHub release job generates latest.json updater manifest and uploads all platform binaries
- Helper script for generating updater signing key pair

## v0.4.1

### SQL Formatting
- Stored procedure formatting: embedded SELECT statements properly formatted with fields on separate lines, aligned AS aliases
- WHERE/ON clauses break AND/OR conditions onto separate indented lines
- Column alignment capped at 50 chars to prevent excessive whitespace
- Fixed false table alias prefix on function calls and string literals
- New "Format with AI" option with diff preview (side-by-side Monaco diff editor, Accept/Reject)
- SQL Formatting settings: indent size, uppercase keywords, AND/OR new lines

### Schema Tree
- Ctrl+scroll font resizing, separate Procedures/Functions categories

### Editor & Settings
- Ctrl+scroll zoom (changed from Shift), routine drag-and-drop, grid-aligned settings layout

## v0.4.0

### Portal Screenshots Gallery
- New Screenshots section on the portal with tabbed gallery: Editor, Connections, AI, Split Editor, Light Theme
- Editor screenshot embedded in Docs Getting Started panel
- Screenshot capture and prep scripts for consistent marketing screenshots

### AI & Editor (from 0.3.4–0.3.6)
- AI prompt history: last 10 prompts navigable with Up/Down arrows in command palette
- AI prompts now include active editor query as context
- Split editor can share the same file across panes
- DbService connect flow for shared database connections

### macOS & Build
- Universal macOS build (arm64 + x86_64) with ad-hoc codesign
- macOS download renamed to _universal.dmg
- Windows release build fixes
- Editor drag-drop fix
- Vite bumped to 8.0.5 (security fixes)

### Portal & Docs
- Why sqail comparison table vs DBeaver, DataGrip, TablePlus, Beekeeper
- macOS Gatekeeper install note with xattr workaround on Downloads page
- Brand guide, marketing strategy, and marketing plan documents
- README rewritten with five messaging pillars

---

## v0.3.6

### macOS Build Fixes
- macOS release is now a true Universal binary (arm64 + x86_64) built via `--target universal-apple-darwin` — runs natively on both Apple Silicon and Intel Macs
- Fixed misleading `_x64.dmg` filename on arm64 hosts: CI output is now `sqail_<version>_<build>_universal.dmg`
- Ad-hoc codesign of the `.app` bundle after lipo, so every binary inside has a valid signature and will execute on arm64
- Workaround documented for "sqail is damaged and cannot be opened" Gatekeeper error: run `xattr -cr /Applications/sqail.app` once per install (proper Apple Developer ID signing + notarization still on the roadmap)

### Portal (sqail.io)
- New "Why sqail?" comparison table vs DBeaver, DataGrip, TablePlus, and Beekeeper Studio
- macOS install-notes block on the download page with the `xattr` workaround, highlighted for visitors detected as macOS

### Docs
- README rewritten with an elevator pitch, feature bullets keyed to the five messaging pillars, and a macOS install note
- New `marketing/brand-guide.md` documenting the color palette, typography, voice, and messaging pillars

---

## v0.2.0

### AI Integration
- AI assistant sidebar panel with 4 flows: Generate SQL, Explain Query, Optimize Query, Generate Documentation
- 7 AI provider types: Claude, OpenAI, Minimax, Z.ai, Claude Code CLI, LM Studio, OpenAI Compatible
- Streaming responses via SSE with real-time output in the sidebar
- Schema context injection — automatically includes active database schema in AI prompts
- Prompt history with persistence (JSON file, capped at 100 entries)
- AI provider settings: add, edit, delete, set default, test connection
- Insert AI-generated SQL directly into the editor
- Monaco editor context menu: "AI: Explain Query" and "AI: Optimize Query"

### Keyboard Shortcuts
- Global keyboard shortcut system with configurable bindings
- Default shortcuts: F5 (Run), Ctrl+S (Save), Ctrl+O (Open), Ctrl+Shift+S (Save As), Ctrl+Shift+N (New Connection), Ctrl+Shift+A (Toggle AI), Ctrl+N (New Tab), Ctrl+W (Close Tab), Ctrl+Shift+F (Format)
- Shortcuts persisted in localStorage, survive app restarts

### File Operations
- Save query to .sql file (Ctrl+S) with native file dialog
- Save As (Ctrl+Shift+S) always prompts for file location
- Open .sql file (Ctrl+O) into current or new editor tab
- Tab title updates to reflect the saved filename
- Tauri dialog and filesystem plugins for native OS integration

### Settings Page
- Settings modal accessible via toolbar gear icon or Ctrl+,
- General tab (placeholder for future editor/appearance/execution preferences)
- Keyboard Shortcuts tab with full shortcut configuration
- Click-to-record shortcut editing: click a binding, press desired key combo
- Per-shortcut and "Reset All" reset buttons

---

## v0.1.0

### Project Foundation
- Tauri v2 desktop app with React + TypeScript + Vite frontend
- Tailwind CSS + shadcn/ui design system with dark/light theme (system preference)
- Collapsible sidebar, resizable editor/results split panel layout

### Database Connections
- Support for PostgreSQL, MySQL, SQLite, and SQL Server (MSSQL)
- Create, edit, delete, and test database connections
- Connection configs persisted as JSON in app data directory
- Multiple simultaneous connections with independent pools
- Color-coded connection indicators, active state tracking

### SQL Editor
- Monaco editor with SQL language mode and custom dark/light themes
- SQL keyword autocompletion (90+ keywords, functions, types)
- Schema-aware completions: table names and column names from connected database
- Multi-tab editor with Ctrl+N / Ctrl+W, persisted in localStorage
- SQL formatting via sql-formatter (Ctrl+Shift+F)
- Execute with Ctrl+Enter (supports text selection for partial execution)

### Query Execution & Results
- Execute queries against active connection with execution time tracking
- Virtualized data grid (TanStack Table + TanStack Virtual) for large result sets
- Sortable columns, row numbers, typed value rendering (NULL, boolean, numbers)
- Multiple result sets from semicolon-separated statements
- Loading spinner and error display in results pane

### Schema Browser
- Tree view: Schema > Tables/Views > Columns with type, PK, nullable info
- Double-click table to generate SELECT statement in editor
- Right-click context menu: SELECT *, COUNT(*), DESCRIBE, DROP (commented)
- Drag-and-drop tables into editor to generate aliased SELECT
- Auto-refresh on connection change, search/filter, manual refresh
