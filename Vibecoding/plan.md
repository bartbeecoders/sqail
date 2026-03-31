# Implementation Plan

## Phase 1 — Project Scaffolding & Core Shell

**Goal:** Bootable Tauri v2 app with empty UI skeleton and build pipeline.

### Steps

- [x] 1.1. Initialize Tauri v2 project with Vite + React + TypeScript template
- [x] 1.2. Configure `tauri.conf.json` — app metadata, window defaults, security allowlist
- [x] 1.3. Add Tailwind CSS, shadcn/ui, and lucide-react; configure theme tokens (dark/light)
- [x] 1.4. Set up Rust workspace in `src-tauri/` — add `tauri-plugin-sql`, `surrealdb` crate deps
- [x] 1.5. Create basic app layout component: sidebar (collapsible), main editor area, bottom results pane (resizable split)
- [x] 1.6. Add top toolbar placeholder (Run, Format, Clear, AI buttons — disabled for now)
- [x] 1.7. Verify cross-platform build: `cargo tauri dev` on Linux, test release build
- [x] 1.8. Add ESLint + Prettier config for frontend; `cargo clippy` for Rust

---

## Phase 2 — Database Connection Management

**Goal:** Users can create, test, edit, delete, and switch between database connections.

### Steps

- [x] 2.1. Design connection data model: `{ id, name, driver, host, port, database, user, password, filePath?, sslMode?, color? }`
- [x] 2.2. JSON file store in app data dir — init on app startup, store connections (SurrealDB deferred to Phase 8)
- [x] 2.3. Create Tauri commands: `create_connection`, `update_connection`, `delete_connection`, `list_connections`, `test_connection`, `connect`, `disconnect`, `get_active_connection`
- [x] 2.4. Build connection form UI (modal) with fields adapting to driver type (PostgreSQL/MySQL show host/port, SQLite shows file picker)
- [x] 2.5. Add "Test Connection" button with success/failure feedback
- [x] 2.6. Build sidebar connection list — show name, driver badge, colored indicator, active state, hover actions
- [x] 2.7. Implement connection switching: clicking a connection establishes it as active, stores pool handle in Rust app state
- [x] 2.8. Handle connection errors gracefully — timeout, auth failure, unreachable host (5s timeout on test, 10s on connect)
- [x] 2.9. Support multiple simultaneous connections (each gets its own sqlx AnyPool)

---

## Phase 3 — SQL Editor (Monaco)

**Goal:** Fully functional SQL editor with syntax highlighting, formatting, and keyboard shortcuts.

### Steps

- [x] 3.1. Integrate `@monaco-editor/react` with SQL language mode
- [x] 3.2. Configure editor options: line numbers, minimap off, word wrap, font size 14, JetBrains Mono, bracket pair colorization, smooth cursor
- [x] 3.3. Implement dark/light theme sync — custom `sqlai-dark` / `sqlai-light` themes follow system preference via `useDarkMode` hook
- [x] 3.4. Add SQL formatting via `sql-formatter` — toolbar button + `Ctrl+Shift+F` keybinding in Monaco
- [x] 3.5. Wire up `Ctrl+Enter` / `Cmd+Enter` to execute current query (or selected text)
- [x] 3.6. Add multi-tab editor support — tab bar with add/close, `Ctrl+N` new tab, `Ctrl+W` close tab
- [x] 3.7. Persist open tabs and their content in localStorage (survives app restart)
- [x] 3.8. Add basic SQL keyword autocompletion (90+ keywords, functions, types)

---

## Phase 4 — Query Execution & Results

**Goal:** Execute queries against active connection and display results in a performant grid.

### Steps

- [x] 4.1. Create Tauri command `execute_query(connection_id, sql)` — returns rows + column metadata + execution time
- [x] 4.2. Handle SELECT vs INSERT/UPDATE/DELETE — return rows for SELECT, affected count for mutations
- [x] 4.3. Build results data grid using TanStack Table + TanStack Virtual for virtualized rendering
- [x] 4.4. Show column headers with data types, sortable columns (client-side), row numbers
- [x] 4.5. Display execution time, row count, and error messages in a status bar below results
- [x] 4.6. Virtualized rendering handles large result sets without pagination (overscan=20)
- [x] 4.7. Add NULL value rendering (italic gray `NULL`), boolean coloring, number tabular-nums, long text truncation
- [x] 4.8. Support multiple result sets (SQL split on `;` respecting strings/comments, tab switcher per result)
- [x] 4.9. Add loading spinner in results pane + toolbar Run button shows "Running..." with spinner
- [ ] 4.10. Implement query cancellation (abort long-running queries) — deferred

---

## Phase 5 — Schema Browser

**Goal:** Users can explore database structure — schemas, tables, columns, indexes, views.

### Steps

- [x] 5.1. Create Tauri commands: `list_schemas`, `list_tables`, `list_columns`, `list_indexes` — per-driver SQL for Postgres, MySQL, SQLite
- [x] 5.2. Build tree view in sidebar: Schema → Tables/Views → Columns (with type, PK icon, nullable)
- [x] 5.3. Show column details: name, data type, PK key icon (amber), nullable tooltip, default value tooltip
- [x] 5.4. Double-click table name inserts `SELECT * FROM schema.table LIMIT 100;` into editor
- [x] 5.5. Right-click context menu: SELECT * LIMIT 100, SELECT COUNT(*), DESCRIBE columns, DROP (commented out for safety)
- [x] 5.6. Auto-refresh on connection change, manual refresh button, auto-expand single schema, filter/search input
- [x] 5.7. Schema-aware Monaco completions: table names (kind=Class), column names (kind=Field) sorted above keywords

