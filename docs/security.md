# sqail security: threat model and checklist

This covers **sqail-service**, the only component that talks to databases, and
the **sqail** desktop client. Each mitigation below names the test that checks
it.

## What we protect

| Asset | Where it lives |
|---|---|
| Database credentials | `service.db` (encrypted, AES-256-GCM); in memory while a pool is open |
| Master key | `<data-dir>/master.key` (mode 0600) or `SQAIL_MASTER_KEY` |
| API tokens | Client: OS credential store (fallback: `tokens.toml`, mode 0600). Service: only a SHA-256 hash |
| Query results | In transit (TLS); never logged; kept in UI memory only |
| Audit log | `service.db`: who ran what, where, and how long it took. Never row values |

## Trust boundaries

```
sqail ──TLS 1.3 + bearer token (+ optional client cert)──▶ sqail-service ──driver TLS──▶ databases
   │                                                               │
   └─ config dir (settings, history, tokens.toml)                 └─ data dir (service.db, master.key, certs)
```

* The **network** between sqail and the service is untrusted. It is always
  HTTPS, and plain HTTP is refused.
* The **service host** is trusted. Anyone who can read the data dir can read
  both `master.key` and `service.db`. Encryption at rest protects copies and
  backups of `service.db` on their own, not a compromised host.
* **Databases** are trusted to be what the profile says they are. For
  Postgres, `verify-full` authenticates the server; `prefer`/`require` only
  encrypt the link.

## Threats and mitigations

| Threat | Mitigation | Checked by |
|---|---|---|
| Eavesdropping or tampering between client and service | TLS 1.3 only (1.2 opt-in); rustls/aws-lc-rs; no plain HTTP listener | `health_needs_no_token…` (it only speaks HTTPS); `smoke.sh` |
| A man-in-the-middle with a self-signed certificate | The client pins the SHA-256 fingerprint on first use, after the user confirms it | `pinning_rejects_other_certificates` |
| Stolen or guessed API token | 256-bit random tokens; only SHA-256 hashes stored; constant-time compare; revocation; per-token rate limit; `last_used_at` tracking | `token_lifecycle`, `scopes_and_revocation`, `rate_limit_applies_per_token` |
| Unauthorised use of the service | Every route except `/v1/health` and the OpenAPI doc needs a token; scopes `read` < `query` < `admin`; `read` can only query read-only profiles | `scopes_and_revocation`, `read_scope_only_queries_read_only_profiles` |
| Stronger client authentication | Optional mutual TLS (`tls.client_ca`), on top of tokens | `mutual_tls_requires_a_client_certificate` |
| Credential disclosure through the API | Passwords are write-only: never returned; `has_password` only | `passwords_are_write_only` |
| Credential disclosure at rest | AES-256-GCM with a random nonce per value; tampering detected | `round_trip`, `tampering_is_detected`, `wrong_key_fails` |
| Secrets or row values in logs | Authorization is a sensitive header; values are never logged; audit stores SQL text only (can be disabled) | `logs_contain_no_secrets_or_row_values` |
| SQL injection through catalog browsing | All names travel as bind parameters; the one exception (SQLite schema) must match an attached database first | `introspect.rs` review; engine matrix tests |
| Reading arbitrary files through SQLite profiles | Paths must be absolute and inside `sqlite.allowed_dirs` after resolving `..` and symlinks; empty list = SQLite off | `path_must_be_inside_allowed_dirs`, `sqlite_outside_allowed_dirs_is_rejected` |
| Writes through a "read-only" profile | Postgres: `default_transaction_read_only`; SQLite: opened read-only; the `read` scope is limited to read-only profiles | `engine_postgres`/`engine_sqlite` (read-only part) |
| Resource exhaustion | Body limit (8 MiB); request timeout; per-query row cap and timeout; pool size per profile; sessions capped per token and reaped when idle; the client caps NDJSON line length | `max_rows_truncates`, `timeout_stops_a_running_query`, `NdjsonDecoder` |
| Crashes on hostile input | Lexers, splitters, completer, formatter and the NDJSON decoder are fuzzed | `cargo run --profile fuzz -p sqail-fuzz -- <secs>` (10 min: 13.2M inputs, 0 crashes; one real bug found and fixed; 2 min on every CI run) |
| Abandoned transactions holding locks | A transaction left open outside a session is rolled back and reported; idle sessions close (rollback); the UI asks before closing a tab or quitting with one open | `open_transaction_outside_session_is_rolled_back`, UI test |
| EXPLAIN ANALYZE changing data | Runs inside a transaction that is always rolled back | engine matrix (`sum(price)` unchanged) |
| The AI assistant changing data, or doing more than read | The CLI runs with no built-in tools (Claude Code `--tools ""` + `--strict-mcp-config`; Grok `--tools search_tool,use_tool` + `--permission-mode dontAsk` + an allow list of sqail's tools, which also denies the user's other MCP servers). `run_query` accepts one `SELECT`/`WITH`/`VALUES` statement without writing keywords or side-effect functions, and runs it in a transaction that is always rolled back (`BEGIN READ ONLY` on Postgres). The service token reaches `sqail mcp` through the environment, never arguments or files | `assistant::guard` tests, `mcp_tools_on_sql_server` (a forced `INSERT` is rolled back), `postgres_assistant_transactions_are_read_only`, `assistant_panel_streams_an_answer_and_inserts_sql` |
| Vulnerable or non-permissive dependencies | `cargo audit` and `cargo deny` (advisories, licences, sources) in the gate | `scripts/check.sh` |

## Known limitations (accepted for 1.0)

* **The AI assistant sends data to its provider.** Questions, schema details
  and up to `assistant.max_rows` rows per assistant query go to Anthropic
  (Claude Code) or xAI (Grok) under the user's own account. It uses the
  user's service token, so it sees exactly what the user may see. On SQL
  Server a rolled-back transaction undoes writes but can't stop every side
  effect of a procedure; the statement check refuses `EXEC` for that reason.

* **SQL Server read-only profiles** use `ApplicationIntent=ReadOnly`, which is
  only enforced on availability-group secondaries. Use a login with read-only
  rights.
* **SQL Server cancel** uses `KILL`, which needs `ALTER ANY CONNECTION`.
  Without it the service drops the connection, and the server finishes the
  batch in the background.
* **Database driver logs at debug level** echo SQL text, which can contain
  literals. The default filter keeps them at warn; do not raise it in
  production.
* **The audit log stores SQL text** (`audit.log_sql = true`, truncated to 4 KiB).
  Turn it off where queries embed sensitive literals.
* **Unauthenticated requests are not rate limited per IP.** Tokens cannot be
  guessed (256 bits), but a flood of requests with bad tokens still costs a
  hash and a lookup each. Put a reverse proxy in front when exposing the
  service beyond a trusted network.
* **An admin token can change the service's settings** through the admin
  page (`/v1/admin`): which address it listens on, its certificate, and
  which folders SQLite profiles may open. That last one means an admin
  token can read any SQLite file the service account can reach. Treat admin
  tokens like credentials for the service host, give everyone else `query`
  or `read`, and set `admin_ui = false` where settings should only change
  on the host itself.
* **mTLS and fingerprint probing:** the first-run fingerprint check cannot
  complete against a service that requires client certificates. Configure
  such services in `settings.toml` (see docs/operations.md).
* `RUSTSEC-2026-0097` (`rand` 0.7, unsound with a custom logger) is ignored.
  It reaches us only through tiberius' Windows-only integrated-auth
  dependency, and sqail installs no such logger.

## Reporting a vulnerability

Please report privately to the maintainer (see the repository's contact
details) rather than opening a public issue.
