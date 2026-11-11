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

## 2. Messages, row counts and errors in order (#150)

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
