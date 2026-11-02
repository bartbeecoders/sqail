# Vendored tiberius 0.13.0 (sqail patch)

Upstream: https://github.com/prisma/tiberius (MIT OR Apache-2.0)

The only change: `Client::cancel_query` reads on into the next response
message for the Attention acknowledgement (`cancel_request` in
`src/client/connection.rs`, `flush_done_attention` in
`src/tds/stream/token.rs`). When the cancelled request was still sending
rows, SQL Server ends that response message and acknowledges the Attention
in a message of its own. Upstream stops at the end of the first message,
fails with "Never got a DONE token acknowledging the Attention signal", and
leaves the acknowledgement on the wire, so every later response on the
connection is read one message late.

sqail-service cancels a single query this way once it has `max_rows` rows
(#59). Wired in with `[patch.crates-io]` in the workspace `Cargo.toml`. Drop
the vendor copy once upstream reads the acknowledgement across messages.
