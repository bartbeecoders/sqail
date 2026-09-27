You are an expert full-stack developer specializing in Tauri v2 apps (Rust backend + modern TypeScript/React frontend). Help me build a lightweight, fast, cross-platform desktop database editor that runs on Windows, macOS, and Linux.

Project goals:
- Extremely fast startup and responsive UI (use Tauri's native webview, avoid Electron-style bloat)
- Modern SQL query editor with syntax highlighting, auto-formatting, line numbers, dark/light themes, and basic autocompletion
- Database connection management (support at minimum PostgreSQL, MySQL, SQLite; bonus for others if easy)
- AI integration hook (a button or sidebar to send the current query or selected text to an AI API like Claude, OpenAI, or Minimax for natural-language-to-SQL generation, query explanation, or optimization)
- Result viewer: nice data grid/table for query results (sortable, paginated if large), export to CSV/JSON
- Keep the app lightweight (<20 MB binary ideally), snappy, and vibe-code friendly (easy to iterate with AI assistance)

Tech stack requirements:
- Tauri v2 (latest stable as of March 2026)
- Frontend: React (with Vite) + TypeScript + shadcn/ui or Tailwind for clean, modern UI components
- SQL editor: Use @monaco-editor/react (the VS Code editor) with 'sql' language support. Add basic schema-aware completion later if possible.
- Database access: Use official tauri-plugin-sql (sqlx backend) for secure, async queries from frontend → Rust
- State management: Zustand or Jotai (simple & lightweight)
- UI: Use shadcn/ui components + lucide-react icons for a professional look

Settings and user preferences storage
- use surrealdb for local storage of settings and user preferences (embed surrealdb in the app)
- see https://surrealdb.com/docs/sdk/rust/ for documentation

AI integration
- The user will be able to use AI LLM models to help create queries, maintain database schemas, and optimize queries.
- Also for generating documentation about the database.
- Allow for the user to maintain a list of AI models and providers to use for different tasks.

Key features to implement step-by-step:
1. Connection setup screen on first launch (or sidebar): form for DB type, host/port/dbname/user/pass (or file path for SQLite). Test connection button.
2. Sidebar or tabs: list active connections, switch between them.
3. Main area: Monaco SQL editor (full height, resizable split with results below).
4. Toolbar: Run query (Ctrl+Enter), Format SQL, Clear, AI Generate/Explain button.
5. Results pane: show rows in a virtualized table, show row count, execution time, errors nicely.
6. AI button: take current query or selected text + user prompt → call external API (you can stub with a fetch placeholder for now, e.g. to http://localhost:mock-ai or real endpoint).
7. Error handling, loading states, dark mode support.

Important constraints & best practices:
- Use tauri-plugin-sql v2 syntax: db.select("SELECT ...", params), db.execute(...), etc. from frontend via invoke.
- Secure: no dangerous eval, proper allowlist in tauri.conf.json (only allow sql:* commands we define).
- Performance: stream large results if possible, or paginate queries.
- Make code modular and easy to extend (e.g. add more DB drivers later).
- Use modern React patterns: hooks, functional components, no class components.
- Include basic theming (sync with system or manual toggle).

Start by generating the complete initial project structure (file tree + key files content):
- package.json & tsconfig.json (Vite + React + TS)
- tauri.conf.json with sql plugin enabled
- src-tauri/src/main.rs with plugin builder and basic state
- src-tauri/src/commands.rs with example connect/execute commands
- src/App.tsx skeleton with layout (sidebar + editor + results split)
- SqlEditor.tsx with Monaco integration (import monaco from '@monaco-editor/react', set language 'sql')
- Basic Connection form and store

After the skeleton, I will ask you to implement one feature at a time (e.g. "now implement connection management", "add AI button", etc.).

Generate the full starter code now. Be verbose with comments so I (and future AI) can understand everything. Make it beautiful and production-ready from the start.