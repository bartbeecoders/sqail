# Setting up sqail2 on Windows

sqail2 has two parts:

* **sqail2.exe** is the editor that people use.
* **sqail-service.exe** is an HTTPS gateway. It holds the database
  connections and credentials, and every query goes through it. The editor
  never talks to SQL Server directly.

```
 users' PCs                    gateway host                     database servers
┌──────────┐   HTTPS :7443   ┌──────────────────┐   TDS :1433   ┌──────────────┐
│ sqail2   │ ───────────────▶│ sqail-service    │──────────────▶│ SQL Server   │
│ (editor) │  token + pinned │ (Windows service)│  login + TLS  │ (any number) │
└──────────┘   certificate   └──────────────────┘               └──────────────┘
```

Pick the setup that fits:

| | **A. Just me** | **B. Shared gateway** |
|---|---|---|
| Who | One person on their own PC | A team, or anyone who shouldn't hold the DB passwords |
| Service runs | Next to the editor, as you, started by sqail2 | As a Windows service on a server |
| SQL Server credentials | Stored on your PC (encrypted) | Stored on the gateway only; users never see them |
| Windows authentication | As **you** | As the **service account** (see [B5](#b5-windows-authentication-optional)) |
| Setup time | 1 minute | 15–30 minutes |

---

## A. Just me

1. Unzip `sqail2-<version>-windows-x64.zip` (or run the MSI) and start
   **sqail2.exe**.
2. Choose **Use the local service**. sqail2 starts `sqail-service.exe` from its
   own folder, creates a token and stores it in Windows Credential Manager.
3. Click **+ Add** next to *Connections* in the sidebar (or *Connections →
   New connection…*) and choose **SQL Server**:

   | Field | What to enter |
   |---|---|
   | Host | `sqlserver01`, `sqlserver01.corp.local` or an IP. `SERVER\INSTANCE` and `SERVER,PORT` work as in SSMS. |
   | Port | `1433`, unless the server uses another port |
   | Instance | For a named instance, e.g. `SQLEXPRESS`. Leave empty for the default instance. |
   | Database | e.g. `Sales` (empty = the login's default database) |
   | Authentication | *SQL login* (user + password), or *Windows (integrated)* (your own Windows account) |
   | Encrypt | `Required` (default). See [SQL Server certificates](#sql-server-certificates) |
   | Trust server certificate | Tick only for test servers with a self-signed certificate |

4. Click **Test**, then **Save**. The connection now appears in the sidebar.

That's it. If you later want colleagues to share the same connections, set
up B on a server.

---

## B. Shared gateway

You need:

* A Windows Server 2016+ or Windows 10/11 machine for the gateway. It must be
  able to reach your SQL Servers on TCP 1433, and the users' PCs must be able
  to reach it on TCP 7443.
* Administrator rights on that machine.
* On each SQL Server, someone who can create logins (a DBA).

### B1. Prepare SQL Server

Do this once per SQL Server instance, in SQL Server Configuration Manager and
SSMS.

**1. Enable TCP/IP.** In *SQL Server Configuration Manager → SQL Server Network
Configuration → Protocols for <instance>*, set **TCP/IP** to *Enabled* and
restart the SQL Server service. Default instances listen on **1433**. Named
instances use a dynamic port, so either start the **SQL Server Browser**
service (UDP 1434), or give the instance a fixed port (*TCP/IP → IP
Addresses → IPAll → TCP Port*) and use `SERVER,PORT`.

**2. Open the firewall** on the SQL Server host for the gateway: TCP 1433 (or
the instance's port), plus UDP 1434 if you rely on SQL Browser.

**3. Create a login** for the gateway with only the rights users need. With
SQL authentication (the server must be in *SQL Server and Windows
Authentication mode*):

```sql
-- On the SQL Server, as a sysadmin.
CREATE LOGIN sqail_reader WITH PASSWORD = 'use-a-long-random-password', CHECK_POLICY = ON;

USE Sales;
CREATE USER sqail_reader FOR LOGIN sqail_reader;
ALTER ROLE db_datareader ADD MEMBER sqail_reader;   -- read all tables
GRANT SHOWPLAN TO sqail_reader;                      -- query plans (Explain)
GRANT VIEW DEFINITION TO sqail_reader;               -- schema tree, "Script CREATE"
-- For a read/write profile instead:
-- ALTER ROLE db_datawriter ADD MEMBER sqail_reader;
-- GRANT EXECUTE TO sqail_reader;                    -- stored procedures
```

Optional: `GRANT ALTER ANY CONNECTION TO sqail_reader;` (on `master`) lets
**Cancel** stop a running query with `KILL` straight away. Without it,
sqail2 still stops the query by closing its connection.

> **Read-only profiles:** a profile marked *read-only* tells SQL Server
> `ApplicationIntent=ReadOnly`, which only routes to readable secondaries.
> It doesn't prevent writes. On SQL Server, a login without write rights is
> what really makes a profile read-only.

### B2. Install the gateway service

1. Copy `sqail2-<version>-windows-x64.zip` to the gateway host. **Before
   unzipping**, right-click the zip → *Properties* → tick **Unblock**, so that
   Windows doesn't block the setup scripts. Then unzip it anywhere, e.g. to
   `C:\Temp\sqail2`. You can also run the MSI instead, which installs to
   `C:\Program Files\sqail2`.
2. Right-click **`setup\Install-SqailService.cmd`** → **Run as administrator**.
   There is nothing to choose. The script:
   * copies the programs to `C:\Program Files\sqail2`;
   * registers the **sqail-service** Windows service. It runs as *Local
     Service*, starts automatically, restarts itself after a crash, and its
     data folder is locked down to SYSTEM, Administrators and the service;
   * adds a Windows Firewall rule for `sqail-service.exe` (*Domain* and
     *Private* networks, local subnet; widen it with
     `-FirewallRemoteAddress 10.0.0.0/8`, or skip it with `-NoFirewall`).
     It has no effect until you let other computers in (step 3);
   * checks that the service answers and **opens its admin page**, already
     signed in.
3. **Finish on the admin page** (`https://127.0.0.1:7443/admin/`). Your
   browser warns once about the self-signed certificate; continue to the
   page. Then, under **Settings**:
   * **Network → Other computers too**, so users' PCs can connect (or pick
     one address of the server, or another port);
   * **Certificate → My own certificate**, if you have one (see below);
   * **SQLite**, if you want SQLite databases: the folders they may be in;
   * **Apply and restart**. The service checks the settings first, restarts
     with them, and keeps the old ones if they don't work.
4. The first admin token is printed in the console once. The admin page
   stays signed in for the browser tab; to sign in again later, keep the
   token in your password manager. If you lose it, run
   `sqail-service --data-dir "%ProgramData%\sqail2\service" admin-link`
   in an elevated prompt: it prints a new sign-in link.

The admin page also shows the **URL and certificate fingerprint** users need
(*Overview*), and manages connections, tokens, backups and the audit log.

**Own certificate (recommended for teams).** With a certificate from your
company CA, sqail2 can verify the gateway through the Windows trust store
instead of pinning, so certificate renewals need no action from users. The
admin page takes PEM files. To convert a `.pfx` (openssl comes with Git for
Windows):

```powershell
openssl pkcs12 -in gw.pfx -clcerts -nokeys -out gw.pem     # certificate; append intermediate CA certs, if any
openssl pkcs12 -in gw.pfx -nocerts -nodes -out gw.key      # private key
```

Choose both files under *Settings → Certificate → My own certificate*, apply,
then delete `gw.key` (a copy now lives in the data folder). The certificate
must name the host users type, e.g. `sqail-gw.corp.local`.

**SQLite folders** need the service account's access:
`icacls D:\sqlite /grant "*S-1-5-19:(OI)(CI)M"` (S-1-5-19 is *Local Service*).

### B3. Register your SQL Server databases

Each database connection is stored once, on the gateway. Every user sees it,
but only the gateway knows the password. Use any of these methods.

**On the admin page** (easiest): *Connections → New connection*, fill in
the form (see the table in [A](#a-just-me)), **Test**, then **Save**. The test
runs from the gateway, so it checks exactly the path users' queries take.

**With the setup script**, for scripted setups, on the gateway, in PowerShell:

```powershell
cd "C:\Program Files\sqail2\setup"
powershell -ExecutionPolicy Bypass -File .\Add-SqailSqlServer.ps1 `
    -Name "Sales (prod)" -Server sqlserver01.corp.local -Database Sales `
    -User sqail_reader -ReadOnly -Environment prod -Color "#c0392b"
```

It asks for the admin token and the SQL password, tests the connection from
the gateway, and saves it only if the test succeeds. On failure it prints a
hint. More examples:

```powershell
# Named instance through SQL Browser
.\Add-SqailSqlServer.ps1 -Name "HR" -Server 'sqlserver02\HR' -Database HR -User sqail_hr
# Fixed port, test server with a self-signed certificate
.\Add-SqailSqlServer.ps1 -Name "Dev" -Server 'devsql,14330' -Database App -User dev -TrustServerCertificate
# Windows authentication as the service account (see B5)
.\Add-SqailSqlServer.ps1 -Name "DW" -Server dwsql -Database DW -WindowsAuth
```

**With sqail2**, on any PC, signed in with the **admin** token: click
**+ Add** next to *Connections*, fill in the form (see the table in [A](#a-just-me)),
then **Test** and **Save**.

### B4. Give users access

For each user (or group of users), create a token: on the admin page under
*Tokens → New token* (it shows the token once, with the URL and fingerprint
to send along), or with the script on the gateway:

```powershell
cd "C:\Program Files\sqail2\setup"
powershell -ExecutionPolicy Bypass -File .\New-SqailToken.ps1 -Name alice             # query scope
powershell -ExecutionPolicy Bypass -File .\New-SqailToken.ps1 -Name reports -Scope read
```

| Scope | Can |
|---|---|
| `read` | browse schemas; run SQL only on **read-only** profiles |
| `query` | run SQL, Explain, and use transactions on every profile (usual choice) |
| `admin` | also add, change or delete connections, manage tokens, read the audit log |

The script prints what to send the user: the URL, the token and the
fingerprint. Send the token through a secure channel. Revoke a token at any
time with `.\New-SqailToken.ps1 -Revoke <id>`, and list tokens with
`-List`.

On the user's PC:

1. Install sqail2 (MSI or zip).
2. Start **sqail2.exe**. On first start it asks how to connect. Later, use
   *Service → Connect to a service…*.
3. Under **Another service**, enter the URL
   (`https://sqail-gw.corp.local:7443`) and the token, then click
   **Connect**.
4. Self-signed gateway: compare the fingerprint shown with the one you
   received, then click **Trust and connect**. Company certificate: click
   **Use system trust**.
5. The registered databases appear in the sidebar.

The token is stored in the user's Windows Credential Manager.

### B5. Windows authentication (optional)

With *Windows* authentication, SQL Server sees the **account the service runs
as**, not the person using sqail2. Every user of that profile shares that
identity. Local Service connects to *other* machines anonymously, so for
Windows authentication to a remote SQL Server, run the service as a domain
account. A group-managed service account (gMSA) is best:

```powershell
# On the gateway (after your AD admin created the gMSA and allowed this host):
Install-ADServiceAccount svc-sqail
sc.exe config sqail-service obj= "CORP\svc-sqail$" password= ""
icacls "C:\ProgramData\sqail2\service" /grant "CORP\svc-sqail$:(OI)(CI)M" /T
Restart-Service sqail-service
```

```sql
-- On each SQL Server:
CREATE LOGIN [CORP\svc-sqail$] FROM WINDOWS;
USE Sales; CREATE USER [CORP\svc-sqail$] FOR LOGIN [CORP\svc-sqail$];
ALTER ROLE db_datareader ADD MEMBER [CORP\svc-sqail$];
```

For an ordinary domain account, set it in *services.msc → sqail-service →
Log On*, which also grants "Log on as a service". Then run the same `icacls`
command. When SQL Server runs on the gateway itself, *Local Service* works
as-is: `CREATE LOGIN [NT AUTHORITY\LOCAL SERVICE] FROM WINDOWS`.

### B6. Check everything

- [ ] `Get-Service sqail-service` shows **Running**.
- [ ] `curl.exe -sk https://127.0.0.1:7443/v1/health` on the gateway returns `{"status":"ok",…}`.
- [ ] From a user PC: `Test-NetConnection sqail-gw.corp.local -Port 7443` succeeds.
- [ ] From the gateway: `Test-NetConnection sqlserver01 -Port 1433` succeeds.
- [ ] In sqail2 with a user token: the connection is listed, and
      `SELECT @@SERVERNAME, SUSER_NAME()` returns the expected login.

---

## SQL Server certificates

Encryption is **required** by default, and the gateway verifies SQL Server's
TLS certificate the way a browser verifies a website's certificate. SQL Server
always runs TLS during login, so turning encryption down doesn't skip the
check.

| Your SQL Server has… | Do this |
|---|---|
| A certificate from a public CA, or from a company CA that is trusted on the gateway host (*Local Computer → Trusted Root Certification Authorities*) | Nothing. Use the host name the certificate was issued for (usually the FQDN). |
| No configured certificate: SQL Server generates a self-signed one (the default on many installations) | Either install a proper certificate on SQL Server (*Configuration Manager → Protocols → Properties → Certificate*), or tick **Trust server certificate** / `-TrustServerCertificate`. The link is still encrypted, but the server isn't authenticated, so use this only on networks you trust. |

The error for the self-signed case reads: *SQL Server's TLS certificate was
rejected: it is an old-style (X.509 v1) certificate…*

---

## Troubleshooting

| Symptom | Cause and fix |
|---|---|
| `Msg 18456 … Login failed for user` | Wrong login or password, or the server only allows Windows authentication. Check *Server Properties → Security → SQL Server and Windows Authentication mode*. The SQL Server error log shows the reason as a *State*: 8 = wrong password, 5 = unknown login, 38/40 = no access to the database. |
| `SQL Server's TLS certificate was rejected` | See [SQL Server certificates](#sql-server-certificates). |
| `SQL browser timeout during resolving instance` | The SQL Server Browser service isn't running, or UDP 1434 is blocked. Start the Browser service, or connect with `SERVER,PORT`. |
| `Connection refused` / `timed out connecting` | TCP/IP is disabled, the port is wrong, or a firewall is in the way. Test from the gateway with `Test-NetConnection <server> -Port 1433`. |
| `Login failed for user 'NT AUTHORITY\ANONYMOUS LOGON'` | Windows authentication from Local Service to a remote server. See [B5](#b5-windows-authentication-optional). |
| sqail2: `the token is not valid` / HTTP 401 | The token was revoked, mistyped, or comes from another gateway. Create a new one with `New-SqailToken.ps1`. |
| sqail2: `certificate fingerprint mismatch` | The gateway's certificate changed (a new install or a new certificate). Check the new fingerprint with `sqail-service --data-dir C:\ProgramData\sqail2\service fingerprint`, then in sqail2 use *Service → Forget this service* and connect again. If nothing changed on the gateway, treat it as a possible attack. |
| sqail2 can't reach the gateway | Does the admin page's *Overview* say "Only this computer can connect"? Then choose *Settings → Network → Other computers too*. Is the gateway's network profile *Public*? (The firewall rule only covers Domain and Private.) Test with `Test-NetConnection <gateway> -Port 7443`. |
| The service doesn't start | Read `C:\ProgramData\sqail2\service\logs\sqail-service.log` and *Event Viewer → Windows Logs → System* (source *Service Control Manager*). The usual causes are the port being in use, or an unreadable certificate or key file. |
| `requires the 'query' scope` | The user's token is `read`-scoped and the profile isn't read-only. Give them a `query` token, or mark the profile read-only. |

Logs never contain passwords, tokens or query results. Queries are recorded
in the audit log (admin: `GET /v1/audit`, see `docs\api.md`).

---

## Running the gateway

**Back up** the data folder: connections, tokens and the audit log. This is
safe while the service runs:

```powershell
& "C:\Program Files\sqail2\sqail-service.exe" --data-dir C:\ProgramData\sqail2\service backup D:\Backup\sqail
```

This writes `D:\Backup\sqail\service-<timestamp>.db`. For a nightly backup,
create a scheduled task (elevated):

```powershell
schtasks /Create /TN "sqail2 backup" /SC DAILY /ST 02:00 /RU SYSTEM /TR `
  "\"C:\Program Files\sqail2\sqail-service.exe\" --data-dir C:\ProgramData\sqail2\service backup D:\Backup\sqail"
```

Also keep a copy of **`C:\ProgramData\sqail2\service\master.key`** somewhere
**else**, such as a password manager or a vault. It encrypts the stored SQL
passwords. Without it, a backup restores everything except the passwords.
With it, anyone holding the backup can decrypt them.

**Restore:** `Stop-Service sqail-service`, copy the backup over
`service.db` (delete `service.db-wal` and `service.db-shm`), put
`master.key` back, and `Start-Service sqail-service`.

**Upgrade:** `Stop-Service sqail-service`, run the new MSI (or copy the new
`sqail-service.exe` over the old one), then `Start-Service sqail-service`.
Data is migrated automatically.

**Remove:** right-click `setup\Uninstall-SqailService.cmd` → *Run as
administrator*. Data is kept unless you run the `.ps1` with `-RemoveData`.

**Settings** live in `C:\ProgramData\sqail2\service\sqail-service.toml`
(row limits, timeouts, rate limits, audit options); every key is listed in
`sqail-service.example.toml`. Restart the service after changing it. More
detail is in `docs\operations.md`, and the threat model is in
`docs\security.md`.
