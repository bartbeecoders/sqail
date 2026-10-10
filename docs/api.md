# sqail-service REST API

The complete, machine-readable reference is [`openapi.json`](openapi.json).
It is the document the service serves at `GET /v1/openapi.json`, and a test
keeps the checked-in copy in sync. Debug builds, or `docs_ui = true`, also
serve interactive docs at `/docs`. This page covers the parts a schema can't
express. Rust clients can use `crates/sqail-client`, which adds certificate
pinning and a typed event stream.

## Basics

* HTTPS only, HTTP/1.1 or HTTP/2. JSON bodies (`content-type:
  application/json`).
* Every route except `GET /v1/health` and `GET /v1/openapi.json` needs
  `Authorization: Bearer sq2_…`.
* Every response carries an `x-request-id`. Send your own to correlate logs.
* Errors are RFC 9457 problem documents (`application/problem+json`):

  ```json
  {"title": "Forbidden", "status": 403, "code": "forbidden", "detail": "requires the 'query' scope"}
  ```

  Status codes: 400 invalid request, 401
  missing or unknown token, 403 insufficient scope, 404 not found, 409
  conflict (for example a session that is already running a query), 413 body
  too large, 429 rate limited, 502 database unreachable, 504 timeout.

```bash
URL=https://127.0.0.1:7443
TOKEN=sq2_...                       # sqail-service token create --name me --scope admin
api() { curl -sk -H "Authorization: Bearer $TOKEN" -H 'content-type: application/json' "$@"; }
api $URL/v1/info
```

(`-k` is for the self-signed dev certificate. Against a real deployment, pin
the certificate with `--pinnedpubkey` or use a trusted certificate.)

## Connection profiles

```bash
api $URL/v1/connections -d '{
  "name": "shop (dev)",
  "params": {"engine": "postgres", "host": "db.internal", "port": 5432,
             "database": "shop", "user": "app", "ssl_mode": "verify-full"},
  "password": "…",
  "read_only": false, "color": "#2e86de", "environment": "dev"
}'
```

`params` by engine:

| `engine` | Fields |
|---|---|
| `postgres` | `host`, `port` (5432), `database`, `user`, `ssl_mode` (`disable` · `prefer` · `require` · `verify-ca` · `verify-full`), `ssl_root_cert`, `ssl_client_cert` |
| `mssql` | `host`, `port` (1433), `instance`, `database`, `auth` (see below), `encrypt` (`off` · `on` · `required`), `trust_server_certificate` |
| `sqlite` | `path` (inside `sqlite.allowed_dirs`), `create` |

SQL Server `auth`:

| `method` | Fields | `password` holds |
|---|---|---|
| `sql` | `user` | the login's password |
| `integrated` | none (the service's Windows account) | nothing |
| `entra_password` | `user`, optional `tenant` (default `organizations`), optional `client_id` (default: Microsoft's SqlClient app) | the Entra password |
| `entra_service_principal` | `tenant`, `client_id` | the client secret |
| `entra_managed_identity` | optional `client_id` (a user-assigned identity) | nothing |

`password` is write-only. On `PUT`, leaving it out keeps the stored password,
and `""` clears it. Responses report only `has_password`.

`ssl_client_key` (PostgreSQL only) is write-only in the same way: leave it
out to keep the stored key, and `""` clears it. Responses report only
`has_ssl_client_key`. The key is an unencrypted PEM private key (PKCS#8,
PKCS#1 or SEC1). `ssl_root_cert` and `ssl_client_cert` are PEM and come back
with the profile. A CA certificate is used only with `ssl_mode` `verify-ca`
or `verify-full`, and then it replaces the system trust store. A client
certificate and its key are a pair. Certificates with `ssl_mode` `disable`
are rejected.

Two admin-only routes take an unsaved profile (the same body as `POST
/v1/connections`) for the connection form:
`POST /v1/connections/test` connects and reports the server version, and
`POST /v1/connections/databases` lists the databases the login can access
(`[{"name": "…"}]`; `name` may be empty; PostgreSQL connects to `postgres` when `database` is
empty). Add `?secret_from=<id>` to either route to use the password and
client key stored with profile `<id>` when the body omits them. A value in
the body, including `""`, is used instead. `POST /v1/connections` takes
`?secret_from=<id>` too, to copy those secrets into a new profile (the UI's
*Duplicate…*).

`POST /v1/connections/{id}/azure/discover` (admin) lists the databases in the
Azure subscriptions that a SQL Server profile's Microsoft Entra ID identity
can read: Azure SQL servers, SQL managed instances and PostgreSQL flexible
servers. The service gets an Azure Resource Manager token with the profile's
credentials (the Azure CLI's public client for `entra_password` without a
`client_id`). Other profiles get `400`; a failed sign-in or subscription list
`502`. Servers or subscriptions that can't be listed are reported in
`warnings` instead of failing the request:

```json
{"subscriptions": [{"id": "…", "name": "Development"}],
 "databases": [{"kind": "sql_server", "subscription_id": "…", "resource_group": "rg-data",
   "server": "sql1", "host": "sql1.database.windows.net", "port": 1433,
   "database": "sales", "location": "westeurope", "admin_login": "sqladmin"}],
 "warnings": ["Production (Azure SQL): AuthorizationFailed: …"]}
```

`kind` is `sql_server`, `sql_managed_instance` or `postgres_flexible`.

## Running SQL

`POST /v1/connections/{id}/query` runs a script on a pooled connection. The
response is **NDJSON** (`application/x-ndjson`), one event per line, streamed
while the database produces rows:

```bash
api $URL/v1/connections/$ID/query -d '{"sql": "SELECT id, name FROM customers; UPDATE t SET x = 1", "max_rows": 1000}'
```

```
{"event":"started","query_id":"8c0…"}
{"event":"result_start","index":0,"columns":[{"name":"id","type_name":"int4","logical":"int"},{"name":"name","type_name":"text","logical":"text"}]}
{"event":"rows","index":0,"rows":[[1,"Ada"],[2,"Linus"]]}
{"event":"result_end","index":0,"row_count":2,"truncated":false}
{"event":"rows_affected","count":3}
{"event":"done","elapsed_ms":4,"cancelled":false}
```

* Order: `started`, then for each statement either `result_start` →
  `rows`\* → `result_end`, or `rows_affected`. `message` events (notices,
  `PRINT`) may appear anywhere. The stream always ends with exactly one `done`,
  and an `error` stops the script but is still followed by `done`.
* Once the stream has started, the HTTP status is 200. Failures inside the
  script arrive as `error` events (`code`, `message` and `db_code`, which holds
  the SQLSTATE or SQL Server error number).
* Cells: `null`; bools and ints (i64) as JSON bool and number; floats as
  numbers (`"NaN"`/`"Infinity"` as strings); bytes as lowercase hex;
  everything else as a string (decimals keep full precision, temporal values
  are ISO-8601, JSON columns are JSON text). `logical` says which applies.
* `max_rows` caps each result set (the service clamps it to
  `limits.max_rows`), and `truncated: true` says the cap was hit.
  `timeout_ms` cancels the query server-side.
* **Parameters:** with `params`, the script must be exactly one statement,
  and placeholders are the engine's own (`$1`, `@P1`, `?1`):
  `{"sql": "SELECT * FROM t WHERE id = $1", "params": [42]}`.
* Scripts are split server-side, and SQL Server `GO` lines are honoured.

**Cancel:** send `x-query-id: <uuid>` with the query, or read `query_id` from
`started`, then call `DELETE /v1/queries/{query_id}`. The stream finishes with
`done` and `cancelled: true`. Closing the HTTP connection stops the query too:
the service notices when it can no longer deliver events, and it discards a
connection that was interrupted instead of returning it to the pool.

## Sessions and transactions

Pooled queries may run on a different connection each time. For transactions,
temp tables or `SET` options, open a session: a dedicated connection that is
yours until you close it.

```bash
SID=$(api $URL/v1/sessions -d "{\"connection_id\":\"$ID\"}" | jq -r .id)
api $URL/v1/sessions/$SID/query -d '{"sql":"BEGIN; UPDATE t SET x = 2"}'
#   … {"event":"done",…,"in_transaction":true}
api $URL/v1/sessions/$SID/query -d '{"sql":"COMMIT"}'
api -X DELETE $URL/v1/sessions/$SID
```

A session runs one query at a time (a second one gets 409). `done` reports
whether a transaction is still open. Sessions idle for
`sessions.idle_timeout_secs` are closed, and closing a session rolls back any
open transaction.

## Plans

`POST /v1/connections/{id}/explain` (or `/v1/sessions/{id}/explain`) with
`{"sql": "…", "analyze": false}` returns a `Plan`: a tree of nodes with
`label`, `object`, `cost`, `rows` and, when analysed, `actual_rows` and
`actual_ms`, plus engine-specific `props`. `raw` holds the engine's own plan
(Postgres JSON, SQL Server showplan XML, SQLite rows). With `analyze: true`,
Postgres and SQL Server execute the statement inside a transaction that is
always rolled back.

## Schema

All schema routes are `GET`s under `/v1/connections/{id}/schema/`:
`databases`, `schemas`, `tables?schema=`, `routines?schema=`, and
`columns`, `indexes`, `foreign-keys` and `privileges`, each with
`?schema=&name=` for a table (`schema` defaults to the connection's current
schema). An index's `constraint` flag says it backs a `PRIMARY KEY` or
`UNIQUE` constraint. `privileges` lists the table's grants (owner excluded),
its owner (Postgres) and the roles and users that could be granted
privileges; `supported` is `false` on SQLite. `GET
/v1/connections/{id}/ddl?schema=&name=` returns a best-effort `CREATE`
script for a table, view or routine. `read` tokens may browse every profile.

## Administration

`/v1/tokens` (list, create, and revoke with `DELETE /v1/tokens/{id}`) and
`/v1/audit?limit=&before=` need the `admin` scope. The token secret is
returned only in the `POST /v1/tokens` response.