---

## Phase 6 — AI Integration

**Goal:** Users can leverage LLMs for natural-language-to-SQL, query explanation, optimization, and documentation.

### Steps

- [x] 6.1. Design AI provider model: `{ id, name, provider, apiKey, model, baseUrl?, isDefault? }`
- [x] 6.2. Store AI provider configs in JSON file; build settings UI to add/edit/remove providers
- [x] 6.3. Create Rust-side HTTP client for AI API calls (Claude, OpenAI, Minimax, custom OpenAI-compatible)
- [x] 6.4. Build AI sidebar panel — text input for natural language prompt, output area for AI response
- [x] 6.5. Implement "Generate SQL" flow: user describes query in natural language → AI returns SQL → insert into editor
- [x] 6.6. Implement "Explain Query" flow: send current query → AI returns explanation displayed in sidebar
- [x] 6.7. Implement "Optimize Query" flow: send current query + schema context → AI suggests optimized version
- [x] 6.8. Implement "Generate Documentation" flow: send schema info → AI produces markdown documentation for tables/columns
- [x] 6.9. Add context injection: automatically include active schema/table metadata in AI prompts for better results
- [x] 6.10. Stream AI responses (SSE/streaming) for real-time output in the sidebar
- [x] 6.11. Add prompt history — recall previous AI interactions

---

## Phase 7 — Data Export

**Goal:** Export query results to common formats.

### Steps

- [x] 7.1. Export to CSV — with proper escaping, configurable delimiter
- [x] 7.2. Export to JSON — array of objects, pretty-printed
- [x] 7.3. Export to SQL INSERT statements
- [x] 7.4. Export to Markdown table
- [x] 7.5. Copy cell / row / selection to clipboard
- [x] 7.6. Add export button to results toolbar with format dropdown

---

## Phase 8 — Settings & Preferences

**Goal:** Persistent, user-configurable settings stored in embedded SurrealDB.

### Steps

- [x] 8.1. Build settings page/modal with sections: General, Editor, Appearance, Keyboard Shortcuts, About
- [x] 8.2. General: default row limit, query timeout
- [x] 8.3. Editor: font size, font family, tab size, minimap, word wrap, line numbers
- [x] 8.4. Appearance: theme (system/light/dark)
- [x] 8.5. Sync all settings to localStorage; load on startup, apply reactively
- [x] 8.6. Add keyboard shortcuts settings — view and customize bindings

---

## Phase 9 — Query History & Saved Queries

**Goal:** Users never lose a query they ran and can save favorites.

### Steps

- [x] 9.1. Auto-log every executed query with timestamp, connection, execution time, status (success/error)
- [x] 9.2. Build query history panel — searchable, filterable by connection/date/status
- [x] 9.3. Click history entry to load it into editor
- [x] 9.4. Add "Save Query" action — name, optional description, tags
- [x] 9.5. Build saved queries panel with folders/tags organization
- [x] 9.6. Import/export saved queries as `.sql` files

---

## Phase 10 — Advanced Editor Features

**Goal:** Power-user editor capabilities.

### Steps

- [x] 10.1. Multi-cursor editing support (Monaco built-in, ensure it works)
- [x] 10.2. Find and replace within editor (`Ctrl+H`)
- [x] 10.3. Query snippets — user-defined templates with `$1`, `$2` placeholders
- [x] 10.4. Split editor view — run two queries side by side
- [x] 10.5. Syntax error highlighting via a lightweight SQL parser
- [x] 10.6. Bracket matching and auto-close for parentheses and quotes

---

## Phase 11 — Data Editing (Optional DML)

**Goal:** Edit table data directly in the results grid.

### Steps

- [ ] 11.1. Enable inline cell editing in results grid (for tables with a primary key)
- [ ] 11.2. Track changed cells, highlight pending edits
- [ ] 11.3. Generate and preview UPDATE/INSERT/DELETE statements before applying
- [ ] 11.4. Commit or discard pending changes
- [ ] 11.5. Add row insertion and row deletion from grid

---

## Phase 12 — Polish, Performance & Release

**Goal:** Production-ready, release-quality app.

### Steps

- [ ] 12.1. Audit binary size — tree-shake unused deps, optimize Rust release profile
- [ ] 12.2. Profile frontend performance — large result sets, editor with big files
- [ ] 12.3. Add onboarding flow for first-time users (quick-start wizard)
- [ ] 12.4. Cross-platform testing: Linux (AppImage/DEB), Windows (MSI/EXE), macOS (DMG)
- [ ] 12.5. Add auto-update support via Tauri updater plugin
- [ ] 12.6. Write user-facing docs / built-in help
- [ ] 12.7. Set up CI/CD pipeline — build + test + package for all platforms
- [ ] 12.8. Create app icon and branding assets
- [ ] 12.9. Final security audit — CSP headers, allowlist review, no credential leaks
