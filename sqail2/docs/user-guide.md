# sqail2 user guide

sqail2 is a native SQL editor for PostgreSQL, SQL Server and SQLite. It never
talks to a database itself. Every query goes through **sqail-service**, an
HTTPS gateway that holds the connection profiles and credentials. That service
can run on your own machine (the default) or on a server your team shares.

## Install

**Linux:** unpack `sqail2-<version>-linux-x86_64.tar.gz` and run
`./install.sh`. Both binaries go to `~/.local/bin`, and a launcher entry and
icon are added. `./install.sh --prefix /usr` installs system-wide (as root),
and `./install.sh --uninstall` removes everything again. On Arch/Omarchy you
can also build the package from `packaging/arch/PKGBUILD`.

**Windows:** run the MSI, or unzip the portable
`sqail2-<version>-windows-x64.zip` anywhere and start `sqail2.exe`.

## First start

sqail2 offers two choices:

* **Use the local service**: sqail2 starts `sqail-service` from next to its
  own executable, creates an admin token for you, pins the service's
  certificate and stores the token in the OS credential store (Secret Service
  on Linux, Credential Manager on Windows). There is nothing else to set up.
* **Connect to a service…**: enter the service URL (for example
  `https://sql-gw.example.com:7443`) and a token from its administrator. When
  the certificate is self-signed, sqail2 shows its SHA-256 fingerprint. Accept
  it only if it matches what the administrator gave you (`sqail-service
  fingerprint`). From then on sqail2 refuses any other certificate for that
  URL. For services with a certificate from a public or company CA, choose
  *Verify with the system trust store* instead.

