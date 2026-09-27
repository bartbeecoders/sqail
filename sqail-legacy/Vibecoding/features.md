# Features

## Connection Management

- **Multi-driver support** — PostgreSQL, MySQL, SQLite out of the box; extensible for SQL Server, MariaDB, CockroachDB
- **Connection form** — adaptive fields per driver type (host/port for network DBs, file picker for SQLite)
- **Test connection** — verify credentials and reachability before saving
- **Connection profiles** — save, edit, delete named connections with optional color coding
- **Multiple simultaneous connections** — maintain several active connection pools, switch between them
- **Connection groups** — organize connections by environment (dev/staging/prod) or project
- **SSL/TLS support** — configure SSL mode, client certs, CA bundles per connection
- **SSH tunnel support** — connect through an SSH bastion host to reach remote databases
- **Connection cloning** — duplicate an existing connection for quick setup of similar environments
- **Auto-reconnect** — detect dropped connections and transparently re-establish them

## SQL Editor

- **Monaco editor** — full VS Code editing experience: syntax highlighting, line numbers, minimap, folding
- **Multi-tab editing** — each tab is an independent query buffer, persisted across app restarts
- **SQL formatting** — auto-format queries with one click or `Ctrl+Shift+F`
- **Execute query** — run full editor content or only selected text with `Ctrl+Enter`
- **Keyword autocompletion** — SQL keywords, functions, and data types
- **Schema-aware autocompletion** — suggest table names, column names, aliases from the active connection's schema
- **Multi-cursor editing** — edit multiple lines simultaneously
- **Find and replace** — within the current editor tab
- **Bracket matching** — highlight and auto-close parentheses, quotes
- **Query snippets** — user-defined templates with tab-stop placeholders (e.g., `SELECT $1 FROM $2 WHERE $3`)
- **Split editor** — view two query tabs side by side
- **Syntax error highlighting** — underline SQL errors before execution
- **Drag-and-drop** — drag a table name from the schema browser into the editor

## Query Execution & Results

- **Virtualized data grid** — handle thousands of rows without UI lag
- **Column sorting** — click column headers to sort results client-side
- **Column resizing** — drag column borders to adjust widths
- **Execution metadata** — display row count, execution time, and affected rows
- **Error display** — clear, readable error messages with the failing SQL highlighted
- **NULL indicator** — visually distinct rendering for NULL values (e.g., italic gray `NULL`)
- **Multiple result sets** — display results for each statement in a multi-statement query
- **Pagination** — navigate large result sets with configurable page size
- **Query cancellation** — abort long-running queries with a cancel button
- **Loading state** — spinner and progress indication during execution
- **Cell value preview** — click a cell to see the full value in a detail pane (useful for long text, JSON, binary)
- **Column type indicators** — show data type icons or badges in column headers
- **Row numbering** — optional row index column in results

## Schema Browser

- **Tree view** — hierarchical: Connection → Schema → Tables / Views / Functions
- **Column details** — expand a table to see columns with type, nullability, default, PK/FK indicators
- **Index information** — view indexes, their columns, and uniqueness
- **Quick actions** — double-click a table to generate `SELECT * FROM <table> LIMIT 100`
- **Context menu** — right-click for SELECT, COUNT, DESCRIBE, TRUNCATE, DROP (with confirmation)
- **Search / filter** — type to filter the schema tree by name
- **Refresh** — manual refresh button; auto-refresh on connection change
- **Table row counts** — optionally show approximate row count next to table names
- **Foreign key visualization** — see FK relationships between tables

## AI Integration

- **Multiple AI providers** — configure Claude, OpenAI, Minimax, or any OpenAI-compatible endpoint
- **Provider management** — add, edit, remove providers; set a default per task type
- **Natural-language-to-SQL** — describe what you want in plain English, get SQL back
- **Query explanation** — send a query to AI for a human-readable breakdown of what it does
- **Query optimization** — AI suggests index hints, rewrites, or structural improvements
- **Database documentation generation** — AI produces markdown docs for selected tables, schemas, or the entire database
- **Schema context injection** — automatically include table/column metadata in AI prompts for accurate results
- **Streaming responses** — see AI output token-by-token as it generates
- **Prompt history** — browse and re-use previous AI interactions
- **Error diagnosis** — when a query fails, offer to send the error + query to AI for troubleshooting
- **SQL translation** — convert queries between dialects (e.g., PostgreSQL → MySQL)
- **Data analysis suggestions** — AI recommends queries based on schema structure ("you might want to look at...")
- **Custom system prompts** — users can customize the system prompt per AI task for domain-specific behavior

