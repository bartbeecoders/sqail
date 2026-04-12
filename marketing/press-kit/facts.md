# Facts

Verifiable claims — everything here is checkable against the repo, releases, or running the app. Numbers update with each release; check `releases.json` for the current version.

## Product

| Claim | Source |
|---|---|
| Cross-platform: Windows, macOS, Linux | CI builds all three from `scripts/build-*.sh` |
| Built on Tauri v2 | `src-tauri/Cargo.toml` |
| Frontend: React + TypeScript + Vite | `package.json` |
| Editor: Monaco (same as VS Code) | `@monaco-editor/react` in `package.json` |
| Supported databases: PostgreSQL, MySQL, SQLite, Microsoft SQL Server | `src-tauri/src/` drivers |
| Target binary size: under 20 MB | Confirmed in release artifacts |
| License: MIT | `LICENSE` at repo root |
| Canonical repo: Codeberg | `codeberg.org/bartbeecoders/sqail` |
| Mirror: GitHub | `github.com/bartbeecoders/sqail` |

## AI providers supported

Bring-your-own-key for all of these:

- Claude (Anthropic)
- OpenAI
- Minimax
- Z.ai
- LM Studio (local)
- Claude Code CLI (local)
- Any OpenAI-compatible endpoint

## Privacy claims

- Zero telemetry — confirmed by searching the repo for any phone-home code
- Credentials stored in a local encrypted SurrealDB — see `src-tauri/src/` storage layer
- AI providers only called when the user configures a key — no implicit network egress at startup
- Prompts include schema metadata (table and column names); never row data or credentials

## History

- First commit: see `git log --reverse`
- First public release: see `RELEASES.md`
- Current version: see `releases.json` and the portal's `Changelog` section

## What sqail is *not*

This list matters as much as the list of what it is. Honesty here prevents false expectations.

- Not an enterprise tool: no SSO, no audit logs, no centrally managed licensing
- No hosted cloud version (the product is the desktop app)
- Not designed for Oracle, Snowflake, BigQuery, or DynamoDB (yet — demand-driven)
- No AI features in the cloud; all prompts go directly from your machine to the provider you chose
