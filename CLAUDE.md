# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**sqail** (pronounced *"snail"*) — A fast, <20 MB, cross-platform desktop SQL editor with first-class AI integration, built on Tauri v2. Speaks PostgreSQL, MySQL, SQLite, and Microsoft SQL Server, plus two remote backends (a self-hosted DbService HTTP proxy and SurrealDB over HTTP). Zero telemetry; credentials and all state live on-disk locally.

Canonical repo is [Codeberg](https://codeberg.org/bartbeecoders/sqail); GitHub is a mirror.

## Commands

Package manager is **pnpm** (not npm). The Rust side is the `app_lib` crate under `src-tauri/`.

```bash
pnpm install
pnpm tauri dev            # run app with hot reload (frontend + Rust backend)
./scripts/run.sh dev      # same, but first launches the local DbService and frees ports 5100/1420 (Linux: also sets WEBKIT_DISABLE_DMABUF_RENDERER=1 to fix blank screen)

pnpm check                # tsc --noEmit (type check)
pnpm lint                 # eslint src/
pnpm format               # prettier --write src/
./scripts/run.sh check    # pnpm check + pnpm lint + cargo clippy -D warnings  ← run this before pushing

cd src-tauri && cargo clippy -- -D warnings   # Rust lint only
cd src-tauri && cargo test                     # Rust tests
```

There is **no frontend test runner configured** — verification is via `check`/`lint`/`clippy` and manual testing in `tauri dev`.

Release builds are per-platform scripts: `./scripts/build-linux.sh`, `./scripts/build-macos.sh`, `.\scripts\build-windows.ps1`. They check prereqs, build frontend + Rust, and emit installers under `src-tauri/target/release/bundle/`.

## Architecture

Two-process Tauri app. The React frontend **never touches databases directly** — despite `tauri-plugin-sql` being registered, all DB and AI work goes through custom Rust `#[tauri::command]`s invoked via `@tauri-apps/api/core`'s `invoke()`. The full command surface is the `invoke_handler!` list in `src-tauri/src/lib.rs` — read it first to see what the backend exposes.

### Backend (`src-tauri/src/`)

- **`lib.rs`** — `run()`: registers Tauri plugins, builds `AppState`, lists every invoke command. Entry point for understanding the API.
- **`state.rs`** — `AppState`: the single managed state struct. Holds in-memory `Mutex`-guarded vectors (connections, AI providers, history, saved queries, metadata) backed by flat-file stores, plus the live `pools: HashMap<connId, DbPool>` and inline-AI / training state.
- **`pool.rs`** — `DbPool` enum unifying every backend: `Postgres`/`Mysql`/`Sqlite` (sqlx pools), `Mssql` (tiberius via bb8), `DbService` (HTTP proxy), `SurrealDb` (HTTP). Adding a backend means adding a variant here and threading it through query/schema code.
- **`db/`** — connection configs (`connections.rs`) and their flat-file JSON store (`store.rs`).
- **`commands.rs`** — most Tauri command handlers (connections, query exec, schema introspection, AI ops, metadata, history, inline AI, training). Thin wrappers over the modules below.
- **`query.rs` / `schema.rs` / `metadata.rs`** — query execution + result shaping, schema introspection (tables/columns/indexes/routines/FKs), and AI-generated object documentation.
- **`crypto.rs`** — at-rest secret encryption (AES-GCM with an Argon2-derived key). Connection passwords/API keys are encrypted before hitting the JSON stores. `sqail_encrypt_secret`/`sqail_decrypt_secret` commands expose this.
- **`auth/entra.rs`** — Microsoft Entra ID (Azure AD) device-code login for SQL Server; tokens cached in `AppState.entra_tokens`.
- **`dbservice.rs`** — HTTP client for the optional ASP.NET **Sqail.DbService** (in `sqail-dbservice/`), a FastEndpoints query proxy authenticated with a pre-shared API key exchanged for a JWT.
- **`ai/`** — provider abstraction (`provider.rs`, `client.rs`, `store.rs`, `prompt.rs`) for cloud LLMs (Claude, OpenAI, Minimax, Z.ai, OpenAI-compatible, OpenRouter, Claude Code CLI). Two sub-systems:
  - **`ai/inline/`** — local ghost-text completion. Manages a downloaded `llama-server` (llama.cpp) **sidecar** + GGUF model catalog, FIM completion. Off by default; binary and models are fetched at runtime into `<app_data>/inline-ai/`. Override the binary with `SQAIL_LLAMA_SERVER_PATH`.
  - **`ai/training/`** — LoRA fine-tuning jobs over collected query datasets (`scripts/train_sql_lora.py`).
- **`git/`** — libgit2 (`git2` crate) integration: a Git panel for versioning saved queries / schema snapshots, with AI-generated migration scripts.

State persistence is **flat JSON files in the OS app-data dir** (e.g. `connections.json`), not an embedded database. (Older docs/README mention "encrypted SurrealDB" for credentials — that's inaccurate; SurrealDB here is only a *remote query target*.)

### Frontend (`src/`)

- **`App.tsx`** — top-level layout wiring: TitleBar, Sidebar, Toolbar, EditorArea, ResultsPane, AiPanel, modals. UI layout prefs persist to `localStorage` under `sqail_*` keys.
- **`stores/`** — **Zustand** stores, one per domain (`connectionStore`, `queryStore`, `editorStore`, `aiStore`, `schemaStore`, `metadataStore`, `gitStore`, `inlineAiStore`, `trainingStore`, etc.). This is the state layer; components subscribe to slices. State management is Zustand, *not* Jotai.
- **`components/`** — feature panels (SqlEditor/Monaco, DataGrid via `@tanstack/react-table` + `react-virtual`, SchemaTree, SchemaDiagram, AiCommandPalette, GitPanel, ProjectPanel, settings tabs, etc.).
- **`lib/`** — pure helpers: SQL formatting (`sqlFormat`, `sqlFormat` uses `sql-formatter`), completions/validation, schema context building for AI prompts, result/schema export, the `.sqail` file codec (`lib/sqail/codec.ts`), inline-AI client glue.
- **`hooks/`** — cross-cutting React hooks (`useAiStream` for streaming AI responses, `useInlineAiLifecycle`, `useGlobalShortcuts`, `useDarkMode`, `useMetadataEvents`).
- **`types/`** — shared TypeScript types mirroring the Rust command payloads.

Streaming AI responses come back as Tauri events, consumed by `useAiStream`.

## Tauri permissions

The webview allowlist lives in `src-tauri/capabilities/default.json` — `sql:`, `dialog:`, `fs:` (scoped to `$HOME`/`$DOWNLOAD`/`$DOCUMENT`/`$DESKTOP`), `updater:`, `window-state:`, `process:allow-restart`. New plugin capabilities must be added here.

## Conventions & constraints

- **Binary-size budget: <20 MB total.** New Rust dependencies (especially a new DB driver) need justification against this — open an issue first. Release profile is already aggressive (`opt-level = "s"`, `lto`, `strip`, `codegen-units = 1`).
- Modern React only — hooks + functional components.
- Commit style: `<area>: <imperative summary>`, present tense, lowercase start, no trailing period, first line ≤72 chars. One logical change per PR; no drive-by refactors. See `CONTRIBUTING.md`.
- User-visible changes get a one-line entry in `RELEASES.md`.
- A build number is tracked in `build-number.json` (`scripts/build-number.*`).

## Adding things

- **A database driver** — add a `DbPool` variant (`pool.rs`), thread it through query/schema/metadata, add connection-form fields under `src/components/`, and a smoke test against a real DB (not a mock). Pattern: existing sqlx/tiberius drivers.
- **An AI provider** — make it OpenAI-compatible or bring a self-contained client in `ai/`; support streaming if the API allows; accept user-supplied keys (never hardcode). Pattern: existing Claude/OpenAI/Minimax integrations.

## Planning documents

`Vibecoding/` holds the full design history — `architecture.md`, `instructions.md` (feature specs + implementation order), `inline-ai.md` + `inline-ai-benchmarks.md`, `git-integration.md`, `database-service.md`, `llm-training.md`, `nosql-connections.md`, `issues-bugs.md`, and more. These describe *intended* design and may run ahead of or behind the code — verify against source before relying on them.