## Data Export

- **CSV export** — with configurable delimiter, quoting, and encoding
- **JSON export** — array of objects, pretty-printed
- **SQL INSERT export** — generate INSERT statements from result rows
- **Markdown table export** — for pasting into docs, READMEs, or issues
- **Clipboard copy** — copy individual cells, rows, or entire result sets
- **Excel export** — `.xlsx` format for spreadsheet users
- **Export selected rows** — export only highlighted rows, not the full result

## Data Editing

- **Inline cell editing** — click a cell in the results grid to edit its value (tables with a primary key)
- **Change tracking** — highlight modified cells, show pending edits count
- **Preview generated SQL** — see the UPDATE/INSERT/DELETE statements before committing
- **Commit / discard** — apply or revert all pending changes in one action
- **Add row** — insert a new row directly from the grid
- **Delete row** — remove rows with confirmation
- **Bulk edit** — apply the same value to multiple selected cells

## Query History & Saved Queries

- **Auto-logged history** — every executed query is recorded with timestamp, connection, duration, and status
- **Searchable history** — full-text search across past queries
- **Filter history** — by connection, date range, success/error status
- **Load from history** — click any history entry to restore it in the editor
- **Save query** — name, describe, and tag favorite queries
- **Folders and tags** — organize saved queries hierarchically or with tags
- **Import/export** — save queries as `.sql` files; import from files
- **Share queries** — export a saved query with its connection template (credentials stripped)

## Settings & Preferences

- **General settings** — default row limit, query timeout, startup behavior (restore last session or blank)
- **Editor settings** — font size, font family, tab size, minimap on/off, word wrap, ligatures
- **Appearance** — theme (system / light / dark), accent color, sidebar position (left/right), compact mode
- **Keyboard shortcuts** — view all bindings, customize any shortcut
- **AI settings** — manage providers, set defaults, configure system prompts
- **Connection defaults** — default SSL mode, default port per driver, connection timeout
- **Backup & restore settings** — export/import all preferences and saved queries as a file

## Appearance & Theming

- **Dark and light mode** — clean, modern styling in both
- **System theme sync** — follow OS appearance setting automatically
- **Manual toggle** — switch theme from toolbar or settings
- **Accent color** — customize the primary color used across the UI
- **Compact mode** — reduce padding and font sizes for users who prefer density
- **Resizable panels** — drag to resize sidebar, editor, and results pane proportions

## Keyboard Shortcuts

- `Ctrl+Enter` / `Cmd+Enter` — execute query
- `Ctrl+Shift+F` — format SQL
- `Ctrl+N` — new editor tab
- `Ctrl+W` — close current tab
- `Ctrl+S` — save current query
- `Ctrl+H` — find and replace
- `Ctrl+L` — clear editor
- `Ctrl+/` — toggle line comment
- `Ctrl+D` — duplicate line
- `Ctrl+Shift+P` — command palette (search all actions)
- `Ctrl+\`` — toggle results pane
- `Ctrl+B` — toggle sidebar
- `Ctrl+,` — open settings

## Cross-Platform

- **Linux** — AppImage, DEB, RPM packages
- **Windows** — MSI installer and portable EXE
- **macOS** — DMG with universal binary (Intel + Apple Silicon)
- **Native look and feel** — respects system fonts, window chrome, and accent colors
- **Auto-update** — built-in updater checks for and applies new versions

## Performance

- **Fast startup** — native webview, no bundled Chromium; target sub-second launch
- **Small binary** — target <20 MB total
- **Virtualized rendering** — results grid only renders visible rows
- **Async execution** — queries run on background threads, UI stays responsive
- **Connection pooling** — reuse database connections efficiently
- **Lazy schema loading** — only fetch schema details when a node is expanded

## Security

- **No eval** — no dynamic code execution in the frontend
- **Tauri allowlist** — only explicitly defined commands are callable from the frontend
- **Credential storage** — connection passwords stored in SurrealDB with local encryption
- **CSP headers** — strict Content Security Policy on the webview
- **No telemetry** — no data sent anywhere unless the user configures an AI provider
