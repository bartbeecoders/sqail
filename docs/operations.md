# Operating sqail-service

sqail-service is a single binary with no runtime dependencies. It keeps all of
its state in one **data directory**, and it is configured by an optional TOML
file plus `SQAIL_*` environment variables (see
[`dev/sqail-service.example.toml`](../dev/sqail-service.example.toml) for
every key and its default).

```
sqail-service [--data-dir DIR] [--config FILE] <command>
  serve [--no-bootstrap-token]     run the server (the default command)
  token create --name N --scope read|query|admin
  token list | token revoke <id>
  fingerprint                      SHA-256 of the served certificate
  admin-link                       new admin token + a link that opens the admin page signed in
  backup <file-or-folder>          consistent copy of service.db (while running)
  service install|uninstall        Windows only, see below
```

## The data directory

| Platform / mode | Default location |
|---|---|
| Linux | `~/.local/share/sqail-service/` |
| Windows (per user) | `%APPDATA%\bartbeecoders\sqail-service\data\` |
| Windows service | `%ProgramData%\sqail\service\` |
| Dev scripts | `.sqail/service/` in the repository |

`--data-dir` or `SQAIL_DATA_DIR` overrides the location. The directory
contains:

| File | What | Protect |
|---|---|---|
| `service.db` (+ `-wal`, `-shm`) | Tokens (hashed), connection profiles (passwords encrypted), audit log | Back up |
| `master.key` | 256-bit key that encrypts the stored passwords (mode 0600) | **Back up separately**, never next to `service.db` copies |
| `tls/dev-cert.pem`, `tls/dev-key.pem` | Self-signed certificate, when no certificate is configured | |
| `sqail-service.toml` | Configuration, read automatically when present | |
| `bootstrap-admin-token.txt` | Only after a first start without a terminal, see below | Read, store, **delete** |
| `logs/` | Windows service log | |

Anyone who can read both `master.key` and `service.db` can decrypt every
stored database password. Keep the directory readable only by the account the
service runs as. On Linux, the service creates its files with mode 0600.
The Windows installer restricts the ACL (see below). As an alternative to the
key file, provide the key with `SQAIL_MASTER_KEY` (base64 of 32 bytes), for
example from a secrets manager.

## The first admin token

When the service starts and no admin token exists, it creates one:

* **On a terminal**, it prints the token once to stderr.
* **Under systemd, a Windows service or any redirected stderr**, it writes the
  token to `<data-dir>/bootstrap-admin-token.txt` (mode 0600) and logs where
  the file is, but not the token. Copy the token somewhere safe and delete the
  file.

Tokens are never logged. `serve --no-bootstrap-token` skips this step. sqail
uses that flag when it starts a local service, because it creates its own token
with `token create`. You can always make another admin token on the service
host with `sqail-service token create --name me --scope admin`, or with
`sqail-service admin-link`, which also prints a sign-in link for the admin
page. Access to the data dir is what authorises that.

## The admin page

The service serves a web admin page at **`https://<host>:<port>/admin/`**
(`/` redirects there). It is compiled into the binary; there is nothing to
install. Sign in with an **admin** token. The installers, `service install`
and the first start on a terminal print a *sign-in link* of the form
`https://127.0.0.1:7443/admin/#token=sq2_…`. The token is in the URL fragment,
which browsers never send to the server, and the page removes it from the
address bar straight away. The page keeps the token for the browser tab only
(`sessionStorage`).

| Page | What |
|---|---|
| Overview | status, the URL and fingerprint to give users, warnings (e.g. "only this computer can connect"), backup, restart |
| Connections | add, edit, test and delete connection profiles (tests run from the service host) |
| Tokens | create (shown once, with the URL and fingerprint to hand over), list, revoke |
| Settings | network (who can connect, port), certificate (upload your own PEM pair), SQLite folders, limits, sessions, audit |
| Audit log | every recorded event, newest first |

