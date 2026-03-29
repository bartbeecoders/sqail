# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**sqail** — A lightweight, cross-platform desktop SQL database editor with AI integration, built with Tauri v2.

## Tech Stack

- **Desktop framework:** Tauri v2 (Rust backend + native webview)
- **Frontend:** React + TypeScript + Vite
- **UI:** shadcn/ui + Tailwind CSS + lucide-react icons
- **SQL editor:** @monaco-editor/react with SQL language support
- **State management:** Zustand or Jotai
- **Database access:** tauri-plugin-sql (sqlx backend) — PostgreSQL, MySQL, SQLite minimum
- **Local storage:** SurrealDB embedded (settings, user preferences) — see https://surrealdb.com/docs/sdk/rust/
- **AI integration:** Multiple LLM providers (Claude, OpenAI, Minimax) for NL-to-SQL, query explanation, optimization, and DB documentation

## Architecture (Planned)

- `src-tauri/` — Rust backend: Tauri plugin setup, database connection commands, SurrealDB for local state
- `src/` — React frontend: Monaco SQL editor, connection manager sidebar, results grid, AI sidebar/button
- Frontend communicates with Rust via Tauri invoke commands (not direct DB access)
- Security: allowlist in `tauri.conf.json` restricts to defined `sql:*` commands only

## Key Constraints

- Target binary size <20 MB
- No Electron patterns — use Tauri's native webview
- Modern React only: hooks, functional components
- Use `tauri-plugin-sql` v2 API: `db.select()`, `db.execute()` from frontend via invoke
- Large result sets should be paginated or streamed
- Dark/light theme support (sync with system or manual toggle)

## Planning Documents

- `Vibecoding/instructions.md` — Full feature specs and implementation order
- `Vibecoding/architecture.md` — Architecture rationale and tech choices
