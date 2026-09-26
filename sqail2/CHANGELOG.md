# Changelog

All notable changes to sqail2 and sqail-service. Versions follow
[Semantic Versioning](https://semver.org/); releases are tagged
`sqail2-v<version>`.

## 1.0.0 — 2026-09-26

First release of the Rust rewrite: a native editor (`sqail2`) and an HTTPS
gateway (`sqail-service`) that holds every database connection.

### sqail-service

- REST API over HTTPS only (rustls, TLS 1.3 by default, HTTP/2), described by
  OpenAPI at `/v1/openapi.json` ([docs/api.md](docs/api.md)).
- PostgreSQL (tokio-postgres), SQL Server (tiberius; SQL and Windows auth,
  `GO` batches) and SQLite (rusqlite, restricted to `sqlite.allowed_dirs`).
- Query results stream as NDJSON with flat memory at 1.4–2 M rows/s; round
  trip for `SELECT 1` ≈ 0.2 ms. Per-result row caps, timeouts, cancellation
  (explicit or on client disconnect).
- Sessions: dedicated connections for transactions, temp tables and `SET`
  options; idle sessions roll back and close.
- Explain / explain analyze for all three engines, normalised into one plan
  tree; analyze always runs inside a rolled-back transaction.
- Schema browsing (databases, schemas, tables, columns, indexes, foreign
  keys, routines) and best-effort DDL.
- Security: scoped API tokens (`read` < `query` < `admin`) stored as SHA-256
  hashes; database passwords encrypted with AES-256-GCM; audit log without
  row values; per-token rate limiting; optional mutual TLS; logs verified
  free of secrets at TRACE level ([docs/security.md](docs/security.md)).
- Runs as a systemd user unit (hardened) or a Windows service
  (`sqail-service service install`); first-start admin token is never logged
  ([docs/operations.md](docs/operations.md)).
- SQL Server named instances via SQL Browser; the host field accepts
  `SERVER\INSTANCE` and `SERVER,PORT` as in SSMS; certificate failures say
  what to change in the profile.
- `sqail-service backup <file|folder>`: consistent online backup of
  `service.db`.

### sqail2 (editor)

- egui/wgpu desktop app for Linux (Wayland/X11) and Windows.
- SQL editor with highlighting, statement-aware run (Ctrl+Enter), script run,
  schema-aware completion, formatter, find/replace, tabs restored on start,
  remappable shortcuts and a command palette.
- Virtualised result grid (millions of rows) with sorting, cell selection
  and copy, value viewer, truncation indicator; export to CSV, JSON, Excel
  and SQL, including streaming the whole query to a file.
- Inline data editing for single-table results, applied in one transaction;
  CSV import.
- Per-tab sessions, auto-commit toggle, commit/rollback, protection against
  closing a tab with an open transaction.
- Plan viewer with hot-spot highlighting and raw plan view.
- Schema tree, query history and snippets in the sidebar.
- Trust on first use with certificate pinning, or OS trust store; tokens in
  the OS credential store; one-click local service setup.

### Project

- Podman test databases (`scripts/db.sh`), check gate
  (`scripts/check.sh`: fmt, clippy, tests, cargo-deny, cargo-audit),
  curl smoke test, benchmarks, stable fuzzer (`sqail-fuzz`).
- Packages: Linux tarball with installer, Arch PKGBUILD, Windows zip and MSI.
  The Windows package carries a setup kit (`setup\`: install/uninstall the
  service, create tokens, register SQL Server databases) and a step-by-step
  guide, [docs/windows-setup.md](docs/windows-setup.md) (shipped as
  `SETUP.md`).
- CI on GitHub Actions: Linux + Windows checks, supply-chain audit,
  integration tests against real databases, fuzzing, release builds on tags.