**Applying settings.** *Apply and restart* first checks the new settings on
the running service: the values, that the certificate files load, that the
SQLite folders exist, that a new port is free, and that the settings file is
writable. It then restarts the server in place (same process, so a Windows
service or systemd unit keeps running). The new settings are written to the
settings file (`<data-dir>/sqail-service.toml`, or `--config`) only once the
server is up with them. If it cannot start with them, it goes back to the
previous settings, and the page shows why. A restart closes open sessions
(open transactions roll back) and cancels running queries. `log_format`
changes need a real restart of the process. Keys set by an `SQAIL_*`
environment variable are shown read-only.

The file is rewritten in full when you save from the page, so comments in a
hand-edited file are lost. Uploaded certificates are stored as
`<data-dir>/tls/server-cert.pem` and `server-key.pem`.

The page and the `/v1/admin/*` API behind it can be switched off with
`admin_ui = false` (or `SQAIL_ADMIN_UI=0`); the rest of the API is
unaffected. The page's static files need no token; every call they make
does. They are served with a strict Content-Security-Policy (own scripts and
styles only, no framing).

## Tokens and scopes

| Scope | May |
|---|---|
| `read` | list profiles, browse the schema of any profile, run SQL on **read-only** profiles |
| `query` | run SQL, explain and open sessions on every profile |
| `admin` | everything, plus create, change and delete profiles, manage tokens, test unsaved profiles, read the audit log |

