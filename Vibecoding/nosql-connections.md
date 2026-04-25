Add to SqaiL the ability to connect to NoSQL databases like MongoDB, Couchbase, surrealdb .

Start with SurrealDB. [www.surrealdb.com](https://surrealdb.com/docs)
Database objects should be displayed in the same way as SQL tables are displayed in the current version.
Create a clear plan and investigate first on what makes sense and what doesn't.
Research what SurrealDB can do and what it cannot do.
Research what typical NoSQL databases can do and what they cannot do.

# Investigation: NoSQL connections, starting with SurrealDB

## Current SqaiL architecture

- **Frontend connection model**
  - `src/types/connection.ts` defines `Driver = "postgres" | "mysql" | "sqlite" | "mssql" | "dbservice"`.
  - `src/components/ConnectionForm.tsx` renders one connection form from that driver list.
  - `src/stores/connectionStore.ts` calls Tauri commands such as `test_connection`, `connect`, and `disconnect`.

- **Frontend schema display**
  - `src/stores/schemaStore.ts` calls `list_schemas`, `list_tables`, `list_columns`, `list_indexes`, and `list_routines`.
  - `src/components/SchemaTree.tsx` already displays database objects by schema, table/view, columns, indexes, and routines.
  - This means the UI can mostly stay the same if a NoSQL backend can provide equivalent `SchemaInfo`, `TableInfo`, `ColumnInfo`, and `IndexInfo` values.

- **Native backend**
  - `src-tauri/src/db/connections.rs` defines the Rust-side driver enum and connection configuration.
  - `src-tauri/src/pool.rs` defines `DbPool` for SQLx pools, MSSQL, and DbService.
  - `src-tauri/src/commands.rs` creates pools and dispatches schema/query commands.
  - `src-tauri/src/schema.rs` contains provider-specific introspection for PostgreSQL, MySQL, SQLite, MSSQL, and DbService.
  - `src-tauri/src/query.rs` contains provider-specific query execution and value decoding.

- **DbService backend**
  - `sqail-dbservice/Sqail.DbService/Services/ConnectionManager.cs` currently stores `SqlConnection` and returns `IDbConnection`.
  - `sqail-dbservice/Sqail.DbService/Services/MetadataService.cs` is MSSQL-specific via `INFORMATION_SCHEMA`, `sys.*`, `SELECT DB_NAME()`, and `SELECT @@VERSION`.
  - This is not the right first extension point for SurrealDB unless the DbService is redesigned around provider interfaces.

## SurrealDB findings

- **What SurrealDB is good at**
  - Multi-model database: document, graph, time-series, relational-style records, geospatial, key-value, vector search, full-text search, and hybrid retrieval.
  - Uses SurrealQL, a SQL-like query language.
  - Has named tables, records, fields, indexes, functions, analyzers, users, namespaces, and databases.
  - Supports schemafull, schemaless, and mixed approaches.
  - Supports indexes on schemafull and schemaless tables.
  - Supports graph relationships and record links.
  - Supports HTTP `/sql` for arbitrary SurrealQL statements using `Surreal-NS` and `Surreal-DB` headers.
  - Supports HTTP `/signin` and RPC/WebSocket protocols.
  - Supports introspection via `INFO FOR DB`, `INFO FOR TABLE <table>`, and `INFO ... STRUCTURE`.

- **What maps well to SqaiL**
  - A SurrealDB namespace/database can map to SqaiL's schema/database browsing context.
  - SurrealDB tables can map to SqaiL `TableInfo`.
  - `INFO FOR DB` can list tables.
  - `INFO FOR TABLE <table> STRUCTURE` can list defined fields and indexes.
  - Double-clicking a table can insert `SELECT * FROM <table> LIMIT 100;`.
  - Query results from `/sql` can be normalized into SqaiL's existing `QueryResponse`.

- **Important constraints**
  - SurrealDB tables can be schemaless. A table may have no defined fields even though documents contain many fields.
  - The existing `ColumnInfo` model is relational and assumes a stable ordered column list.
  - For schemaless tables, columns are best-effort:
    - Defined fields from `INFO FOR TABLE ... STRUCTURE` are authoritative.
    - Sampled document keys from `SELECT * FROM <table> LIMIT n` are inferred, not guaranteed.
  - SurrealDB's current Rust SDK documentation says it requires Rust `1.89+`.
  - SqaiL currently declares `rust-version = "1.77.2"` in `src-tauri/Cargo.toml`.
  - Therefore, the official SurrealDB Rust SDK is probably not appropriate for the first implementation unless SqaiL upgrades its Rust toolchain requirement.
  - SqaiL already depends on `reqwest`, so the SurrealDB HTTP API is likely the safest first implementation path.

- **What does not make sense initially**
  - Do not force SurrealDB into `sqlx`; it is not a SQLx database.
  - Do not treat schemaless data as if it has strict relational columns.
  - Do not add MongoDB/Couchbase abstractions before proving the NoSQL metadata shape with SurrealDB.
  - Do not begin with embedded SurrealDB; remote HTTP connection support is simpler and closer to current SqaiL connections.
  - Do not implement live queries, graph visualizations, vector search UI, or record-level editing in the first pass.

## Typical NoSQL database findings

- **Common strengths**
  - Flexible or schema-optional data models.
  - Horizontal scale and high throughput.
  - Natural fit for nested documents, key-value access, graph traversal, or wide-column workloads depending on database type.
  - Often better for rapidly evolving application data than highly normalized relational schemas.
  - Many systems offer replication, sharding, and distributed availability features.

- **Common limitations**
  - Capabilities differ significantly between products.
  - No universal metadata model equivalent to `INFORMATION_SCHEMA`.
  - Joins may be limited, product-specific, expensive, or absent.
  - Transactions may be narrower than relational databases, though modern systems increasingly support ACID in specific scopes.
  - Schema inference can be expensive and incomplete.
  - Constraints, foreign keys, stored routines, views, and indexes are not portable concepts across NoSQL products.
  - Query languages vary widely: MongoDB query API/aggregation, Couchbase SQL++, SurrealQL, graph traversal languages, key-value APIs, etc.

## Recommended product model for SqaiL

- **Keep the existing schema tree shape where possible**
  - Continue showing `schemas -> tables -> columns/indexes`.
  - For NoSQL, rename only in display text if needed, not in internal contracts initially.
  - Example: SurrealDB tables still display under the existing "Tables" section.

- **Add provider capability flags**
  - Each driver should expose what it supports:
    - schemas/namespaces
    - tables/collections
    - fields/columns
    - indexes
    - routines/functions
    - foreign keys/relations
    - query validation
    - explain plans
    - mutations
  - The UI can hide or disable unsupported sections instead of showing errors.

- **Treat NoSQL fields as metadata with confidence**
  - Defined field: high confidence.
  - Inferred sampled field: low confidence.
  - Unknown schema: show a placeholder such as "No defined fields; schemaless table".

- **Normalize query results, not query languages**
  - SqaiL should not pretend every backend speaks SQL.
  - The editor can still run text queries and show result grids, but the driver decides how to execute and decode them.

## Phase 1: SurrealDB read/query support

- **Connection type**
  - Add `surrealdb` to frontend and Rust `Driver`.
  - Add SurrealDB-specific fields to `ConnectionConfig`:
    - `surrealUrl`, e.g. `http://localhost:8000`
    - `surrealNamespace`
    - `surrealDatabase`
    - `surrealUsername`
    - `surrealPassword`
    - optional auth mode later
  - Alternatively reuse `host`, `port`, `database`, `user`, `password` and add only `namespace`, but explicit fields avoid confusion with SQL databases.

- **HTTP client**
  - Add a `SurrealDbClient` struct in `src-tauri/src/pool.rs`.
  - Store:
    - base URL
    - namespace
    - database
    - username/password or bearer token
    - `reqwest::Client`
  - Add `DbPool::SurrealDb(Arc<SurrealDbClient>)`.

- **Test connection**
  - Use `GET /version` or `GET /health` to check server availability.
  - Run a minimal authenticated query via `POST /sql`, for example `RETURN 1;`, with `Surreal-NS` and `Surreal-DB` headers.

- **Connect**
  - Build the HTTP client and insert it into `state.pools`.
  - Prefer basic auth first if it works with root/database users.
  - Add `/signin` token support only if needed for common deployments.

- **Query execution**
  - Implement SurrealDB execution in `query.rs`.
  - Send editor contents to `POST /sql`.
  - Convert each SurrealDB response item into `QueryResult`.
  - For array/object records:
    - Build columns from object keys.
    - Always include `id` when present.
    - Preserve nested objects/arrays as JSON values.
  - For scalar responses:
    - Return one column named `value`.

- **Schema display**
  - `list_schemas`: return one schema named from the configured namespace/database, for example `<namespace>/<database>` or just the database name.
  - `list_tables`: run `INFO FOR DB;`, read the `tables` object keys, return each as `TableInfo { tableType: "table" }`.
  - `list_columns`: run `INFO FOR TABLE <table> STRUCTURE;`.
    - Use `fields` as defined columns.
    - If no fields are defined, optionally sample `SELECT * FROM <table> LIMIT 25;` and infer field names from returned objects.
  - `list_indexes`: use `INFO FOR TABLE <table> STRUCTURE;` indexes.
  - `list_routines`: return functions from `INFO FOR DB` only if mapping is useful; otherwise return empty for phase 1.
  - `list_foreign_keys`: return empty for phase 1.

- **Editor helpers**
  - Double-click table should insert `SELECT * FROM <table> LIMIT 100;`.
  - Quoting/escaping should be SurrealQL-aware where table names need backticks or escaped identifiers.

## Phase 2: Better SurrealDB metadata

- **Improve field display**
  - Show whether fields are defined or inferred.
  - Show SurrealDB field type where available.
  - Show table schema mode: `SCHEMAFULL`, `SCHEMALESS`, or mixed/unknown.

- **Improve object categories**
  - Add optional display sections for:
    - functions
    - analyzers
    - models
    - params
    - APIs
  - Only add these if they fit naturally into the existing schema tree.

- **Authentication**
  - Support `/signin` bearer tokens.
  - Support root, namespace, database, and possibly record-user auth later.

- **Validation**
  - Disable current SQL validation for SurrealDB initially.
  - Later add SurrealQL-aware parsing or server-side dry-run/explain if available.

## Phase 3: General NoSQL provider abstraction

- **Introduce provider interfaces**
  - Move driver-specific query/schema logic toward a common trait-like structure:
    - connect/test
    - execute query
    - list logical databases/schemas
    - list object containers
    - list fields
    - list indexes
    - capability flags
  - This prepares for MongoDB and Couchbase without forcing all databases into SQL table semantics.

- **MongoDB likely mapping**
  - database -> schema
  - collection -> table
  - document keys -> inferred columns
  - indexes -> indexes
  - aggregation/query shell syntax is not SQL.

- **Couchbase likely mapping**
  - bucket/scope -> schema grouping
  - collection -> table
  - SQL++ queries can fit SqaiL's text editor better than MongoDB commands.
  - indexes and scopes are first-class.

## Open decisions before coding

- **Rust version**
  - Decide whether SqaiL can raise `rust-version` from `1.77.2`.
  - If not, use SurrealDB HTTP API, not the official Rust SDK.

- **Connection form UX**
  - Decide whether SurrealDB reuses host/port/user/password/database fields plus a namespace field, or gets explicit SurrealDB-specific fields.

- **Schemaless column inference**
  - Decide whether phase 1 should sample documents or only show defined fields.
  - Sampling is friendlier but can be slow or misleading on large/heterogeneous tables.

- **Naming**
  - Decide whether the tree should continue saying "Tables" for SurrealDB, or use a driver-specific label later.

## Recommended next implementation step

Start with a minimal SurrealDB HTTP backend:

1. Add `surrealdb` as a driver in TypeScript and Rust.
2. Add connection form support for URL, namespace, database, username, and password.
3. Add `SurrealDbClient` and `DbPool::SurrealDb`.
4. Implement `test_connection`, `connect`, `execute_query`.
5. Implement `list_schemas`, `list_tables`, `list_columns`, and `list_indexes` using `INFO`.
6. Disable routines, foreign keys, and SQL validation for SurrealDB.
7. Verify with a local SurrealDB instance and a small dataset containing both schemafull and schemaless tables.
