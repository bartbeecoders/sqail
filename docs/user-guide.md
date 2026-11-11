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
can also build the package with the `PKGBUILD` attached to each GitHub release
(it pins the source tarball's checksum): put it in an empty folder and run
`makepkg -si`.

**Windows:** run the MSI, or unzip the portable
`sqail-<version>-windows-x64.zip` anywhere and start `sqail.exe`.

**Nightly builds:** every night that `main` changed, a build of it is
published as the [`nightly` prerelease](https://github.com/bartbeecoders/sqail/releases/tag/nightly)
(`sqail-<version>-nightly.<date>-…`, Linux tarball and Windows zip, no MSI).
It has passed the full test suite but is otherwise untried; use a tagged
release for anything that matters. Each nightly replaces the previous one.

**Verify the download:** each release has a `SHA256SUMS` file and a signed
build provenance attestation for every package. In the download folder,
`sha256sum -c --ignore-missing SHA256SUMS` checks the hashes, and
`gh attestation verify <file> --repo bartbeecoders/sqail` (GitHub CLI) checks
that the file was built by sqail's release workflow. See
[Verifying a download](security.md#verifying-a-download).

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

The status bar shows whether the service is reachable, as of the last
request: when a request can't reach it, the dot turns red and a
**Reconnect** button appears; it turns green again as soon as a request gets
through. When the service no longer accepts the token (revoked or expired),
sqail asks for a new one.

## Connections

**+ Connection** under *Connections* in the sidebar (or *Connections → New
connection…*) opens the connection form. Profiles are stored **on the service**,
so everyone using that service with a suitable token sees the same list.
Passwords are write-only: the service encrypts them and never sends them back.
For PostgreSQL and SQL Server, fill in the server and login first, then click
**⏷** next to *Database* to choose from the databases that login can access
(you can still type a name). The admin page's *List* button does the same.
Listing needs an admin token, as *Test* does for an unsaved connection.

Each tab runs on the connection chosen in its toolbar. A tab gets one by
itself only when the service has exactly one connection; with several,
choose one first (new tabs take the current tab's connection).

A click beside the connection form doesn't close it; *Cancel* or Esc on a
form you changed asks before discarding it.

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
| SQLite | The path is a file on the **service's** machine and must be inside one of the service's `sqlite.allowed_dirs` (and not in the service's own data folder). `ATTACH` and `VACUUM INTO` follow the same rule, and read-only profiles can't use them on files at all. |

Give production profiles a colour and an environment tag. Both show in the
status bar while that connection is active. **Read-only** profiles also can't
be edited in the grid. How strictly "read-only" is enforced depends on the
engine:

* **PostgreSQL:** the session runs with `default_transaction_read_only = on`,
  and the service refuses statements that would switch it off: setting
  `transaction_read_only` or `default_transaction_read_only`,
  `BEGIN … READ WRITE`, `SET SESSION CHARACTERISTICS`, `RESET ALL`,
  `set_config()` and `DO` blocks. A function the role may call can still
  write, so for a hard guarantee use a role that only has read permissions.
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
| Esc | Cancel the running query (Esc in the find bar, the completion list or a grid cell closes that first) |
| Ctrl+E | Show the plan of the statement under the cursor |
| Ctrl+Z / Ctrl+Y (or Ctrl+Shift+Z) | Undo / redo. Typing undoes a word or a line at a time (and at each pause, paste or cursor jump); a format, completion, replace, drop, insert, indent or comment toggle is one step of its own, so undoing it keeps what you typed before it |
| Ctrl+Space | Completion (also opens by itself while you type a name it can finish, but not when the word is already a whole name or keyword, nor in comments and strings; a space, comma or other punctuation closes it, so Enter and Tab go to the text) |
| Ctrl+Shift+F | Format the selection or the whole tab. It only changes layout and keyword case: when the formatter would change more (some string literals, `0x…` numbers, Postgres `E'…'` or dollar-quoted strings), the text is left alone with a notice |
| Tab / Shift+Tab | Indent / outdent the selected lines (without a selection, Tab types a tab and Shift+Tab outdents the line) |
| Ctrl+/ | Comment or uncomment the selected lines with `--` |
| Ctrl+Shift+P | Command palette: every action, searchable |
| Ctrl+P | Quick open: jump to a table or snippet |
| Ctrl+T / Ctrl+W | New / close tab |
| Ctrl+PageDown / Ctrl+PageUp | Next / previous tab |
| Ctrl+O / Ctrl+S / Ctrl+Shift+S | Open / save / save as (`.sql` files) |
| Ctrl+F / Ctrl+H | Find and replace |
| F3 / Shift+F3 | Find next / previous (Enter / Shift+Enter in the find field; Esc there closes it) |
| Ctrl+= / Ctrl+- | Larger / smaller editor font |
| Ctrl+mouse wheel, or the wheel with the middle button held | Zoom the editor text |
| Ctrl+Shift+B | Expand / collapse the [sidebar](#sidebar) |
| Ctrl+Shift+A | Expand / collapse the [AI assistant](#ai-assistant) |
| Ctrl+, | [Settings](#settings) |

These shortcuts act on the editor, so they only work when the SQL editor
has the keyboard focus, or nothing you can type in has it: while you type
in the find bar, a filter, a grid cell, a table designer or the assistant,
Ctrl+W, Ctrl+Enter, Ctrl+S and the like do nothing. The command palette,
Settings and the sidebar and assistant toggles work everywhere. A shortcut
needs exactly its modifiers (only Shift may be added, for keys like `+`
that need it), so AltGr combinations type their character.

**Pinned tabs.** Right-click a tab for *Pin tab* (or *File → Pin / unpin
tab*). Pinned tabs get a row of their own, labelled *Pinned*, above the
other tabs; they are tinted, marked ⚑ and have no close button, and middle
clicks don't close them. Ctrl+W or *Close tab* in their menu still closes
them. Untick *Pinned tabs on a separate row* (in the same menu, or Settings →
Tabs) to keep them at the front of the one tab row instead. Pins are
remembered with the open tabs.

**File encodings.** sqail opens UTF-8 files with or without a byte-order
mark, UTF-16 (LE or BE, with a byte-order mark, as older SSMS versions
save), and treats any other file that isn't valid UTF-8 as Windows-1252
("ANSI"). Windows line endings (CRLF) become plain line breaks in the
editor. Saving writes the file back with its own encoding, byte-order mark
and line endings (CRLF or LF, whichever the file mostly used), so saving an
untouched file leaves it unchanged. New files are UTF-8 without a
byte-order mark, with LF line endings on Linux and CRLF on Windows. If you
type a character a Windows-1252 file can't hold, sqail saves that file as
UTF-8 and says so.

The script splitter understands comments, string and identifier quoting,
Postgres dollar quotes and SQL Server `GO` lines, so *Run statement* picks
exactly the statement you're in. On SQL Server a `CREATE`/`ALTER`
procedure, function, trigger or view runs to the next `GO` (or the end of
the tab), with or without `BEGIN … END`, so semicolons and blank lines in
its body don't cut it short. A SQL Server batch from a `DECLARE` on also
runs to the next `GO`, since its variables only exist in that batch, and
`IF … ; ELSE …` stays one statement. A `GO` line is `GO`, optionally with a
repeat count of 1 or more (`GO 0` is not a separator, for the editor nor
for the service). An unclosed `(` or `CASE` ends at a blank line followed by
a new statement (`SELECT`, `DELETE`, …), so it doesn't pull the rest of the
script into the statement under the cursor. Completion knows keywords, schemas, tables,
columns and functions of the current connection. After `alias.` it lists
that alias's columns: tables (`FROM orders o`), CTEs (`WITH x AS (…)`) and
subqueries (`FROM (SELECT …) s`), with the innermost subquery winning when
an alias is reused. A column name shared by several tables in the query is
offered qualified (`o.id`, `c.id`). Where only a table fits it offers
tables, schemas and CTEs instead of columns: after `FROM`, `JOIN`,
`CROSS`/`OUTER APPLY`, `UPDATE`, `INTO`, `MERGE … USING`, a comma in the
`FROM` list (also after an alias or a table hint such as
`WITH (NOLOCK)`), and on SQL Server after `INSERT`, `DELETE` and `MERGE`.

**Transactions:** every tab has its own server session. `BEGIN … COMMIT`
spans runs, and the status bar shows **in transaction** while one is open.
Switch off **Auto-commit** in the toolbar and sqail sends `BEGIN` before the
next run whenever no transaction is open. Finish with **Commit** or
**Rollback** in the toolbar.
Closing a tab or switching its connection with an open transaction asks
first; going ahead closes the tab's session, which rolls the transaction
back. Quitting closes every tab's session the same way (it asks first when a
transaction is open), so nothing is left holding locks on the server.
Idle sessions close after 30 minutes on the service, which rolls them back.
If a tab's session is gone (it expired, the service restarted, or its database
connection dropped), the next run opens a new session. When a transaction was
open, sqail doesn't run the statement: it reports that the session was lost
and the transaction rolled back, and clears **Transaction open**. Run the
statement again to start over in a new transaction.
**Esc** stops only the running statement: the session keeps its open
transaction, temp tables and `SET` options, as in SSMS. (On SQL Server with
`SET XACT_ABORT ON`, the server rolls the transaction back.)

A script stops at the first statement that fails. The statements before it
keep their effect, as in psql and SSMS: each statement commits on its own
unless the script (or Auto-commit off) opened a transaction, which the error
then leaves open in the tab (or, outside a session, rolls back). On
PostgreSQL the service splits the script like psql does; only when that
depends on how the server reads backslashes in strings (`'a\'`) does the
script run as one transaction, and the error then says that the statements
before it were rolled back too.

`COPY … TO STDOUT` (text or CSV) on PostgreSQL shows its output as a result
with one `copy` column, a row per line. `COPY … FROM STDIN`, as in pg_dump
output, can't be run: sqail refuses the whole script before running any of
it, so the tab's transaction and temp tables stay as they were. Use
*Import CSV*, `INSERT` statements, or `COPY … FROM` a file on the database
server instead.

**Plans:** *Explain* (Ctrl+E) shows the estimated plan as a tree, with cost
and row estimates and the most expensive nodes highlighted. *Explain analyze*
actually runs the statement to get real timings and row counts, and its
effects are **always rolled back**, so it is safe for `UPDATE` and `DELETE` as
well. When the tab has a transaction open, only the explained statement is
undone (to a savepoint): the transaction and your earlier changes in it stay
open for **Commit** or **Rollback**. A PostgreSQL transaction that already
failed must be rolled back first. On SQLite, *Explain analyze* shows the
estimated plan and runs nothing. The *Raw* toggle shows the engine's own JSON
or XML plan.

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
  finished loading. NULLs sort first. In a text column, values that are
  numbers (`9`, `10`, `2.5`) sort numerically and come before the other
  text (`5a`, `abc`), which sorts alphabetically.
* Column widths start from the column name and the first 1 000 rows, up to
  about 80 characters. Drag a column's right border to resize it, or
  double-click the border to fit the column to its values (the first
  20 000 rows). Right-click a column name for *Auto-fit column* or
  *Auto-fit all columns*.
* A value that is cut off (too wide, or more than one line) shows in full
  when you hover over it, up to 2 000 characters and 40 lines.
* **Multi-line rows** (right-click a cell or a column name, or Settings →
  Queries; off by default): values wrap to the column width and line breaks
  in them start a new line. Each row grows to fit, up to 10 lines; a value
  that needs more ends in `…`, and hovering or *View value* shows the rest.
  Rows are measured as they come into view, so the scrollbar can adjust
  slightly while you scroll through a large result.
* Double-click a cell to see its full value (long text, JSON, binary as hex).
  A row may take at most 64 MiB in a result: longer values are cut (they end
  in `…`) and a warning says so. Select a part of them (`substring`) to see
  the rest.
* Values show as the database has them. Floats print like psql: very large
  or small ones with an exponent (`1.1805916207174113e+21`, `1e-05`), and
  `NaN`, `Infinity`, `-Infinity`. SQL Server `money` shows its four decimals
  (`1.1000`), `datetime` its three (`13:45:12.997`), and `time`, `datetime2`
  and `datetimeoffset` the column's own number of fraction digits, so
  exported values and grid edits go back to the server unchanged.
* **Export** saves the rows in the grid, or re-runs the query and streams
  every row straight to a file, as CSV, JSON, Excel (.xlsx) or SQL `INSERT`s.
  The re-run ignores your row limit and is only bounded by the
  service's `limits.max_rows` (5 000 000 by default). It runs only the
  statement that produced the result you're looking at, in the tab's session
  (so temp tables and uncommitted rows are there), and only if that
  statement just reads: when it writes, or sqail can't tell which statement
  produced the result, it refuses and asks you to run the `SELECT` on its own
  first. A session runs one query at a time, so runs in that tab wait until
  the export finishes.
* **✎ Edit data** is available when a result comes from a single table with a
  primary key. Edit cells (double-click or F2), add rows and delete rows. The
  changes are shown as SQL before **Apply** runs them, all or none, in the
  tab's session: the same table the result came from, even after
  `SET search_path`, `USE` or with a temp table of the same name, and with
  the tab's uncommitted rows visible. With Auto-commit and no transaction
  open, they are committed at once. Otherwise (Auto-commit off, or a
  transaction open) they become part of the tab's transaction and are not
  committed until you click **Commit**. Only `*` and plain column names are
  editable: aliased columns
  (`a AS b`), literals and expressions stay read-only, and the primary key
  must be selected once, without an alias. Binary columns (`bytea`,
  `varbinary`, `BLOB`) are edited as `0x` followed by hex digits, as they are
  shown (`0x` alone is an empty value); sqail writes the bytes themselves,
  and refuses a value that isn't valid hex before anything is sent.
* Messages (`RAISERROR`, `RAISE NOTICE`, `PRINT`) and row counts appear in the
  *Messages* tab, in the order the server sent them, between the results.
  On SQL Server every statement reports its row count (unless
  `SET NOCOUNT ON`), and, as in SSMS, an error does not stop a script with
  `GO` batches: the next batch still runs. Error line numbers count from the
  start of the script (inside a procedure, from the start of the procedure,
  which the message names).

## Sidebar

**«** at the top right collapses the sidebar to a thin strip at the left
edge; click the strip, or press **Ctrl+Shift+B**, to bring it back.
Dragging its edge all the way in or out does the same. The AI assistant
panel on the right collapses the same way (**»**, Ctrl+Shift+A), and both
remember their state.

* **Connections:** a tree of schemas, tables, views, columns, indexes, foreign
  keys and routines, loaded lazily. Right-click a table for *SELECT top 100*,
  *Design table…*, *Import CSV…*, *Script CREATE*, *Drop table…* or *Copy
  name*. *Import CSV* reads binary columns as `0x` + hex, the way CSV
  export writes them, and stops (changing nothing) at a value that isn't
  valid hex. Right-click a routine for *Script definition* or *Copy name*.
  *Script CREATE* on a table writes columns with their identity, computed
  or generated definition, defaults, collations, keys, unique, check and
  foreign-key constraints (with their `ON DELETE`/`ON UPDATE` actions) and
  indexes with sort order, `INCLUDE` columns and filters; on PostgreSQL also
  partitioning and comments, with serial columns written as `serial`. SQL
  Server constraints whose names it generated are scripted without a name,
  so a copy gets fresh ones. When
  the object can't be scripted, the new tab says why instead; on SQL Server
  that is usually a missing `VIEW DEFINITION` grant for the service's login,
  and the tab shows the `GRANT` to run (see `docs/windows-setup.md`).
  Right-click *Tables* or a connection for *New table…*. Double-click a
  connection to edit it. Drag a table, view or routine
  into the editor: inside a statement it inserts the qualified name; on a
  blank line (or below the text) a table or view becomes a formatted
  `SELECT` of all its columns. A procedure becomes `EXEC name` on SQL
  Server or `CALL name()` on Postgres, and a function becomes
  `SELECT name();`. Names that need it are quoted, here and wherever sqail
  writes SQL for you (completion, the table designer): mixed case, special
  characters, and every word the engine reserves, such as `"user"` or
  `"position"` on PostgreSQL.
* **History:** every query you ran, with its connection, duration and row
  count. Filter it, open an entry in a new tab, or clear it (*Clear* asks
  first). The last 5 000 entries are kept.
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
  constraint is marked *constraint* and stays one. Descending keys show
  `DESC`; `INCLUDE` columns, a `WHERE` filter and *clustered* are shown next
  to the index but can't be edited here.
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
* Changing an index drops and re-creates it. Its sort order (`DESC`),
  `INCLUDE` columns, `WHERE` filter and, on SQL Server, `CLUSTERED` or
  `NONCLUSTERED` are kept (the Indexes page shows them); a PostgreSQL index
  method other than B-tree is not.
* Dropping a column takes it out of the indexes that include it (they are
  re-created without it); an index whose filter uses the column is dropped.
* SQL Server: changing a column's type or NULL-ness re-creates the indexes
  that use it (as a key, included or in the filter) and the key on it, and a
  default is dropped and re-added by its generated name. *Open in editor*
  gives a script that runs as one batch.
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
`FOR UPDATE`, transaction control (`COMMIT`, `ROLLBACK`, …), session and
admin commands, multiple statements, unfinished strings or comments and
functions with side effects (also when the name is quoted, like
`"pg_sleep"(1)`, or written with `U&"…"` escapes) are refused. A quoted
column or table that happens to share such a function's name is still fine. The service also refuses anything
but a single statement for the assistant. Every assistant query also runs in
its own transaction that is always rolled back (read-only on PostgreSQL), so
even something that slipped through would be undone. For the strongest
guarantee, point the assistant at a connection whose login can only read
(see [security](security.md)). The CLI gets no other tools: no shell, no files, no web.
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
| Tabs | Pinned tabs on a separate row. **Ask before closing a tab with unsaved changes** (on by default). Off, such a tab closes at once and its changes are lost. A tab with an open transaction always asks, because closing it rolls the transaction back. |
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

sqail writes these files atomically (a temporary file, flushed to disk, then
renamed over the old one), so a crash or power cut never leaves half a file.
If `settings.toml`, `workspace.json` or `snippets.json` can't be read
anyway (a hand-edit typo, say), sqail renames it to `<name>.bad`
(`<name>.1.bad` and so on if that exists), starts without it and says so in
the status bar; a bad `settings.toml` also shows the error in the welcome
dialog. (A bad `tokens.toml` is set aside the same way and logged.) Fix the
`.bad` file and rename it back while sqail is closed to get your services,
tabs or snippets back. Out-of-range values in `settings.toml` are brought
into the range the Settings window offers: interface size 75–200 %, font
size 9–28, indent 2–4, assistant rows 1–10,000, row limit at least 1 (its
upper bound is the service's).

Only one sqail runs per settings folder: starting a second one shows
"sqail is already running" and closes, because two windows would overwrite
each other's tabs and settings. To run two side by side, give the second
its own folder with `SQAIL_CONFIG_DIR`.

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
`file.save_as`, `edit.find`, `edit.find_next`, `edit.find_previous`,
`edit.format`, `edit.save_snippet`, `view.command_palette`,
`view.quick_open`, `view.font_bigger`,
`view.font_smaller`, `view.toggle_theme`, `view.connections`,
`view.history`, `view.snippets`, `view.sidebar`, `view.assistant`,
`connection.new`, `connection.refresh`, `table.new`, `service.connect`,
`app.settings`, `tab.pin`.

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
* **A connection says "not supported by this sqail version":** the service
  is newer than your sqail and the connection uses an engine your version
  doesn't know. It can't be opened, edited or picked for a tab (only deleted)
  until you update sqail; the rest of the list works as usual.
* **Logs:** start sqail from a terminal with `SQAIL_UI_LOG=debug`.
