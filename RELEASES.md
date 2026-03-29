# Releases

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