If the service requires client certificates (mTLS), add the certificate and
key (PEM) to the service entry in `settings.toml`; see
[Files](#files-and-settings).

## Connections

**+ Add** next to *Connections* in the sidebar (or *Connections → New
connection…*) opens the connection form. Profiles are stored **on the service**,
so everyone using that service with a suitable token sees the same list.
Passwords are write-only: the service encrypts them and never sends them back.

| Engine | Notes |
|---|---|
| PostgreSQL | *SSL mode* works like libpq's: `prefer` by default. Use `verify-full` when the server has a proper certificate. |
| SQL Server | SQL login or Windows integrated auth (integrated only works when the service runs on Windows). `Encrypt = Required` is the default; tick *Trust server certificate* only for test servers. Named instances are found through SQL Browser; the host field also accepts `SERVER\INSTANCE` and `SERVER,PORT`. Step-by-step: [windows-setup.md](windows-setup.md). |
| SQLite | The path is a file on the **service's** machine and must be inside one of the service's `sqlite.allowed_dirs`. |

Give production profiles a colour and an environment tag. Both show in the
status bar while that connection is active. **Read-only** profiles also can't
be edited in the grid. How strictly "read-only" is enforced depends on the
engine:

* **PostgreSQL:** the session runs with `default_transaction_read_only = on`.
* **SQLite:** the file is opened read-only.
* **SQL Server:** the connection declares `ApplicationIntent=ReadOnly`, which
  routes it to a readable secondary in an availability group but doesn't block
  writes on a plain server. For a hard guarantee, use a login that only has
  read permissions.

## Writing and running SQL

| Keys | Action |
|---|---|
| Ctrl+Enter | Run the statement under the cursor, or the selection |
| F5 / Ctrl+Shift+Enter | Run the whole script |
| Esc | Cancel the running query |
| Ctrl+E | Show the plan of the statement under the cursor |
| Ctrl+Space | Completion (also opens by itself while you type) |
| Ctrl+Shift+F | Format the selection or the whole tab |
| Ctrl+Shift+P | Command palette: every action, searchable |
| Ctrl+P | Quick open: jump to a table or snippet |
| Ctrl+T / Ctrl+W | New / close tab |
| Ctrl+PageDown / Ctrl+PageUp | Next / previous tab |
| Ctrl+O / Ctrl+S / Ctrl+Shift+S | Open / save / save as (`.sql` files) |
| Ctrl+F | Find and replace |
| Ctrl+= / Ctrl+- | Larger / smaller editor font |

The script splitter understands comments, string and identifier quoting,
Postgres dollar quotes and SQL Server `GO` lines, so *Run statement* picks
exactly the statement you're in. Completion knows keywords, schemas, tables,
columns (including aliases like `o.` after `FROM orders o`) and functions of
the current connection.

**Transactions:** every tab has its own server session. `BEGIN … COMMIT`
spans runs, and the status bar shows **in transaction** while one is open.
Switch off **Auto-commit** in the toolbar and sqail2 sends `BEGIN` before the
next run whenever no transaction is open. Finish with **Commit** or
**Rollback** in the toolbar.
Closing a tab with an open transaction asks whether to commit or roll back.
Idle sessions close after 30 minutes on the service, which rolls them back.

**Plans:** *Explain* (Ctrl+E) shows the estimated plan as a tree, with cost
and row estimates and the most expensive nodes highlighted. *Explain analyze*
actually runs the statement to get real timings and row counts, inside a
transaction that is **always rolled back**, so it is safe for `UPDATE` and
`DELETE` as well. The *Raw* toggle shows the engine's own JSON or XML plan.

## Results

* Results stream in. You can scroll, sort and copy while rows are still
  arriving, and the grid stays smooth with millions of rows. Each result set is
  capped at *Query → Row limit* (100 000 by default). When a result hits the
  cap, its tab reads like `Result 1 (100,000+)` in the warning colour.
* Click a header to sort. Click, or Shift+click, to select cells, and Ctrl+C
  copies them as TSV (pastes straight into a spreadsheet). Double-click a cell
  to see its full value (long text, JSON, binary as hex).
* **Export** saves the rows in the grid, or re-runs the query and streams
  every row straight to a file, as CSV, JSON, Excel (.xlsx) or SQL `INSERT`s.
  The re-run ignores your row limit and is only bounded by the
  service's `limits.max_rows` (5 000 000 by default).
* **✎ Edit data** is available when a result comes from a single table with a
  primary key. Edit cells (double-click or F2), add rows and delete rows. The
  changes are shown as SQL before **Apply** runs them in one transaction.
* Messages (`RAISERROR`, `RAISE NOTICE`, `PRINT`) and row counts appear in the
  *Messages* tab.

## Sidebar

* **Connections:** a tree of schemas, tables, views, columns, indexes, foreign
  keys and routines, loaded lazily. Right-click a table for *SELECT top 100*,
  *Import CSV…*, *Script CREATE* or *Copy name*.
* **History:** every query you ran, with its connection, duration and row
  count. Filter it, open an entry in a new tab, or clear it. The last 5 000
  entries are kept.
* **Snippets:** save a selection with *Save as snippet…*, then insert it
  from the sidebar or with Ctrl+P.

Open tabs, including unsaved text, are restored when you start sqail2 again.

## Files and settings

sqail2 keeps its settings in `~/.config/sqail2/` on Linux and
`%APPDATA%\bartbeecoders\sqail2\config\` on Windows
(`SQAIL2_CONFIG_DIR` overrides it):

| File | Contents |
|---|---|
| `settings.toml` | Services, theme, font size, row limit, autocomplete, formatter options |
| `keybindings.toml` | Your shortcut overrides |
| `history.jsonl`, `snippets.json`, `workspace.json` | History, snippets, open tabs |
| `tokens.toml` | Only when the OS credential store is unavailable (mode 0600) |

Example `keybindings.toml`. A list binds several shortcuts to one command,
and an empty list `[]` removes a default shortcut. Problems (an unknown command
or a shortcut that can't be parsed) are reported once when sqail2 starts.

```toml
"query.run" = "Ctrl+R"
"query.run_script" = ["F5", "Ctrl+Shift+R"]
"view.toggle_theme" = "Ctrl+Shift+T"
"view.font_bigger" = []
```

Command ids: `query.run`, `query.run_script`, `query.cancel`,
`query.explain`, `query.explain_analyze`, `transaction.commit`,
`transaction.rollback`, `transaction.toggle_autocommit`, `tab.new`,
`tab.close`, `tab.next`, `tab.previous`, `file.open`, `file.save`,
`file.save_as`, `edit.find`, `edit.format`, `edit.save_snippet`,
`view.command_palette`, `view.quick_open`, `view.font_bigger`,
`view.font_smaller`, `view.toggle_theme`, `view.connections`,
`view.history`, `view.snippets`, `connection.new`, `connection.refresh`,
`service.connect`.

Client certificate for a service that requires mTLS, in `settings.toml`:

```toml
[[services]]
name = "team gateway"
url = "https://sql-gw.example.com:7443"
client_cert = "/home/me/.config/sqail2/me.crt.pem"
client_key = "/home/me/.config/sqail2/me.key.pem"
```

## Troubleshooting

* **"certificate fingerprint mismatch" on connect:** the service now presents
  a different certificate than the one you pinned. This is expected after the
  administrator replaced it. Otherwise treat it as an attack. After checking
  the new fingerprint, choose *Service → Forget this service*, then connect
  again.
* **Logs:** start sqail2 from a terminal with `SQAIL2_LOG=debug`.
