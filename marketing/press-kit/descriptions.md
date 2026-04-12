# Descriptions

Ready-to-paste blocks at three lengths. Pick one and go — no rewriting required.

## Short (≈50 words)

sqail is a lightweight, open-source SQL editor with first-class AI integration. It speaks PostgreSQL, MySQL, SQLite, and SQL Server from a single native app under 20 MB. Schema-aware AI turns plain-English questions into real SQL against your real tables. Credentials stay local. No telemetry, no paid tier.

## Medium (≈120 words)

sqail (pronounced *"snail"*) is a cross-platform desktop SQL client built on Tauri v2. It launches in under a second, ships as a sub-20 MB binary, and integrates AI as a core feature rather than a paid add-on. The editor is Monaco — the same one that powers VS Code — with multi-cursor, split view, and SQL language support. sqail connects to PostgreSQL, MySQL, SQLite, and SQL Server; stores credentials locally in an encrypted SurrealDB; and lets users bring their own API key for Claude, OpenAI, Minimax, Z.ai, LM Studio, or any OpenAI-compatible endpoint. The project is MIT-licensed, hosted on Codeberg with a GitHub mirror, and ships with zero telemetry.

## Long (≈260 words)

sqail is an open-source SQL editor designed for developers, data analysts, and students who want a fast, modern tool that treats AI as built-in rather than as a paid add-on. It was built on Tauri v2 — Rust plus a native webview — to avoid the Electron tax: the distributable is under 20 megabytes, cold-start is measured in hundreds of milliseconds, and memory use stays flat across long sessions.

The editing experience is based on Monaco, the same editor that powers VS Code, with SQL language support, multi-cursor, snippets, and a true split editor. A schema browser, a connection manager, and SSH tunnels cover the daily workflow; query history is searchable and re-runnable. AI assistance is available through a command palette: natural-language-to-SQL, query explanation, formatting, and optimization, with the current schema injected as context so answers resolve against your real tables and columns. Bring your own key for Claude, OpenAI, Minimax, Z.ai, LM Studio, Claude Code CLI, or any OpenAI-compatible endpoint.

Privacy is a product decision, not a marketing line. sqail ships with zero telemetry, stores credentials locally in an encrypted SurrealDB, and only contacts AI providers when the user explicitly configures them. Schema metadata is sent with prompts; row data and credentials never are.

sqail is MIT-licensed, hosted on Codeberg with a GitHub mirror, and binaries are published at [sqail.io](https://sqail.io) for Windows, macOS, and Linux. It is free forever — no account, no freemium trap, no feature gates.
