# sqail user guide

sqail is a native SQL editor for PostgreSQL, SQL Server and SQLite. It never
talks to a database itself. Every query goes through **sqail-service**, an
HTTPS gateway that holds the connection profiles and credentials. That service
can run on your own machine (the default) or on a server your team shares.

## Install

**Linux:** unpack `sqail-<version>-linux-x86_64.tar.gz` and run
`./install.sh`. Both binaries go to `~/.local/bin`, and a launcher entry and
icon are added. `./install.sh --prefix /usr` installs system-wide (as root),
and `./install.sh --uninstall` removes everything again. On Arch/Omarchy you
can also build the package from `packaging/arch/PKGBUILD`.

**Windows:** run the MSI, or unzip the portable
`sqail-<version>-windows-x64.zip` anywhere and start `sqail.exe`.

## First start

sqail offers two choices:

* **Use the local service**: sqail starts `sqail-service` from next to its
  own executable, creates an admin token for you, pins the service's
  certificate and stores the token in the OS credential store (Secret Service
  on Linux, Credential Manager on Windows). There is nothing else to set up.
* **Connect to a service…**: enter the service URL (for example
  `https://sql-gw.example.com:7443`) and a token from its administrator. When
  the certificate is self-signed, sqail shows its SHA-256 fingerprint. Accept
  it only if it matches what the administrator gave you (`sqail-service
  fingerprint`). From then on sqail refuses any other certificate for that
  URL. For services with a certificate from a public or company CA, choose
  *Verify with the system trust store* instead.

