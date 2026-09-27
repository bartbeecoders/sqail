# Changelog

All notable changes to sqail and sqail-service. Versions follow
[Semantic Versioning](https://semver.org/); releases are tagged
`v<version>`. Version 1.0.0 was released as sqail2 (tag `sqail2-v1.0.0`);
the Tauri app that came before it (0.x) lives in `sqail-legacy/`.

## Unreleased

## 1.1.0 — 2026-09-27

### Renamed to sqail

- The editor is now called **sqail**: binary `sqail`/`sqail.exe`, packages
  `sqail-<ver>-linux-x86_64.tar.gz`, `sqail-<ver>-windows-x64.msi`/`.zip`,
  Arch package `sqail` (replaces `sqail2`). Releases are tagged `v<version>`.
- On first start sqail moves the sqail2 settings folder (`~/.config/sqail2`,
  `%APPDATA%\bartbeecoders\sqail2\config`), its window state and the tokens
  saved in the OS credential store to the new names.
- The MSI upgrades an installed sqail2 in place (same upgrade code) and now
  installs to `Program Files\sqail`. The Windows service keeps using
  `%ProgramData%\sqail2\service` when that is where its data already is.
- `install.sh` removes the old `sqail2` binary, desktop entry and icons.
- Environment variables: `SQAIL2_CONFIG_DIR` → `SQAIL_CONFIG_DIR`,
  `SQAIL2_TOKEN_STORE` → `SQAIL_TOKEN_STORE`, `SQAIL2_LOG` → `SQAIL_UI_LOG`
  (`SQAIL_LOG` stays the service's).
- Development: the repository root is now this workspace (it was `sqail2/`),
  dev data lives in `.sqail/`, and the test containers are `sqail-postgres`
  and `sqail-mssql`.

### sqail-service

- Admin page at `/admin/`, built into the binary: status and the details to
  hand to users, connections (with test), tokens, settings and the audit
  log. It signs in with an admin token; installers print a sign-in link
  (`/admin/#token=…`), and `sqail-service admin-link` makes a new one.
- Settings are changed on the admin page and applied by an in-process
  restart. They are checked first and saved only once the service runs with
  them; otherwise the previous settings stay and the page shows why. New
  `/v1/admin` API (status, settings, restart, certificate upload, backup).
- `admin_ui` setting (`SQAIL_ADMIN_UI`) switches the page and `/v1/admin` off.
- Windows: the service restarts itself after a crash.

### Installers

- `Install-SqailService.ps1` no longer takes configuration options
  (`-Network`, `-Port`, `-CertFile`/`-KeyFile`, `-SqliteFolder`, `-Force`
  are gone). It installs with safe defaults, adds a program-scoped firewall
  rule, and opens the admin page signed in. Set the network, certificate and
  SQLite folders there. Scripts that passed the old options must drop them.
- `install.sh` prints the admin page sign-in link after starting the
  systemd unit and removes the one-time token file.

### sqail

- Drag tables, views and routines from the sidebar into the editor. Dropped
  inside a statement, a table or view inserts its qualified name; on a
  blank line (or below the text) it becomes a formatted `SELECT` listing its
  columns. Procedures become `EXEC name` (SQL Server) or `CALL name()`.
- Completion after `alias.` also works for CTEs and subqueries, respects
  subquery scope when an alias is reused, and finds unqualified tables
  outside the default schema. Column names that appear in more than one
  table are offered qualified (`o.id`).

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
