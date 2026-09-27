# sqail

sqail (pronounced *"snail"*) is a fast, native SQL editor (Rust, egui) backed
by **sqail-service**, an HTTPS REST gateway that holds the database
connections. It supports PostgreSQL, SQL Server and SQLite and runs on Linux
(Omarchy) and Windows.

[**Download →**](https://sqail.io) · [Codeberg](https://codeberg.org/bartbeecoders/sqail) ·
[GitHub mirror](https://github.com/bartbeecoders/sqail)

> Version 1.0 was released under the name **sqail2**. The earlier Tauri app
> (0.x, with AI features and MySQL) is kept in [`sqail-legacy/`](sqail-legacy/)
> and is no longer developed.

**Docs:** [user guide](docs/user-guide.md) · [Windows + SQL Server setup](docs/windows-setup.md) · [operations](docs/operations.md)
(certificates, tokens, backups, services) · [REST API](docs/api.md) ·
[security](docs/security.md) · [benchmarks](docs/benchmarks.md) ·
[changelog](CHANGELOG.md)

## Install

| Platform | How |
|---|---|
| Linux | `sqail-<ver>-linux-x86_64.tar.gz` → `./install.sh` (per user; `--prefix /usr` system-wide) |
| Arch / Omarchy | `makepkg -si` in `packaging/arch/` |
| Windows | `sqail-<ver>-windows-x64.msi`, or the portable `.zip` |

Downloads are on [sqail.io](https://sqail.io) and the
[GitHub releases](https://github.com/bartbeecoders/sqail/releases).

Build the packages yourself with `scripts/package-linux.sh` or
`.\scripts\package-windows.ps1` (MSI needs the WiX 5 CLI:
`dotnet tool install --global wix --version 5.0.2`).

## Quick start

Prerequisites: Rust (rustup, 1.95+), podman, curl. On Windows you also need
the MSVC build tools and Podman Desktop (`podman machine init; podman machine start`).

```bash
scripts/db.sh up         # test databases (Postgres, SQL Server, SQLite)
scripts/check.sh --it    # fmt + clippy + unit and integration tests
scripts/smoke.sh         # end-to-end curl test against all three engines
scripts/dev.sh           # DBs + service + UI
```

Windows uses the same names: `.\scripts\db.ps1 up`, `.\scripts\check.ps1 -It`, …

## sqail (the editor)

`scripts/dev.sh` starts everything. On first run it sets up the local
service and pins its certificate, and the token goes into the OS credential
store. To use a remote service, choose **Service → Connect to a service…**
and enter its URL and a token. You'll be asked to confirm the certificate
fingerprint.

| Keys | Action |
|---|---|
| Ctrl+Enter | Run the statement at the cursor, or the selection |
| F5 / Ctrl+Shift+Enter | Run the whole script |
| Esc | Cancel the running query |
| Ctrl+E | Show the query plan of the statement at the cursor |
| Ctrl+Space | Complete (also opens by itself while typing) |
| Ctrl+Shift+F | Format the selection or the whole tab |
| Ctrl+Shift+P / Ctrl+P | Command palette / open a table or snippet |
| Ctrl+T / Ctrl+W | New / close tab |
| Ctrl+O / Ctrl+S / Ctrl+Shift+S | Open / save / save as |
| Ctrl+F | Find and replace |
| F2 | Edit the selected cell (in edit mode) |

Rebind keys in `keybindings.toml` in the config dir, for example
`"query.run" = "Ctrl+R"`. Command ids are listed in the command palette's
source, `crates/sqail-ui/src/commands.rs`.

Each tab has its own server session, so `BEGIN … COMMIT` works across
runs. You can also switch off **Auto-commit** for a tab. Result grids sort when you click a header. Click, or Shift+click, to
select cells, and Ctrl+C copies them as TSV. Double-click a cell to see its
full value. **Export** a result, or re-run the query straight to CSV, JSON,
Excel or SQL. **✎ Edit data** edits a single-table result in place and
applies the changes in one transaction. Right-click a table in the sidebar
for *SELECT top 100*, *Import CSV…* or *Script CREATE*. History and snippets
live in the sidebar, and open tabs come back after a restart.

UI tests run headlessly (`cargo test -p sqail-ui`) and write screenshots to
`target/ui-shots/`. The fuzzer runs time-boxed on stable Rust:
`cargo run --profile fuzz -p sqail-fuzz -- 60`.

## sqail-service

```bash
scripts/service.sh                                          # https://127.0.0.1:7443
scripts/service.sh token create --name me --scope admin     # prints a new token
scripts/service.sh token list | revoke <id>
scripts/service.sh fingerprint                              # cert fingerprint to pin
```

On first start the service creates `service.db`, `master.key` and a
self-signed certificate in its data directory. It also prints a one-time
**admin token** and a sign-in link for the **admin page** (or, without a
terminal, writes the token to `bootstrap-admin-token.txt` there).

The admin page, at `https://127.0.0.1:7443/admin/`, is built into the binary.
Everything after installing is done there: who can connect, the
certificate, SQLite folders, limits, connections, tokens, backups and the
audit log. Settings are applied with an in-process restart, and are only
saved once the service runs with them. `sqail-service admin-link` prints a
new sign-in link.

Installing is a single step with no options: `setup\Install-SqailService.cmd`
(as Administrator) on Windows, or `install.sh` on Linux (systemd user unit).
Both start the service and hand you the sign-in link. See
[docs/operations.md](docs/operations.md). The dev scripts use
`.sqail/service/` in this repo as the data directory. Every setting is also
in [`dev/sqail-service.example.toml`](dev/sqail-service.example.toml).

API reference: `GET /v1/openapi.json`, or the interactive docs at `/docs` in
debug builds. Query results stream as NDJSON, one `QueryEvent` per line:

```bash
curl -sk -H "Authorization: Bearer $TOKEN" -H 'content-type: application/json' \
  https://127.0.0.1:7443/v1/connections/$ID/query -d '{"sql":"SELECT 1"}'
```

## Layout

| Path | What |
|---|---|
| `crates/sqail-proto` | REST wire types shared by service and clients |
| `crates/sqail-service` | the gateway: auth, profiles, drivers, streaming |
| `crates/sqail-client` | typed async client with certificate pinning |
| `crates/sqail-ui` | the desktop app (egui), binary `sqail` |
| `dev/seed` | identical test schema for every engine |
| `crates/sqail-fuzz` | stable, time-boxed fuzzer for the parsers and the NDJSON decoder |
| `vendor/tokio-postgres` | tokio-postgres plus a one-field patch; see its `SQAIL-PATCH.md` |
| `packaging/` | icons, desktop entry, systemd unit, installer, PKGBUILD, WiX source |
| `docs/` | user, operations, API, security docs and `openapi.json` |

## Repository

| Path | What |
|---|---|
| `crates/`, `packaging/`, `scripts/`, `docs/`, `dev/` | sqail and sqail-service (above) |
| `sqail.portal/` | the website, [sqail.io](https://sqail.io) (Vite + React, served by nginx) |
| `k8s/portal/` | the portal's Kubernetes manifests |
| `marketing/` | brand guide and press kit |
| `Vibecoding/` | planning notes, including the rewrite plan (`PLAN.html`) |
| `sqail-legacy/` | the 0.x Tauri app, frozen; see its README |

## Releasing

1. Bump `version` in `Cargo.toml` (and `pkgver` in `packaging/arch/PKGBUILD`),
   move the `CHANGELOG.md` entries from *Unreleased* to the new version,
   regenerate `docs/openapi.json`
   (`SQAIL_BLESS=1 cargo test -p sqail-service --test it openapi`).
2. `scripts/check.sh --it` and `scripts/smoke.sh`.
3. Tag `v<version>` and push it. CI (`.github/workflows/ci.yml`) runs every
   check, then `release.yml` attaches the Linux tarball and the Windows
   zip/MSI to a GitHub release, uploads them to the download server and
   redeploys the portal with the new version.