If the service requires client certificates (mTLS), add the certificate and
key (PEM) to the service entry in `settings.toml`; see
[Files](#files-and-settings).

## Connections

**+ Connection** under *Connections* in the sidebar (or *Connections → New
connection…*) opens the connection form. Profiles are stored **on the service**,
so everyone using that service with a suitable token sees the same list.
Passwords are write-only: the service encrypts them and never sends them back.
For PostgreSQL and SQL Server, fill in the server and login first, then click
**⏷** next to *Database* to choose from the databases that login can access
(you can still type a name). The admin page's *List* button does the same.
Listing needs an admin token, as *Test* does for an unsaved connection.

Double-click a connection in the sidebar to edit it; *New query tab* is in
its right-click menu. *Duplicate…* in the same menu opens the form filled in
from that connection, named “… (copy)”. Leave the password empty to copy the
stored password (and PostgreSQL client key) from the original.

### Folders

**+ Folder** under *Connections* adds a folder and lets you name it right
away. Drag a connection onto a folder, or onto a connection inside one, to
move it there; while you drag a connection that is in a folder, a strip at
the bottom of the tree takes it out again. *Move to folder* in a
connection's right-click menu does the same without dragging.

Right-click a connection or a folder for **Rename** to rename it in place:
Enter or clicking elsewhere saves, Esc cancels. A folder's menu also has
*New connection here…*, and *Delete folder* when it is empty.

Folders are stored on the service with the connections in them, so everyone
sees the same ones; renaming a folder updates each of its connections. A
folder without connections (new, or emptied by moving its connections out)
exists only in your own sqail, until you move a connection into it.

| Engine | Notes |
|---|---|
| PostgreSQL | *SSL mode* works like libpq's: `prefer` by default. CA and client certificates are described in [PostgreSQL TLS](#postgresql-tls). |
| SQL Server | SQL login, Windows integrated auth (integrated only works when the service runs on Windows), or Microsoft Entra ID for Azure SQL (see [below](#azure-sql-database)). `Encrypt = Required` is the default; tick *Trust server certificate* only for test servers. Named instances are found through SQL Browser; the host field also accepts `SERVER\INSTANCE` and `SERVER,PORT`. Step-by-step: [windows-setup.md](windows-setup.md). |
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

### PostgreSQL TLS

*SSL mode* matches libpq:

| Mode | What it does |
|---|---|
| `disable` | No TLS. |
| `prefer` | TLS if the server allows it, otherwise plain. The server is not authenticated. |
| `require` | TLS is required. The server is not authenticated. |
| `verify-ca` | TLS, and the server certificate must chain to a trusted CA. The host name is not checked. |
| `verify-full` | TLS, a trusted CA, and the certificate name must match the host. |

*CA certificate* is a PEM file. With `verify-ca` or `verify-full` it replaces
the operating system's trust store. Leave it empty to use that store.

*Client certificate* and *Client key* are for servers that require a client
certificate. The certificate is PEM, leaf first. The key is an unencrypted
PEM private key (PKCS#8, PKCS#1 or SEC1). The service encrypts the key and
never sends it back; the form shows only that one is stored. Decrypt an
encrypted key first (`openssl pkey -in key.pem -out key.pem`).

The files are stored with the connection profile on the service. They are
not paths on your machine.

### Azure SQL Database

Choose **SQL Server**, enter the server name from the Azure portal
(`myserver.database.windows.net`, port 1433) and the database, and keep
*Encryption* on `Required`. sqail follows Azure's gateway redirect by itself.
*Authentication* is one of:

| Method | Fields | Use it for |
|---|---|---|
| SQL login | User, Password | A SQL login created in the database or on the server. |
| Microsoft Entra password | User (`ann@contoso.com`), Password, Tenant (optional) | Your own Entra account. It doesn't work for accounts that require MFA; Microsoft is also retiring this sign-in flow. |
| Microsoft Entra service principal | Tenant, Client ID, Client secret | An app registration. Best for a shared service. |
| Microsoft Entra managed identity | Client ID (only for a user-assigned identity) | sqail-service running on Azure (VM, App Service, Container Apps, AKS with node identity). No secret is stored. |

Entra sign-in happens on the **service** host, which needs HTTPS access to
`login.microsoftonline.com` (or the Azure metadata endpoint for managed
identity). The account, app or identity must exist as a user in the
database, e.g. `CREATE USER [my-app] FROM EXTERNAL PROVIDER;` run by the
server's Entra admin. The service caches tokens and fetches a new one before
it expires.

#### Discovering Azure databases

Right-click a SQL Server connection that signs in with Microsoft Entra ID and
choose **Discover Azure databases…**. The service uses that connection's
identity to ask Azure Resource Manager for the Azure SQL databases, SQL
Managed Instance databases and Azure Database for PostgreSQL (flexible
server) databases in every subscription the identity can read. Tick the ones
you want, pick a folder (*Azure* by default) and click **Add**. Databases that
already have a connection are greyed out.

* New SQL Server connections sign in the same way as the original, with its
  stored password or client secret.
* New PostgreSQL connections get the server's admin login and SSL mode
  `require`; edit them to enter the password.
* Colour, environment tag and read-only are copied from the original.

The identity needs the **Reader** role (or another role that can list these
resources) on the subscriptions, and the service host needs HTTPS access to
`management.azure.com`. Entra password sign-in uses the Azure CLI's public
client for this unless the profile names its own. Subscriptions or servers
that could not be listed show up as warnings in the dialog. Discovery needs
an admin token.

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
| Ctrl+mouse wheel, or the wheel with the middle button held | Zoom the editor text |
| Ctrl+Shift+B | Expand / collapse the [sidebar](#sidebar) |
| Ctrl+Shift+A | Expand / collapse the [AI assistant](#ai-assistant) |
| Ctrl+, | [Settings](#settings) |

The script splitter understands comments, string and identifier quoting,
Postgres dollar quotes and SQL Server `GO` lines, so *Run statement* picks
exactly the statement you're in. Completion knows keywords, schemas, tables,
columns and functions of the current connection. After `alias.` it lists
that alias's columns: tables (`FROM orders o`), CTEs (`WITH x AS (…)`) and
subqueries (`FROM (SELECT …) s`), with the innermost subquery winning when
an alias is reused. A column name shared by several tables in the query is
offered qualified (`o.id`, `c.id`).

**Transactions:** every tab has its own server session. `BEGIN … COMMIT`
spans runs, and the status bar shows **in transaction** while one is open.
Switch off **Auto-commit** in the toolbar and sqail sends `BEGIN` before the
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
* A result wider than the pane scrolls sideways (scrollbar at the bottom, or
  Shift+mouse wheel).
* Click a cell to select it; Shift+click extends that to a rectangle. Click a
  row number to select the row, a column name to select the column, and the
  **#** corner (or Ctrl+A) to select the whole table. Shift+click on a row
  number or a column name extends that selection. Ctrl+C copies the selection
  as TSV, which pastes straight into a spreadsheet; Ctrl+Shift+C includes the
  column names. Right-click for *Copy*, *Copy with headers*, *Copy row*,
  *Copy column* or *Copy table*. A copy stops after 50 000 rows and says so;
  export the result for the rest.
* Click the **▲▼** on a column header to sort (ascending, then descending,
  then back to the original order). Sorting waits until the result has
  finished loading.
* Double-click a cell to see its full value (long text, JSON, binary as hex).
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

**«** at the top right collapses the sidebar to a thin strip at the left
edge; click the strip, or press **Ctrl+Shift+B**, to bring it back.
Dragging its edge all the way in or out does the same. The AI assistant
panel on the right collapses the same way (**»**, Ctrl+Shift+A), and both
remember their state.

* **Connections:** a tree of schemas, tables, views, columns, indexes, foreign
  keys and routines, loaded lazily. Right-click a table for *SELECT top 100*,
  *Design table…*, *Import CSV…*, *Script CREATE*, *Drop table…* or *Copy
  name*; right-click *Tables* or a connection for *New table…*. Double-click a
  connection to edit it. Drag a table, view or routine
  into the editor: inside a statement it inserts the qualified name; on a
  blank line (or below the text) a table or view becomes a formatted
  `SELECT` of all its columns. A procedure becomes `EXEC name` on SQL
  Server or `CALL name()` on Postgres, and a function becomes
  `SELECT name();`.
* **History:** every query you ran, with its connection, duration and row
  count. Filter it, open an entry in a new tab, or clear it. The last 5 000
  entries are kept.
* **Snippets:** save a selection with *Save as snippet…*, then insert it
  from the sidebar or with Ctrl+P.

Open tabs, including unsaved text, are restored when you start sqail again.

## Table designer

*Design table…* (right-click a table) opens the table in a window; *New
table…* (right-click *Tables* or a connection, the *Connections* menu or the
command palette) starts an empty one with an `id` key. Several designers can
be open at once. Pages:

* **Columns:** name, type (type it, or pick a common one from ⏷), whether it
  allows NULL, and the default as an SQL expression (`0`, `'none'`,
  `now()`). Tick *Key* to put a column in the primary key. Rows you added
  are marked *new*, edited ones *changed*; removed columns are listed under
  *Dropped* and can be restored until you apply. Column order can be changed
  only for a new table.
* **Indexes:** name, *Unique*, and the key columns in order (click the
  column list to add, remove or reorder). An index that backs a `UNIQUE`
  constraint is marked *constraint* and stays one.
* **Primary key:** the key columns in order and, on PostgreSQL and SQL
  Server, the constraint name. Key columns are always NOT NULL.
* **Security:** who holds which privilege on the table (`SELECT`, `INSERT`,
  `UPDATE`, `DELETE`, …). Tick and untick to grant and revoke; *+ Add
  grantee* adds a role or user (⏷ lists the ones the database has). Not
  available for SQLite, which has no permissions.
* **SQL:** the statements your edits add up to; *Open in editor* copies them
  to a new tab.

**Apply…** shows the statements again and runs them in one transaction: if
one fails, nothing changes and the error is shown. Afterwards the designer
reloads the table from the database. *Revert* throws the edits away, ⟳
reloads, and *Drop table…* asks before it drops the table (on PostgreSQL
optionally with `CASCADE`). A read-only connection can be browsed but not
changed.

Good to know:

* Renames are real renames (`RENAME COLUMN`, `sp_rename`), so data stays.
* Changing an index drops and re-creates it as a plain index: PostgreSQL
  index methods, `INCLUDE` columns and `WHERE` clauses are not kept.
* SQL Server: changing a column's type or NULL-ness re-creates the indexes
  and key on it, and a default is dropped and re-added by its generated
  name.
* SQLite can change only a few things in place (renames, adding and dropping
  columns, indexes). Anything else (a type, NULL-ness, a default, the
  primary key) rebuilds the table: the rows are copied into a new table
  that replaces the old one. Triggers on the table are dropped, and `CHECK`
  constraints, `AUTOINCREMENT` and foreign-key actions are not carried
  over; the review says so before you apply.

## AI assistant

The assistant answers questions about your data and writes queries, using
**Claude Code** or **Grok**, the command-line tools you already have. It runs
the CLI in the background with sqail's own tools, so it can look at the
schema and the data of the connection you're working on instead of guessing.

**Set up:** install [Claude Code](https://claude.com/claude-code) (`claude`)
or the Grok CLI (`grok`) and sign in once in a terminal. sqail finds them on
`PATH`. Nothing else is needed; sqail does not hold an API key.

**Use:** press **Ctrl+Shift+A** (or *View → Expand / collapse the AI assistant*),
pick Claude Code or Grok, and ask, for example *"Which customers ordered the
most last quarter?"* or *"Why is the query in my editor slow?"*. The answer
streams in. Each query it proposes gets **Insert** (at the cursor in the
editor), **New tab** and **Copy** buttons. Follow-up questions continue the
same conversation; **New chat** starts over. Enter sends, Shift+Enter adds a
line. *Include editor SQL* sends the active tab's SQL along with your question.

A conversation stays on the connection of the tab that was active when it
started (shown under the title). The assistant can:

| Tool | What it does |
|---|---|
| `list_schemas`, `list_tables` | See what's in the database |
| `describe_table` | Columns, types, keys, indexes and foreign keys |
| `run_query` | Run **one read-only statement** and see up to 100 rows |

Each tool call shows up in the conversation; click it to see the SQL and the
result.

**What it can't do:** change anything. `run_query` accepts only a single
`SELECT` / `WITH` / `VALUES` statement: writes, DDL, `EXEC`, `SELECT … INTO`,
`FOR UPDATE`, multiple statements and functions with side effects are
refused. Every assistant query also runs in its own transaction that is always
rolled back (read-only on PostgreSQL), so even something that slipped through
would be undone. The CLI gets no other tools: no shell, no files, no web.
When you ask for changes, it writes the SQL for you to review and run
yourself.

**What leaves your machine:** your questions, the schema details and the
query results (up to 100 rows per query) the assistant looks at go to the
provider you picked, under that provider's terms. Use it on data you're
allowed to share with them.

**Settings** (`[assistant]` in `settings.toml`):

| Key | Default | Meaning |
|---|---|---|
| `provider` | `"claude_code"` | or `"grok"` |
| `claude_path`, `grok_path` | found on `PATH` | Full path to the CLI |
| `claude_model`, `grok_model` | the CLI's default | e.g. `"sonnet"` |
| `max_rows` | `100` | Most rows one assistant query returns to the model |

On Windows, sqail uses the native `grok.exe` behind npm's `grok.cmd`. If it
can't find it, set `grok_path`.

## Settings

*File → Settings…* (Ctrl+,) opens every application-wide setting in one
window. Changes apply right away and are saved to `settings.toml`.

| Section | Settings |
|---|---|
| Appearance | **Theme**, **interface size** (75–200 %), **editor font size** |
| Editor | Complete while typing; formatter keyword case and indent (2 or 4 spaces) |
| Tabs | **Ask before closing a tab with unsaved changes** (on by default). Off, such a tab closes at once and its changes are lost. A tab with an open transaction always asks, because closing it rolls the transaction back. |
| Queries | Row limit per result set |
| Service | Start the local service when sqail starts |
| AI assistant | Provider, model and program path per provider, rows shown to the model, sending the active tab's SQL |

Themes: *Follow system* (sqail's light or dark, whichever the OS uses),
*sqail light*, *sqail dark*, **Omarchy**, Nord, Tokyo Night, Gruvbox,
Catppuccin Mocha, Catppuccin Latte and Solarized Light. *View → Theme* has
the same list.

**Omarchy** takes its colours from the current Omarchy theme
(`~/.local/state/omarchy/current/theme/colors.toml`), editor syntax colours
included, and follows along within a couple of seconds when you switch
themes with `omarchy theme set` or the theme menu. On a first start on
Omarchy it is the default. Elsewhere, choosing it falls back to *Follow
system* and says why.

## Files and settings

sqail keeps its settings in `~/.config/sqail/` on Linux and
`%APPDATA%\bartbeecoders\sqail\config\` on Windows
(`SQAIL_CONFIG_DIR` overrides it). Version 1.0 was called sqail2; on first
start sqail moves its `sqail2` settings folder and saved tokens over. The files:

| File | Contents |
|---|---|
| `settings.toml` | Services and everything in [Settings](#settings) |
| `keybindings.toml` | Your shortcut overrides |
| `history.jsonl`, `snippets.json`, `workspace.json` | History, snippets, open tabs |
| `tokens.toml` | Only when the OS credential store is unavailable (mode 0600) |

Example `keybindings.toml`. A list binds several shortcuts to one command,
and an empty list `[]` removes a default shortcut. Problems (an unknown command
or a shortcut that can't be parsed) are reported once when sqail starts.

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
`view.history`, `view.snippets`, `view.sidebar`, `view.assistant`,
`connection.new`, `connection.refresh`, `table.new`, `service.connect`,
`app.settings`.

Client certificate for a service that requires mTLS, in `settings.toml`:

```toml
[[services]]
name = "team gateway"
url = "https://sql-gw.example.com:7443"
client_cert = "/home/me/.config/sqail/me.crt.pem"
client_key = "/home/me/.config/sqail/me.key.pem"
```

## Troubleshooting

* **"certificate fingerprint mismatch" on connect:** the service now presents
  a different certificate than the one you pinned. This is expected after the
  administrator replaced it. Otherwise treat it as an attack. After checking
  the new fingerprint, choose *Service → Forget this service*, then connect
  again.
* **Logs:** start sqail from a terminal with `SQAIL_UI_LOG=debug`.