A `read` token is only as read-only as the profile it uses. On SQL Server that
is advisory; see [security.md](security.md#known-limitations).

Give each person or machine its own token, so the audit log tells them apart
and you can revoke one without affecting the others. `token list` shows when
each token was last used. Tokens are also managed over the API (`/v1/tokens`,
admin scope). Every request is rate-limited per token (`limits.requests_per_second`
and `burst`).

## TLS

The service only speaks HTTPS, with TLS 1.3 by default
(`tls.allow_tls12 = true` also allows 1.2).

**Self-signed (default):** on the first start the service generates a
certificate for `localhost`, `127.0.0.1`, `::1` and the host name. Clients pin
its fingerprint on first use. Tell your users the value of `sqail-service
fingerprint` so they can compare it with what sqail shows. If you delete
`tls/`, a new certificate is generated and every client has to re-pin it.

**Your own certificate:** set both `tls.cert` (PEM chain, leaf first) and
`tls.key`. Users then choose *Use system trust* in sqail. To renew, replace
the files and restart the service. Clients that pinned the old fingerprint
must re-pin, so prefer system trust whenever you have a CA-issued
certificate.

**Mutual TLS:** `tls.client_ca = "/path/ca.pem"` makes the service require a
client certificate signed by that CA, on top of the bearer token. A minimal
CA with openssl:

```bash
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -days 3650 \
  -subj "/CN=sqail clients" -keyout ca.key -out ca.pem
openssl req -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes \
  -subj "/CN=alice" -keyout alice.key -out alice.csr
openssl x509 -req -in alice.csr -CA ca.pem -CAkey ca.key -CAcreateserial -days 365 \
  -extfile <(printf 'extendedKeyUsage=clientAuth') -out alice.crt
```

Give Alice `alice.crt` and `alice.key`. She adds them as `client_cert` and
`client_key` to the service entry in her sqail `settings.toml` (see the
[user guide](user-guide.md#files-and-settings)).

## Serving other machines

The default `bind = "127.0.0.1:7443"` only accepts local connections. For a
shared gateway (steps 1, 2, 4 and 5 are all on the admin page's *Settings*):

1. Set `bind = "0.0.0.0:7443"` (or a specific interface).
2. Use a real certificate, or distribute the self-signed fingerprint through
   a channel you trust.
3. Open the port in the host firewall, but only to the networks that need it.
4. Consider `tls.client_ca`, so that a leaked token alone isn't enough.
5. Set `sqlite.allowed_dirs` deliberately. SQLite profiles can open any file
   inside those directories with the service's permissions.

## Running as a service

### Linux (systemd user unit)

`install.sh` installs `sqail-service.service` as a user unit:

```bash
systemctl --user enable --now sqail-service
journalctl --user -u sqail-service          # JSON log lines
loginctl enable-linger "$USER"              # keep it running while logged out
```

The unit runs with `NoNewPrivileges`, `PrivateTmp`, `ProtectSystem=full` and
`UMask=0077`. Because of `PrivateTmp`, SQLite databases under `/tmp` are not
visible to the service. For a system-wide unit, copy the file to
`/etc/systemd/system/`, add `User=`/`Group=` for a dedicated account, and point
`--data-dir` at a directory that account owns.

### Windows service

**[windows-setup.md](windows-setup.md)** walks through the whole thing:
installation with `setup\Install-SqailService.ps1`, SQL Server preparation,
tokens, certificates and troubleshooting. What follows is the underlying
command. From an elevated prompt:

```powershell
sqail-service.exe service install          # registers, restricts the data dir, starts it
sqail-service.exe service install --no-start
sqail-service.exe service uninstall        # stops and removes it; data stays
```

* The service runs as **LocalService**, starts automatically, and keeps its
  data in `%ProgramData%\sqail\service` (pass `--data-dir` / `--config` to
  `install` to change them).
* `install` removes inherited permissions from the data directory. Only
  SYSTEM, Administrators and LocalService have access afterwards.
* `install` prints the first admin token on your console. Nothing is written
  to disk.
* Logs are written to `<data-dir>\logs\sqail-service.log`. Service start and
  stop events also go to the Windows event log.
* **Windows integrated auth to SQL Server:** LocalService authenticates to
  other machines anonymously. To use integrated auth against a remote SQL
  Server, run the service as a domain account or gMSA that has a login there:
  `sc.exe config sqail-service obj= "DOMAIN\svc-sqail$"`. Then grant that
  account modify rights on the data directory.
* sqail's *Use the local service* creates tokens in the **per-user** data
  directory. With the machine-wide service installed, create a token instead
  (`sqail-service --data-dir "%ProgramData%\sqail\service" token create
  --name alice` in an elevated prompt) and use *Connect to a service…* with
  `https://127.0.0.1:7443`.

## Backups and restore

`service.db` runs in WAL mode, so don't copy it with `cp` while the service
is running. Use the built-in consistent backup, which is safe while running.
Given a folder, it writes `service-<timestamp>.db` into it:

```bash
sqail-service backup /srv/backup/sqail/
```

Store `master.key` separately: in a password manager, or in a different
backup set from the one that holds `service.db`. A `service.db` without its
key still restores tokens, profiles and the audit log, but every stored
password is lost and has to be re-entered. To restore, stop the service, put
back `service.db` and `master.key`, remove any stale `-wal`/`-shm` files, and
start it again.

The master key can't be rotated in place yet. To rotate it, re-create the
profiles on a fresh data directory.

## Audit log

Every token, profile, query (`audit.log_sql`, truncated to
`audit.max_sql_len`), explain and session event is recorded with the actor,
target, duration and outcome. Row values are never recorded. Read it with
`GET /v1/audit?limit=500&before=<id>` (admin scope). There is no automatic
retention yet. To prune old entries:

```bash
sqlite3 service.db "DELETE FROM audit_log WHERE at < '2026-01-01'; VACUUM;"
```

## Logging

`SQAIL_LOG` takes a tracing filter (default
`info,tower_http=info,tiberius=warn,tokio_postgres=warn`). `log_format = "json"`
(the systemd unit sets it) emits one JSON object per line. Request logs include
the method, path, status, latency and `x-request-id`. They never include SQL
parameters, row values, passwords or tokens; `crates/sqail-service/tests/log_leak.rs`
checks this at TRACE level.

## Upgrading

Stop the service, replace the binary and start it again. `service.db`
migrates itself on start. Read `CHANGELOG.md` for anything that needs manual
steps.
