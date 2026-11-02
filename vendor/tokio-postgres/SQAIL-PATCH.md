# Vendored tokio-postgres 0.7.18 (sqail patch)

Upstream: https://github.com/sfackler/rust-postgres (MIT OR Apache-2.0)

Two changes:

1. `SimpleColumn` keeps the column's type OID from the `RowDescription` message
   (`SimpleColumn::type_oid()`). Upstream parses it and then discards it, which
   leaves simple-query callers unable to tell an int from a string.

   sqail-service runs editor scripts through the simple-query protocol, because it
   handles multiple statements and returns column headers even for empty results.
   So it needs these types.

2. Row-limited execution (#59): `Client::simple_query_limited` runs one statement
   through the extended protocol with text-format results and a row limit on
   `Execute`, and returns the same messages as `simple_query_raw`;
   `Client::query_raw_limited` is `query_raw` with a row limit. Both close the
   unnamed portal before `Sync`. The server stops executing once it has sent the
   rows asked for, so `SELECT * FROM big_table` with `max_rows = 100` reads 101
   rows instead of the whole table. `SimpleQueryStream` and `RowStream` skip the
   extended-protocol acknowledgements (`ParseComplete`, `BindComplete`, `NoData`,
   `PortalSuspended`, `CloseComplete`).

Wired in with `[patch.crates-io]` in the workspace `Cargo.toml`. Drop the vendor
copy once upstream exposes the type OID and a row-limited query.
