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
* `AttentionHandle::packets_read` counts the packets the connection has
  read (`Connection::poll_next`), so that a canceller can tell a response
  still arriving after the Attention from a server that went quiet (#215).

Unit tests: `requested_attention_goes_out_on_the_next_read`,
`no_attention_without_a_request`,
`attention_requested_after_the_response_is_not_sent` and
`the_handle_counts_packets_read` in
`src/tds/stream/token.rs`.

## 3. Exact money, smallmoney and smalldatetime types (#151)

- `money`/`smallmoney` decode to `ColumnData::Numeric` with scale 4 (the
  scaled integer on the wire) instead of `ColumnData::F64`, which cannot
  hold money's 19 significant digits (`src/tds/codec/column_data/money.rs`).
  Reading a money column as `f64` through `FromSql` no longer works; read it
  as `Numeric`.
- `ColumnType::from(&TypeInfo)` tells the nullable forms apart by length:
  `MONEYN` of 4 bytes is `Money4`, `DATETIMEN` of 4/8 bytes is
  `Datetime4`/`Datetime` (`src/row.rs`). Upstream reports `Money` and
  `Datetimen`, so a nullable `smallmoney` or `smalldatetime` column looked
  like `money` or `datetime`.

## 4. Messages, row counts and errors in order (#150)

Upstream's `QueryStream` drops INFO tokens (`PRINT`, `RAISERROR` up to
severity 10) and DONE row counts, and the token stream holds back the first
ERROR until the response ends, so an error mid-batch surfaces after the
result sets that followed it.

* `Client::set_messages(true)` (flag on `Connection`, read in
  `Connection::query_tokens`) makes the streams of `query`, `simple_query`
  and `Query::query` yield three more `QueryItem` variants where the server
  sent the tokens: `Message(TokenError)` (an INFO token, same fields),
  `Error(TokenError)` and `Done(StatementDone)` (DONE, DONEINPROC or
  DONEPROC: `kind`, `rows` when DONE_COUNT is set, `has_more`, `is_error`,
  `is_attention`). Off by default; then nothing changes.
* With it on, `TokenStream::with_messages` does not keep the first ERROR to
  fail with at the end of the response, `forward_to_metadata` stops at those
  tokens instead of skipping them, and `columns` fails on an ERROR instead of
  skipping it. `into_results`/`into_row`/`into_first_result` still fail with
  the first error (after reading the response) and `into_row_stream` fails
  at it.
* `QueryItem`'s accessors use wildcard arms; `StatementDone` and `DoneKind`
  are re-exported from the crate root; `TokenDone` gains `count`,
  `has_more` and `is_error`.

Unit tests: `with_messages_infos_dones_and_errors_come_in_order`,
`without_messages_only_results_come` and
`with_messages_collecting_still_fails_with_the_first_error` in
`src/tds/stream/query.rs`.
