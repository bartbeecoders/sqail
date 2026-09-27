# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**sqail** (pronounced *"snail"*) is a native SQL editor written in Rust (egui/eframe) plus **sqail-service**, an HTTPS REST gateway that holds every database connection. The editor never talks to a database directly: all queries go through the service. Engines: PostgreSQL, SQL Server, SQLite. Platforms: Linux (Omarchy/Arch first) and Windows. No macOS build.

Version 1.0 shipped as **sqail2** from a `sqail2/` subfolder. It has since been renamed to sqail and moved to the repository root. `sqail-legacy/` holds the frozen 0.x Tauri/React app, which has its own `CLAUDE.md`. Don't change it unless asked.

Canonical repo is [Codeberg](https://codeberg.org/bartbeecoders/sqail); GitHub is a mirror, and it runs CI.

## Commands

Rust workspace (edition 2024, rust-version 1.95). Every script has a `.ps1` twin for Windows.

```bash
scripts/check.sh          # fmt --check + clippy -D warnings + tests (+ cargo-deny/audit if installed)  ← run before pushing
scripts/check.sh --it     # also start the podman DBs and run the #[ignore]d integration tests (SQAIL_IT=1)
scripts/db.sh up|down|reset|status|psql|sqlcmd   # test DBs: sqail-postgres :55432, sqail-mssql :51433, SQLite in dev/data/
scripts/dev.sh            # DBs + service + UI
scripts/service.sh        # run sqail-service on https://127.0.0.1:7443 (data in .sqail/service)
scripts/ui.sh             # run the editor (config in .sqail/ui)
scripts/smoke.sh          # end-to-end curl test against all three engines

cargo test -p sqail-ui                                   # headless UI tests (egui_kittest + wgpu); screenshots in target/ui-shots/
cargo run --profile fuzz -p sqail-fuzz -- 60             # time-boxed fuzzer
SQAIL_BLESS=1 cargo test -p sqail-service --test it openapi   # regenerate docs/openapi.json
scripts/package-linux.sh / scripts/package-windows.ps1  # release packages into dist/
```

## Architecture

| Crate | Role |
|---|---|
| `sqail-proto` | REST wire types shared by service and clients (serde; OpenAPI via utoipa) |
| `sqail-service` | the gateway: axum over rustls, token auth + scopes, encrypted connection profiles in `service.db`, drivers in `engine/` (tokio-postgres, tiberius, rusqlite), sessions, NDJSON streaming, admin page (`admin/`, embedded), Windows service mode (`winsvc.rs`) |
| `sqail-client` | typed async client with certificate pinning, used by the UI |
| `sqail-ui` | the editor, binary `sqail`. `app.rs` is the top-level state; the editor, grid, sidebar, palette and dialogs are separate modules; `sql/` holds the splitter, completion, formatter and drag-and-drop SQL; `worker.rs` runs async work off the UI thread |
| `sqail-fuzz` | stable-Rust fuzzer for the parsers and the NDJSON decoder |

`vendor/tokio-postgres` is patched to expose column type OIDs on simple-query results (see `SQAIL-PATCH.md`).

The UI stores settings, history and the workspace in the OS config dir (`~/.config/sqail`, `SQAIL_CONFIG_DIR` overrides it) and tokens in the OS credential store (fallback `tokens.toml`). `settings::migrate_from_sqail2` moves 1.0 (sqail2) data on first start. The service keeps its data in `~/.local/share/sqail-service` (Linux user), `%ProgramData%\sqail\service` (Windows service), or `SQAIL_DATA_DIR`. Service env vars are `SQAIL_*`; the UI's log filter is `SQAIL_UI_LOG` so it doesn't collide with the service's `SQAIL_LOG`.

## Docs

`docs/`: user guide, Windows + SQL Server setup, operations, REST API, security, benchmarks. Update the user guide when user-visible behaviour changes. `CHANGELOG.md` gets an entry under *Unreleased* for every user-visible change.

## Conventions

- Commit style: `<area>: <imperative summary>`, lowercase, no trailing period, ≤72 chars.
- `unsafe_code` is forbidden workspace-wide; clippy runs with `-D warnings`.
- New dependencies must pass `cargo deny` (`deny.toml`) and `cargo audit` (`.cargo/audit.toml`).
- Integration tests are `#[ignore]` and gated on `SQAIL_IT=1`; they need `scripts/db.sh up`.

## Release and deployment

Tag `v<version>` (after bumping `Cargo.toml`, `packaging/arch/PKGBUILD` and the changelog). `.github/workflows/release.yml` reuses `ci.yml` for the checks, builds the Linux tarball and the Windows zip/MSI, publishes a GitHub release, uploads the files to the VPS (`/opt/sqail-releases`), and rebuilds and redeploys the portal (`sqail.portal/`, image in ACR, k3s deployment in `k8s/portal/`). The portal reads the version from `sqail.portal/src/lib/constants.ts`, which CI rewrites.
