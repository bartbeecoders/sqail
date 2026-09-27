# sqail

sqail (pronounced *"snail"*) is a fast, native SQL editor for **PostgreSQL**,
**SQL Server** and **SQLite**, on **Windows** and **Linux**. It streams millions
of rows without blinking, and it keeps database passwords off your desktop.

[**Download →**](https://sqail.io) · [GitHub releases](https://github.com/bartbeecoders/sqail/releases) ·
[Codeberg](https://codeberg.org/bartbeecoders/sqail) · [GitHub mirror](https://github.com/bartbeecoders/sqail)

> Version 1.0 was released under the name **sqail2**. The earlier Tauri app
> (0.x, with AI features and MySQL) is kept in [`sqail-legacy/`](sqail-legacy/)
> and is no longer developed.

**Contents:** [How it works](#how-it-works) ·
[Windows: just me](#windows-just-me) ·
[Windows Server: shared gateway](#windows-server-a-shared-gateway) ·
[Linux](#linux) · [Connection settings](#connection-settings) ·
[Using the editor](#using-the-editor) · [Development](#development)

**Full docs:** [user guide](docs/user-guide.md) ·
[Windows + SQL Server setup (in depth)](docs/windows-setup.md) ·
[operations](docs/operations.md) · [security](docs/security.md) ·
[REST API](docs/api.md) · [benchmarks](docs/benchmarks.md) ·
[changelog](CHANGELOG.md)

## How it works

sqail comes in two parts, and both are in every download:

* **sqail** (`sqail.exe`) is the editor people use.
* **sqail-service** (`sqail-service.exe`) is a small HTTPS gateway. It holds
  the database connections and their passwords (encrypted), and every query
  goes through it. The editor never talks to a database directly. It only
  holds a token.

```
 your PC / users' PCs          gateway (any machine)              databases
┌──────────────┐  HTTPS :7443  ┌──────────────────┐            ┌──────────────┐
│ sqail        │ ─────────────▶│ sqail-service    │───────────▶│ SQL Server   │
│ (the editor) │  token +      │ admin page at    │            │ PostgreSQL   │
└──────────────┘  pinned cert  │ /admin/          │            │ SQLite files │
                               └──────────────────┘            └──────────────┘
```

There are two ways to set it up:

| | **Just me** | **Shared gateway** |
|---|---|---|
| For | One person on their own PC | A team, or anyone who shouldn't hold the DB passwords |
| sqail-service runs | On your PC, started by sqail | On a server, as a Windows service or systemd unit |
| Database passwords live | On your PC (encrypted) | On the server only; users never see them |
| Setup time | 1 minute | 15–30 minutes |

---

## Windows: just me

1. Download `sqail-<version>-windows-x64.msi` from
   [sqail.io](https://sqail.io) or the
   [releases page](https://github.com/bartbeecoders/sqail/releases) and run
   it. (Prefer no installer? Unzip `sqail-<version>-windows-x64.zip` anywhere.)
2. Start **sqail** from the Start menu.
3. On the first-start dialog choose **Use the local service**. sqail starts
   `sqail-service.exe` from its own folder, creates a token for you, pins
   the service's certificate and stores the token in Windows Credential
   Manager.
4. Click **+ Add** next to *Connections* in the sidebar, choose the engine
   and fill in the form (see [Connection settings](#connection-settings)).
5. Click **Test**, then **Save**. The database appears in the sidebar.
6. Open a tab (Ctrl+T), write SQL, and press **Ctrl+Enter**.

---

## Windows Server: a shared gateway

The gateway runs as a Windows service on a server. It holds the database
connections; your users only get a token. This is the short version.
[docs/windows-setup.md](docs/windows-setup.md) has every option, Windows
authentication with a gMSA, and troubleshooting.

**You need:**

* Windows Server 2016 or later (Windows 10/11 also works), with
  Administrator rights.
* Network access from the gateway **to** your databases (SQL Server: TCP
  1433; PostgreSQL: TCP 5432), and **from** your users' PCs to the gateway
  (TCP 7443).
* Someone who can create a login on each database (a DBA).

### Step 1. Prepare the database

Create a login for the gateway with only the rights your users need.

**SQL Server** (in SSMS, as a sysadmin). TCP/IP must be enabled in *SQL
Server Configuration Manager*, and the server must allow *SQL Server and
Windows Authentication mode* for a SQL login:

```sql
CREATE LOGIN sqail_reader WITH PASSWORD = 'use-a-long-random-password', CHECK_POLICY = ON;
USE Sales;
CREATE USER sqail_reader FOR LOGIN sqail_reader;
ALTER ROLE db_datareader ADD MEMBER sqail_reader;   -- read all tables
GRANT SHOWPLAN TO sqail_reader;                      -- query plans (Ctrl+E)
GRANT VIEW DEFINITION TO sqail_reader;               -- schema tree, "Script CREATE"
-- Read/write instead: ALTER ROLE db_datawriter ADD MEMBER sqail_reader; GRANT EXECUTE TO sqail_reader;
```

**PostgreSQL** (as a superuser; make sure `pg_hba.conf` allows the gateway's
address):

```sql
CREATE ROLE sqail_reader LOGIN PASSWORD 'use-a-long-random-password';
GRANT CONNECT ON DATABASE sales TO sqail_reader;
GRANT pg_read_all_data TO sqail_reader;              -- PostgreSQL 14+; read everything
-- Read/write instead: GRANT pg_write_all_data TO sqail_reader;
```

### Step 2. Install the service

1. Copy the MSI to the server and run it. It installs to
   `C:\Program Files\sqail`.
   *Using the zip instead?* Right-click it → *Properties* → tick **Unblock**
   **before** unzipping, or Windows blocks the setup scripts.
2. Open `C:\Program Files\sqail\setup` (or the zip's `setup` folder),
   right-click **`Install-SqailService.cmd`** → **Run as administrator**.
   There are no questions. The script:
   * registers the **sqail-service** Windows service (runs as *Local
     Service*, starts automatically, restarts after a crash), with its data
     in `C:\ProgramData\sqail\service`, locked down to SYSTEM,
     Administrators and the service;
   * adds a Windows Firewall rule for the Domain and Private networks, local
     subnet;
   * prints the first **admin token** once and **opens the admin page**
     signed in.
3. **Save the admin token** in your password manager. You need it to sign in
   to the admin page later. If you lose it, run this in an elevated prompt
   to get a new sign-in link:
   `"C:\Program Files\sqail\sqail-service.exe" --data-dir "C:\ProgramData\sqail\service" admin-link`

### Step 3. Configure it on the admin page

The admin page is at **`https://127.0.0.1:7443/admin/`** on the server. Your
browser warns once about the self-signed certificate; continue to the page.
Under **Settings**:

1. **Network → Other computers too.** Out of the box the service only
   accepts connections from the server itself.
2. **Certificate → My own certificate** (recommended for teams): upload a
   PEM certificate and key from your company CA, issued for the name users
   will type (e.g. `sqail-gw.corp.local`). Users then don't have to compare
   fingerprints, and renewing the certificate needs nothing from them.
   Converting a `.pfx` is described in
   [windows-setup.md](docs/windows-setup.md#b2-install-the-gateway-service).
   Without it, the service uses a self-signed certificate that users pin on
   first use.
3. **SQLite** (only if you use SQLite): the folders SQLite databases may be
   in. The service account needs access:
   `icacls D:\sqlite /grant "*S-1-5-19:(OI)(CI)M"`.
4. Click **Apply and restart**. The service checks the new settings first
   and keeps the old ones if they don't work.

### Step 4. Add your database connections

On the admin page: **Connections → New connection**. Choose the engine, fill
in the form (see [Connection settings](#connection-settings)), click
**Test**, then **Save**. The test runs from the gateway, so it checks exactly
the path your users' queries take.

Give production connections an **Environment** (e.g. `prod`) and a
**Colour** (e.g. `#c0392b`). Both show in the status bar of sqail while that
connection is active. Tick **Read-only** for connections that should never
write.

For scripted setups of SQL Server connections there is
`setup\Add-SqailSqlServer.ps1`
([examples](docs/windows-setup.md#b3-register-your-sql-server-databases)).

### Step 5. Create a token for each user

On the admin page: **Tokens → New token**. Enter a name (a person or a
team), choose a scope, and click **Create**. The token is shown **once**,
together with the URL and certificate fingerprint to send along. Send the
token through a secure channel.

| Scope | Can |
|---|---|
| `read` | browse schemas; run SQL only on **read-only** connections |
| `query` | run SQL, Explain and transactions on every connection (the usual choice) |
| `admin` | also manage connections, tokens and settings, and read the audit log |

Give each person their own token, so the audit log tells them apart and you
can revoke one without affecting anyone else.

### Step 6. Connect the users

On each user's PC:

1. Install sqail with the MSI.
2. Start sqail. In the first-start dialog (later: *Service → Connect to a
   service…*), under **Another service**, enter the URL, e.g.
   `https://sqail-gw.corp.local:7443`, and the token, then click **Connect**.
3. With a self-signed certificate, compare the fingerprint sqail shows with
   the one you received, then click **Trust and connect**. With your own
   certificate, click **Use system trust**.
4. The gateway's connections appear in the sidebar. The token is stored in
   the user's Windows Credential Manager.

### Step 7. Check that it works

- [ ] `Get-Service sqail-service` on the server shows **Running**.
- [ ] `curl.exe -sk https://127.0.0.1:7443/v1/health` on the server returns `{"status":"ok",…}`.
- [ ] From a user's PC: `Test-NetConnection sqail-gw.corp.local -Port 7443` succeeds.
- [ ] From the server: `Test-NetConnection sqlserver01 -Port 1433` succeeds.
- [ ] In sqail with a user's token, the connection is listed and a query runs.

When something fails, see
[Troubleshooting](docs/windows-setup.md#troubleshooting).

### Running the gateway

* **Backups:** *Overview → Back up* on the admin page, or
  `sqail-service.exe --data-dir C:\ProgramData\sqail\service backup D:\Backup\sqail`
  (safe while the service runs). Keep a copy of
  `C:\ProgramData\sqail\service\master.key` **somewhere else**, such as a
  vault. It decrypts the stored database passwords.
* **Upgrade:** `Stop-Service sqail-service`, run the new MSI,
  `Start-Service sqail-service`. Data migrates automatically.
* **Remove:** `setup\Uninstall-SqailService.cmd` as administrator. The data
  is kept.
* **Audit log, limits, timeouts:** on the admin page. Details are in
  [docs/operations.md](docs/operations.md).

---

## Linux

### Install

**Tarball** (any x86_64 distribution, Wayland or X11):

```bash
tar xzf sqail-<version>-linux-x86_64.tar.gz
cd sqail-<version>-linux-x86_64
./install.sh                  # into ~/.local, no root needed
# sudo ./install.sh --prefix /usr   system-wide
# ./install.sh --uninstall          remove it again (your data is kept)
```

This installs `sqail` and `sqail-service`, a launcher entry, and
sqail-service as a **systemd user unit**, which it starts. It then prints a
sign-in link for the admin page. Add `--no-service` to skip the unit.

**Arch / Omarchy:** build the package from this repository with
`cd packaging/arch && makepkg -si`.

### Just me

1. Start **sqail** from your launcher (or run `sqail`).
2. Choose **Use the local service**. sqail uses the running
   sqail-service, creates a token for you and stores it in the Secret
   Service (GNOME Keyring, KWallet). Without a keyring, it falls back to
   `~/.config/sqail/tokens.toml`, readable only by you.
3. Click **+ Add** next to *Connections*, fill in the form (see
   [Connection settings](#connection-settings)), then **Test** and **Save**.
4. Write SQL and press **Ctrl+Enter**.

### A Linux server as a shared gateway

The steps match the [Windows Server gateway](#windows-server-a-shared-gateway),
with these differences:

1. **Install** on the server with `./install.sh` as the account that should
   run the service, then keep it running after you log out:
   `loginctl enable-linger "$USER"`.
2. **Open the admin page.** `install.sh` prints a sign-in link for
   `https://127.0.0.1:7443/admin/#token=…`. On a headless server, forward the
   port from your PC first, then open that link in your local browser:
   `ssh -L 7443:127.0.0.1:7443 you@server`. Keep the token; for a new link
   later, run `sqail-service admin-link`.
3. **Settings → Network → Other computers too**, optionally your own
   certificate, then **Apply and restart**, as in
   [step 3](#step-3-configure-it-on-the-admin-page).
4. **Open the firewall** for your users' network only, e.g.
   `sudo ufw allow from 10.0.0.0/8 to any port 7443 proto tcp`.
5. Add connections, create tokens and connect users as in
   [steps 4–6](#step-4-add-your-database-connections).

The service's data lives in `~/.local/share/sqail-service/`. Check it with
`systemctl --user status sqail-service` and `journalctl --user -u sqail-service`.
For a system-wide unit under a dedicated account, see
[operations.md](docs/operations.md#linux-systemd-user-unit).

---

## Connection settings

The same form is used in sqail (**+ Add**) and on the admin page
(**Connections → New connection**).

| Engine | Field | What to enter |
|---|---|---|
| **SQL Server** | Host | `sqlserver01`, a FQDN or an IP. `SERVER\INSTANCE` and `SERVER,PORT` work as in SSMS. |
| | Port / Instance | `1433` for a default instance. For a named instance, the instance name (needs the SQL Server Browser service, UDP 1434). |
| | Database | e.g. `Sales`; empty means the login's default database |
| | Authentication | *SQL login* (user + password) or *Windows (integrated)*, as the account the service runs as |
| | Encryption | *Required* (default). For test servers with a self-signed certificate, tick **Trust server certificate**. |
| **PostgreSQL** | Host / Port | e.g. `pg01.corp.local`, `5432` |
| | Database, User, Password | e.g. `sales`, `sqail_reader` |
| | SSL mode | `prefer` (default) works like libpq. Use `verify-full` when the server has a proper certificate. |
| **SQLite** | File | A path **on the service's machine**, inside one of the SQLite folders allowed under *Settings*. |
| all | Environment, Colour, Folder | Optional labels: shown in the status bar, and used to group connections in the sidebar. |
| all | Read-only | PostgreSQL and SQLite enforce it. On SQL Server it is only advisory, so use a login without write rights. |

---

## Using the editor

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

Rebind keys in `keybindings.toml` in the config folder
(`~/.config/sqail/` on Linux, `%APPDATA%\bartbeecoders\sqail\config\` on
Windows), for example `"query.run" = "Ctrl+R"`.

* **Transactions:** each tab has its own server session, so `BEGIN …
  COMMIT` works across runs. Switch off **Auto-commit** to get an implicit
  transaction, and finish it with **Commit** or **Rollback**.
* **Results:** click a header to sort; click or Shift+click to select cells,
  and Ctrl+C copies them as TSV. Double-click a cell to see its full value.
* **Export** to CSV, JSON, Excel or SQL, or re-run the query straight to a
  file.
* **✎ Edit data** edits a single-table result in place and applies the
  changes in one transaction.
* **Sidebar:** right-click a table for *SELECT top 100*, *Import CSV…* or
  *Script CREATE*, or drag it into the editor. History and snippets are
  there too, and open tabs come back after a restart.

Everything else is in the [user guide](docs/user-guide.md).

---

## Development

Prerequisites: Rust (rustup, 1.95+), podman, curl. On Windows you also need
the MSVC build tools and Podman Desktop
(`podman machine init; podman machine start`).

```bash
scripts/db.sh up         # test databases (Postgres, SQL Server, SQLite)
scripts/check.sh --it    # fmt + clippy + unit and integration tests
scripts/smoke.sh         # end-to-end curl test against all three engines
scripts/dev.sh           # DBs + service + UI
scripts/service.sh       # only the service, on https://127.0.0.1:7443
```

Windows uses the same names: `.\scripts\db.ps1 up`, `.\scripts\check.ps1 -It`, …
The dev scripts keep their data in `.sqail/` in this repository. Every
service setting is listed in
[`dev/sqail-service.example.toml`](dev/sqail-service.example.toml).

UI tests run headlessly (`cargo test -p sqail-ui`) and write screenshots to
`target/ui-shots/`. The fuzzer runs time-boxed on stable Rust:
`cargo run --profile fuzz -p sqail-fuzz -- 60`. Build the packages with
`scripts/package-linux.sh` or `.\scripts\package-windows.ps1` (the MSI needs
the WiX 5 CLI: `dotnet tool install --global wix --version 5.0.2`).

The service's REST API is described at `GET /v1/openapi.json` and in
[docs/api.md](docs/api.md). Query results stream as NDJSON:

```bash
curl -sk -H "Authorization: Bearer $TOKEN" -H 'content-type: application/json' \
  https://127.0.0.1:7443/v1/connections/$ID/query -d '{"sql":"SELECT 1"}'
```

### Layout

| Path | What |
|---|---|
| `crates/sqail-proto` | REST wire types shared by service and clients |
| `crates/sqail-service` | the gateway: auth, profiles, drivers, streaming, admin page |
| `crates/sqail-client` | typed async client with certificate pinning |
| `crates/sqail-ui` | the desktop app (egui), binary `sqail` |
| `crates/sqail-fuzz` | stable, time-boxed fuzzer for the parsers and the NDJSON decoder |
| `vendor/tokio-postgres` | tokio-postgres plus a one-field patch; see its `SQAIL-PATCH.md` |
| `dev/` | seed schema for every engine, example service config |
| `packaging/` | icons, desktop entry, systemd unit, installer, PKGBUILD, WiX source |
| `docs/` | user, operations, API, security docs and `openapi.json` |
| `sqail.portal/`, `k8s/portal/` | the website, [sqail.io](https://sqail.io), and its Kubernetes manifests |
| `marketing/` | brand guide and press kit |
| `Vibecoding/` | planning notes, including the rewrite plan (`PLAN.html`) |
| `sqail-legacy/` | the 0.x Tauri app, frozen; see its README |

### Releasing

1. Bump `version` in `Cargo.toml` (and `pkgver` in `packaging/arch/PKGBUILD`),
   move the `CHANGELOG.md` entries from *Unreleased* to the new version, and
   regenerate `docs/openapi.json`
   (`SQAIL_BLESS=1 cargo test -p sqail-service --test it openapi`).
2. `scripts/check.sh --it` and `scripts/smoke.sh`.
3. Tag `v<version>` and push it. `release.yml` runs every check from
   `ci.yml`, attaches the Linux tarball and the Windows zip/MSI to a GitHub
   release, uploads them to the download server, and redeploys the portal.
