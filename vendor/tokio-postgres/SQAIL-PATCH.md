# Vendored tokio-postgres 0.7.18 (sqail patch)

Upstream: https://github.com/sfackler/rust-postgres (MIT OR Apache-2.0)

The only change: `SimpleColumn` keeps the column's type OID from the
`RowDescription` message (`SimpleColumn::type_oid()`). Upstream parses it and then
discards it, which leaves simple-query callers unable to tell an int from a string.

sqail-service runs editor scripts through the simple-query protocol, because it
handles multiple statements and returns column headers even for empty results. So
it needs these types.

Wired in with `[patch.crates-io]` in the workspace `Cargo.toml`. Drop the vendor
copy once upstream exposes the type OID.
