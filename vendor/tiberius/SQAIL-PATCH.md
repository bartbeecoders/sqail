# Vendored tiberius 0.13.0 (sqail patch)

Upstream: https://github.com/prisma/tiberius (MIT OR Apache-2.0)

Wired in with `[patch.crates-io]` in the workspace `Cargo.toml`. Drop the
vendor copy once upstream has equivalents of the changes below. Every
change is marked `sqail patch` in the source.

## 1. Attention acknowledgement across messages (#59)

`Client::cancel_query` reads on into the next response message for the
Attention acknowledgement (`cancel_request` in `src/client/connection.rs`,
`flush_done_attention` in `src/tds/stream/token.rs`). When the cancelled
request was still sending rows, SQL Server ends that response message and
acknowledges the Attention in a message of its own. Upstream stops at the end
of the first message, fails with "Never got a DONE token acknowledging the
Attention signal", and leaves the acknowledgement on the wire, so every later
response on the connection is read one message late.

sqail-service cancels a single query this way once it has `max_rows` rows.

## 2. Attention while the request's future runs (#148)

`cancel_query` borrows the client, so it can only run after the future that
reads the response was dropped, and dropping that future mid-token desyncs
the connection. sqail-service needs Stop/Esc to abort just the running batch,
as SSMS does, without `KILL` (which ends the whole session and needs
`ALTER ANY CONNECTION`).

* `src/client/attention.rs` (new): `AttentionHandle`, a cloneable handle
  (`Client::attention_handle`) whose `request()` sets a flag and wakes the
  connection's pending read.
* `Connection::poll_attention`, called from the connection's packet stream
  (`poll_next`): when an Attention is requested and a response is
  outstanding (`!flushed`), it writes the Attention packet between packet
  reads, so it never splits a message, and the response runs on to the
  acknowledging DONE as usual.
* `TokenStream::get_done*` note an acknowledging DONE
  (`Connection::note_attention_ack`).
* `Client::finish_attention`: after the response is read, reads the
  acknowledgement if it comes in a message of its own, sends a requested
  Attention that never went out while a response is still unread, clears
  the request and says whether there was one.
* `cancel_request` does not send a second Attention when one from the handle
  is already out; it reads that one's acknowledgement instead.
* `AttentionHandle` is re-exported from the crate root.

Unit tests: `requested_attention_goes_out_on_the_next_read`,
`no_attention_without_a_request` and
`attention_requested_after_the_response_is_not_sent` in
`src/tds/stream/token.rs`.
