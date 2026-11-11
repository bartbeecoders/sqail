use crate::tds::codec::{TokenDone, TokenError, TokenInfo};
use crate::tds::stream::ReceivedToken;
use crate::{row::ColumnType, Column, Row};
use futures_util::{
    ready,
    stream::{BoxStream, Peekable, Stream, StreamExt, TryStreamExt},
};
use std::{
    fmt::Debug,
    pin::Pin,
    sync::Arc,
    task::{self, Poll},
};

/// A set of `Streams` of [`QueryItem`] values, which can be either result
/// metadata or a row.
///
/// The `QueryStream` needs to be polled empty before sending another query to
/// the [`Client`], failing to do so causes a flush before the next query,
/// slowing it down in an undeterministic way.
///
/// Every stream starts with metadata, describing the structure of the incoming
/// rows, e.g. the columns in the order they are presented in every row.
///
/// If after consuming rows from the stream, another metadata result arrives, it
/// means the stream has multiple results from different queries. This new
/// metadata item will describe the next rows from here forwards.
///
/// If having one set of results in the response, using [`into_row_stream`]
/// might be more convenient to use.
///
/// The struct provides non-streaming APIs with [`into_results`],
/// [`into_first_result`] and [`into_row`].
///
/// # Example
///
/// ```
/// # use tiberius::{Config, QueryItem};
/// # use tokio_util::compat::TokioAsyncWriteCompatExt;
/// # use std::env;
/// # use futures_util::stream::TryStreamExt;
/// # #[tokio::main]
/// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
/// # let c_str = env::var("TIBERIUS_TEST_CONNECTION_STRING").unwrap_or(
/// #     "server=tcp:localhost,1433;integratedSecurity=true;TrustServerCertificate=true".to_owned(),
/// # );
/// # let config = Config::from_ado_string(&c_str)?;
/// # let tcp = tokio::net::TcpStream::connect(config.get_addr()).await?;
/// # tcp.set_nodelay(true)?;
/// # let mut client = tiberius::Client::connect(config, tcp.compat_write()).await?;
/// let mut stream = client
///     .query(
///         "SELECT @P1 AS first; SELECT @P2 AS second",
///         &[&1i32, &2i32],
///     )
///     .await?;
///
/// // The stream consists of four items, in the following order:
/// // - Metadata from `SELECT 1`
/// // - The only resulting row from `SELECT 1`
/// // - Metadata from `SELECT 2`
/// // - The only resulting row from `SELECT 2`
/// while let Some(item) = stream.try_next().await? {
///     match item {
///         // our first item is the column data always
///         QueryItem::Metadata(meta) if meta.result_index() == 0 => {
///             // the first result column info can be handled here
///         }
///         // ... and from there on from 0..N rows
///         QueryItem::Row(row) if row.result_index() == 0 => {
///             assert_eq!(Some(1), row.get(0));
///         }
///         // the second result set returns first another metadata item
///         QueryItem::Metadata(meta) => {
///             // .. handling
///         }
///         // ...and, again, we get rows from the second resultset
///         QueryItem::Row(row) => {
///             assert_eq!(Some(2), row.get(0));
///         }
///     }
/// }
/// # Ok(())
/// # }
/// ```
///
/// [`Client`]: struct.Client.html
/// [`into_row_stream`]: struct.QueryStream.html#method.into_row_stream
/// [`into_results`]: struct.QueryStream.html#method.into_results
/// [`into_first_result`]: struct.QueryStream.html#method.into_first_result
/// [`into_row`]: struct.QueryStream.html#method.into_row
pub struct QueryStream<'a> {
    token_stream: Peekable<BoxStream<'a, crate::Result<ReceivedToken>>>,
    columns: Option<Arc<Vec<Column>>>,
    result_set_index: Option<usize>,
    /// sqail patch: also yield INFO, DONE and ERROR tokens
    /// (`Client::set_messages`).
    messages: bool,
}

impl<'a> Debug for QueryStream<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QueryStream")
            .field(
                "token_stream",
                &"BoxStream<'a, crate::Result<ReceivedToken>>",
            )
            .finish()
    }
}

impl<'a> QueryStream<'a> {
    pub(crate) fn new(token_stream: BoxStream<'a, crate::Result<ReceivedToken>>) -> Self {
        Self {
            token_stream: token_stream.peekable(),
            columns: None,
            result_set_index: None,
            messages: false,
        }
    }

    /// sqail patch: see `Client::set_messages`.
    pub(crate) fn with_messages(mut self, messages: bool) -> Self {
        self.messages = messages;
        self
    }

    /// sqail patch: whether `token` is yielded as a [`QueryItem`] of its own.
    fn is_message(messages: bool, token: &ReceivedToken) -> bool {
        messages
            && matches!(
                token,
                ReceivedToken::Info(_)
                    | ReceivedToken::Error(_)
                    | ReceivedToken::Done(_)
                    | ReceivedToken::DoneInProc(_)
                    | ReceivedToken::DoneProc(_)
            )
    }

    /// Moves the stream forward until having result metadata, stream end or an
    /// error.
    pub(crate) async fn forward_to_metadata(&mut self) -> crate::Result<()> {
        loop {
            let item = Pin::new(&mut self.token_stream)
                .peek()
                .await
                .map(|r| r.as_ref().map_err(|e| e.clone()))
                .transpose()?;

            match item {
                Some(ReceivedToken::NewResultset(_)) => break,
                // sqail patch: left for the consumer, in order.
                Some(token) if Self::is_message(self.messages, token) => break,
                Some(_) => {
                    self.token_stream.try_next().await?;
                }
                None => break,
            }
        }

        Ok(())
    }

    /// The list of columns either for the current result set, or for the next
    /// one. If the stream is just created, or if the next item in the stream
    /// contains metadata, the metadata will be taken from the stream. Otherwise
    /// the columns will be returned from the cache and reflect on the current
    /// result set.
    ///
    /// # Example
    ///
    /// ```
    /// # use tiberius::Config;
    /// # use tokio_util::compat::TokioAsyncWriteCompatExt;
    /// # use std::env;
    /// # use futures_util::stream::TryStreamExt;
    /// # #[tokio::main]
    /// # async fn main() -> anyhow::Result<()> {
    /// # let c_str = env::var("TIBERIUS_TEST_CONNECTION_STRING").unwrap_or(
    /// #     "server=tcp:localhost,1433;integratedSecurity=true;TrustServerCertificate=true".to_owned(),
    /// # );
    /// # let config = Config::from_ado_string(&c_str)?;
    /// # let tcp = tokio::net::TcpStream::connect(config.get_addr()).await?;
    /// # tcp.set_nodelay(true)?;
    /// # let mut client = tiberius::Client::connect(config, tcp.compat_write()).await?;
    /// let mut stream = client
    ///     .query(
    ///         "SELECT @P1 AS first; SELECT @P2 AS second",
    ///         &[&1i32, &2i32],
    ///     )
    ///     .await?;
    ///
    /// // Nothing is fetched, the first result set starts.
    /// let cols = stream.columns().await?.unwrap();
    /// assert_eq!("first", cols[0].name());
    ///
    /// // Move over the metadata.
    /// stream.try_next().await?;
    ///
    /// // We're in the first row, seeing the metadata for that set.
    /// let cols = stream.columns().await?.unwrap();
    /// assert_eq!("first", cols[0].name());
    ///
    /// // Move over the only row in the first set.
    /// stream.try_next().await?;
    ///
    /// // End of the first set, getting the metadata by peaking the next item.
    /// let cols = stream.columns().await?.unwrap();
    /// assert_eq!("second", cols[0].name());
    /// # Ok(())
    /// # }
    /// ```
    pub async fn columns(&mut self) -> crate::Result<Option<&[Column]>> {
        use ReceivedToken::*;

        loop {
            let item = Pin::new(&mut self.token_stream)
                .peek()
                .await
                .map(|r| r.as_ref().map_err(|e| e.clone()))
                .transpose()?;

            match item {
                Some(token) => match token {
                    NewResultset(metadata) => {
                        self.columns = Some(Arc::new(metadata.columns().collect()));
                        break;
                    }
                    Row(_) => {
                        break;
                    }
                    // sqail patch: an error is not skipped silently.
                    Error(e) if self.messages => {
                        let e = crate::Error::Server(e.clone());
                        self.token_stream.try_next().await?;
                        return Err(e);
                    }
                    _ => {
                        self.token_stream.try_next().await?;
                        continue;
                    }
                },
                None => {
                    break;
                }
            }
        }

        Ok(self.columns.as_ref().map(|c| c.as_slice()))
    }

    /// Collects results from all queries in the stream into memory in the order
    /// of querying.
    pub async fn into_results(self) -> crate::Result<Vec<Vec<Row>>> {
        collect_results(self).await
    }

    /// Collects the output of the first query, dropping any further
    /// results.
    pub async fn into_first_result(self) -> crate::Result<Vec<Row>> {
        let mut results = self.into_results().await?.into_iter();
        let rows = results.next().unwrap_or_default();

        Ok(rows)
    }

    /// Collects the first row from the output of the first query, dropping any
    /// further rows.
    pub async fn into_row(self) -> crate::Result<Option<Row>> {
        let mut results = self.into_first_result().await?.into_iter();

        Ok(results.next())
    }

    /// Convert the stream into a stream of rows, skipping metadata items.
    pub fn into_row_stream(self) -> BoxStream<'a, crate::Result<Row>> {
        let s = self.try_filter_map(|item| async {
            match item {
                QueryItem::Row(row) => Ok(Some(row)),
                QueryItem::Error(e) => Err(crate::Error::Server(e)),
                _ => Ok(None),
            }
        });

        Box::pin(s)
    }
}

/// Collect a stream of [`QueryItem`]s into one `Vec<Row>` per result set, in
/// stream order.
///
/// Each result set is delimited by a [`QueryItem::Metadata`] item: a metadata
/// item opens a new (initially empty) result set and every subsequent
/// [`QueryItem::Row`] is appended to it. Nothing is discarded on the assumption
/// that the first item is metadata — if a row were ever to arrive before any
/// metadata it is still captured into a result set rather than silently dropped.
///
/// Behaviour preserved from the original implementation:
/// - an empty stream yields `Ok(vec![])`;
/// - a result set with zero rows still yields an (empty) inner `Vec` in the
///   right position (metadata with no following rows -> one empty result set);
/// - ordering across multiple result sets is preserved.
async fn collect_results<S>(mut stream: S) -> crate::Result<Vec<Vec<Row>>>
where
    S: Stream<Item = crate::Result<QueryItem>> + Unpin,
{
    let mut results: Vec<Vec<Row>> = Vec::new();
    let mut current: Option<Vec<Row>> = None;
    // sqail patch: with messages on, errors come as items; fail with the
    // first one after reading the rest, as the token stream does without.
    let mut first_error = None;

    while let Some(item) = stream.try_next().await? {
        match item {
            QueryItem::Metadata(_) => {
                // A new result set begins. Flush the previous one (if any) and
                // open a fresh, empty set so a zero-row result still produces an
                // inner Vec at the correct position.
                if let Some(previous) = current.take() {
                    results.push(previous);
                }
                current = Some(Vec::new());
            }
            QueryItem::Row(row) => {
                // Rows are always preceded by their metadata in a well-formed
                // stream, so `current` is normally `Some`. Be defensive and open
                // a result set on the fly rather than discard a leading row.
                current.get_or_insert_with(Vec::new).push(row);
            }
            QueryItem::Error(e) => {
                first_error.get_or_insert(e);
            }
            QueryItem::Message(_) | QueryItem::Done(_) => {}
        }
    }
    if let Some(e) = first_error {
        return Err(crate::Error::Server(e));
    }

    if let Some(last) = current.take() {
        results.push(last);
    }

    Ok(results)
}

/// Info about the following stream of rows.
#[derive(Debug, Clone)]
pub struct ResultMetadata {
    pub(crate) columns: Arc<Vec<Column>>,
    pub(crate) result_index: usize,
}

impl ResultMetadata {
    /// Column info. The order is the same as in the following rows.
    pub fn columns(&self) -> &[Column] {
        &self.columns
    }

    /// The number of the result set, an incrementing value starting from zero,
    /// which gives an indication of the position of the result set in the
    /// stream.
    pub fn result_index(&self) -> usize {
        self.result_index
    }
}

/// Resulting data from a query.
#[derive(Debug)]
pub enum QueryItem {
    /// A single row of data.
    Row(Row),
    /// Information of the upcoming row data.
    Metadata(ResultMetadata),
    /// sqail patch, only with `Client::set_messages`: an informational
    /// message (INFO token: `PRINT`, `RAISERROR` up to severity 10). It has
    /// the same fields as an error.
    Message(TokenError),
    /// sqail patch, only with `Client::set_messages`: an error, where the
    /// server sent it. The response goes on; the server decides whether the
    /// rest of the batch runs.
    Error(TokenError),
    /// sqail patch, only with `Client::set_messages`: the end of a
    /// statement, procedure or batch (DONE, DONEINPROC, DONEPROC).
    Done(StatementDone),
}

/// sqail patch: a DONE, DONEINPROC or DONEPROC token
/// ([`QueryItem::Done`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatementDone {
    kind: DoneKind,
    rows: Option<u64>,
    more: bool,
    error: bool,
    attention: bool,
}

/// sqail patch: which token a [`StatementDone`] came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DoneKind {
    /// DONE: a statement of the batch, or the batch.
    Done,
    /// DONEINPROC: a statement inside a procedure.
    InProc,
    /// DONEPROC: a procedure (or RPC) call.
    Proc,
}

impl StatementDone {
    fn new(kind: DoneKind, done: &TokenDone) -> Self {
        Self {
            kind,
            rows: done.count(),
            more: done.has_more(),
            error: done.is_error(),
            attention: done.is_attention(),
        }
    }

    /// Which token this is.
    pub fn kind(&self) -> DoneKind {
        self.kind
    }

    /// The statement's row count (rows affected, or rows returned by a
    /// SELECT), when the server sent one: not with `SET NOCOUNT ON`, and not
    /// for statements that do not count rows.
    pub fn rows(&self) -> Option<u64> {
        self.rows
    }

    /// More DONE tokens follow in this response.
    pub fn has_more(&self) -> bool {
        self.more
    }

    /// The statement ended with an error.
    pub fn is_error(&self) -> bool {
        self.error
    }

    /// The token acknowledges an Attention.
    pub fn is_attention(&self) -> bool {
        self.attention
    }
}

impl From<TokenInfo> for TokenError {
    fn from(i: TokenInfo) -> Self {
        TokenError {
            code: i.number,
            state: i.state,
            class: i.class,
            message: i.message,
            server: i.server,
            procedure: i.procedure,
            line: i.line,
        }
    }
}

impl QueryItem {
    pub(crate) fn metadata(columns: Arc<Vec<Column>>, result_index: usize) -> Self {
        Self::Metadata(ResultMetadata {
            columns,
            result_index,
        })
    }

    /// Returns a reference to the metadata, if the item is of a correct variant.
    pub fn as_metadata(&self) -> Option<&ResultMetadata> {
        match self {
            QueryItem::Metadata(ref metadata) => Some(metadata),
            _ => None,
        }
    }

    /// Returns a reference to the row, if the item is of a correct variant.
    pub fn as_row(&self) -> Option<&Row> {
        match self {
            QueryItem::Row(ref row) => Some(row),
            _ => None,
        }
    }

    /// Returns the metadata, if the item is of a correct variant.
    pub fn into_metadata(self) -> Option<ResultMetadata> {
        match self {
            QueryItem::Metadata(metadata) => Some(metadata),
            _ => None,
        }
    }

    /// Returns the row, if the item is of a correct variant.
    pub fn into_row(self) -> Option<Row> {
        match self {
            QueryItem::Row(row) => Some(row),
            _ => None,
        }
    }
}

impl<'a> Stream for QueryStream<'a> {
    type Item = crate::Result<QueryItem>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut task::Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        loop {
            let token = match ready!(this.token_stream.poll_next_unpin(cx)) {
                Some(res) => res?,
                None => return Poll::Ready(None),
            };

            return match token {
                ReceivedToken::NewResultset(meta) => {
                    let column_meta = meta
                        .columns
                        .iter()
                        .map(|x| Column {
                            name: x.col_name.to_string(),
                            column_type: ColumnType::from(&x.base.ty),
                        })
                        .collect::<Vec<_>>();

                    let column_meta = Arc::new(column_meta);
                    this.columns = Some(column_meta.clone());

                    this.result_set_index = this.result_set_index.map(|i| i + 1);

                    let query_item =
                        QueryItem::metadata(column_meta, *this.result_set_index.get_or_insert(0));

                    return Poll::Ready(Some(Ok(query_item)));
                }
                ReceivedToken::Row(data) => {
                    let Some(columns) = this.columns.as_ref() else {
                        return Poll::Ready(Some(Err(crate::Error::Protocol(
                            "ROW token arrived before any column metadata".into(),
                        ))));
                    };
                    let columns = columns.clone();
                    let result_index = this.result_set_index.unwrap_or(0);

                    let row = Row {
                        columns,
                        data,
                        result_index,
                    };

                    Poll::Ready(Some(Ok(QueryItem::Row(row))))
                }
                // sqail patch: messages, row counts and errors in order.
                ReceivedToken::Info(info) if this.messages => {
                    Poll::Ready(Some(Ok(QueryItem::Message(info.into()))))
                }
                ReceivedToken::Error(e) if this.messages => {
                    Poll::Ready(Some(Ok(QueryItem::Error(e))))
                }
                ReceivedToken::Done(d) if this.messages => Poll::Ready(Some(Ok(
                    QueryItem::Done(StatementDone::new(DoneKind::Done, &d)),
                ))),
                ReceivedToken::DoneInProc(d) if this.messages => Poll::Ready(Some(Ok(
                    QueryItem::Done(StatementDone::new(DoneKind::InProc, &d)),
                ))),
                ReceivedToken::DoneProc(d) if this.messages => Poll::Ready(Some(Ok(
                    QueryItem::Done(StatementDone::new(DoneKind::Proc, &d)),
                ))),
                _ => continue,
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::row::ColumnType;
    use crate::tds::codec::TokenRow;
    use crate::Column;
    use futures_util::stream;

    fn columns() -> Arc<Vec<Column>> {
        Arc::new(vec![Column::new("c".to_string(), ColumnType::Int4)])
    }

    fn meta(cols: &Arc<Vec<Column>>, result_index: usize) -> QueryItem {
        QueryItem::metadata(cols.clone(), result_index)
    }

    fn row(cols: &Arc<Vec<Column>>, result_index: usize) -> QueryItem {
        QueryItem::Row(Row {
            columns: cols.clone(),
            data: TokenRow::new(),
            result_index,
        })
    }

    async fn collect(items: Vec<QueryItem>) -> Vec<Vec<Row>> {
        let stream = stream::iter(items.into_iter().map(Ok::<_, crate::error::Error>));
        collect_results(stream).await.expect("collect_results")
    }

    #[tokio::test]
    async fn empty_stream_yields_no_result_sets() {
        assert!(collect(vec![]).await.is_empty());
    }

    #[tokio::test]
    async fn metadata_without_rows_yields_one_empty_result_set() {
        let cols = columns();
        let results = collect(vec![meta(&cols, 0)]).await;
        assert_eq!(results.len(), 1);
        assert!(results[0].is_empty());
    }

    #[tokio::test]
    async fn multiple_empty_result_sets_are_preserved() {
        let cols = columns();
        let results = collect(vec![meta(&cols, 0), meta(&cols, 1)]).await;
        assert_eq!(results.len(), 2);
        assert!(results[0].is_empty());
        assert!(results[1].is_empty());
    }

    #[tokio::test]
    async fn rows_are_grouped_by_result_set_in_order() {
        let cols = columns();
        let results = collect(vec![
            meta(&cols, 0),
            row(&cols, 0),
            row(&cols, 0),
            meta(&cols, 1),
            row(&cols, 1),
        ])
        .await;

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].len(), 2);
        assert_eq!(results[1].len(), 1);
    }

    #[tokio::test]
    async fn leading_row_is_not_discarded() {
        // The old implementation consumed and threw away the first stream item,
        // assuming it was metadata. If a row led, it was silently lost. The
        // robust implementation keeps every row: a stream of two leading rows
        // must produce one result set containing BOTH rows (the old logic would
        // have produced a single-row set).
        let cols = columns();
        let results = collect(vec![row(&cols, 0), row(&cols, 0)]).await;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].len(), 2);
    }

    #[tokio::test]
    async fn single_metadata_with_rows_yields_one_result_set_of_n_rows() {
        // One metadata item followed by N rows must collapse into exactly one
        // result set holding all N rows.
        let cols = columns();
        let results = collect(vec![
            meta(&cols, 0),
            row(&cols, 0),
            row(&cols, 0),
            row(&cols, 0),
        ])
        .await;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].len(), 3);
    }

    #[tokio::test]
    async fn collect_propagates_errors() {
        // An error partway through the stream must surface rather than be
        // silently swallowed by the collection loop.
        let cols = columns();
        let items = vec![
            Ok(meta(&cols, 0)),
            Ok(row(&cols, 0)),
            Err(crate::Error::Protocol("boom".into())),
        ];
        let stream = stream::iter(items);
        let err = collect_results(stream).await.expect_err("expected error");
        assert!(matches!(err, crate::Error::Protocol(_)));
    }

    // --- QueryItem accessor helpers -------------------------------------

    #[test]
    fn query_item_accessors_on_metadata() {
        let cols = columns();
        let item = meta(&cols, 7);

        assert!(item.as_metadata().is_some());
        assert!(item.as_row().is_none());
        assert_eq!(item.as_metadata().unwrap().result_index(), 7);

        let md = item.into_metadata().expect("metadata");
        assert_eq!(md.result_index(), 7);
        assert_eq!(md.columns().len(), 1);
    }

    #[test]
    fn query_item_accessors_on_row() {
        let cols = columns();
        let item = row(&cols, 3);

        assert!(item.as_row().is_some());
        assert!(item.as_metadata().is_none());
        assert_eq!(item.as_row().unwrap().result_index, 3);

        let r = item.into_row().expect("row");
        assert_eq!(r.result_index, 3);
    }

    #[test]
    fn query_item_into_wrong_variant_is_none() {
        let cols = columns();
        assert!(meta(&cols, 0).into_row().is_none());
        assert!(row(&cols, 0).into_metadata().is_none());
    }

    // --- Full QueryStream state machine (poll_next + into_* helpers) -----
    //
    // These build a `QueryStream` from an in-memory stream of `ReceivedToken`s
    // (no server), exercising `poll_next` (result-set indexing, the
    // ROW-before-metadata guard) and the `into_results`/`into_first_result`/
    // `into_row`/`into_row_stream` helpers end to end.

    use crate::tds::codec::{
        BaseMetaDataColumn, FixedLenType, MetaDataColumn, TokenColMetaData, TypeInfo,
    };
    use crate::tds::stream::ReceivedToken;
    use futures_util::stream::{StreamExt, TryStreamExt};
    use std::borrow::Cow;

    fn token_meta(name: &'static str) -> ReceivedToken {
        let col = MetaDataColumn {
            base: BaseMetaDataColumn {
                flags: enumflags2::BitFlags::empty(),
                ty: TypeInfo::FixedLen(FixedLenType::Int4),
                table_name: None,
            },
            col_name: Cow::Borrowed(name),
        };
        ReceivedToken::NewResultset(Arc::new(TokenColMetaData { columns: vec![col] }))
    }

    fn token_row() -> ReceivedToken {
        ReceivedToken::Row(TokenRow::new())
    }

    fn query_stream(tokens: Vec<ReceivedToken>) -> QueryStream<'static> {
        let s = stream::iter(tokens.into_iter().map(Ok::<_, crate::Error>));
        QueryStream::new(s.boxed())
    }

    #[tokio::test]
    async fn query_stream_into_results_groups_and_indexes_result_sets() {
        let stream = query_stream(vec![
            token_meta("first"),
            token_row(),
            token_row(),
            token_meta("second"),
            token_row(),
        ]);

        let results = stream.into_results().await.expect("into_results");
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].len(), 2);
        assert_eq!(results[1].len(), 1);

        // result_index must increment across result sets.
        assert!(results[0].iter().all(|r| r.result_index == 0));
        assert!(results[1].iter().all(|r| r.result_index == 1));
    }

    #[tokio::test]
    async fn query_stream_empty_yields_no_results() {
        let results = query_stream(vec![]).into_results().await.expect("empty");
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn query_stream_metadata_only_yields_one_empty_result_set() {
        let results = query_stream(vec![token_meta("first")])
            .into_results()
            .await
            .expect("metadata only");
        assert_eq!(results.len(), 1);
        assert!(results[0].is_empty());
    }

    #[tokio::test]
    async fn query_stream_row_before_metadata_is_protocol_error() {
        // The rewritten poll_next must reject a ROW token that arrives before
        // any column metadata rather than panic or silently drop it.
        let err = query_stream(vec![token_row()])
            .into_results()
            .await
            .expect_err("expected protocol error");
        assert!(matches!(err, crate::Error::Protocol(_)));
    }

    #[tokio::test]
    async fn query_stream_into_first_result_and_into_row() {
        let first = query_stream(vec![
            token_meta("first"),
            token_row(),
            token_row(),
            token_meta("second"),
            token_row(),
        ])
        .into_first_result()
        .await
        .expect("into_first_result");
        assert_eq!(first.len(), 2);

        let one = query_stream(vec![token_meta("first"), token_row(), token_row()])
            .into_row()
            .await
            .expect("into_row");
        assert!(one.is_some());

        let none = query_stream(vec![token_meta("first")])
            .into_row()
            .await
            .expect("into_row empty");
        assert!(none.is_none());
    }

    #[tokio::test]
    async fn query_stream_into_row_stream_skips_metadata() {
        let rows: Vec<Row> = query_stream(vec![
            token_meta("first"),
            token_row(),
            token_meta("second"),
            token_row(),
            token_row(),
        ])
        .into_row_stream()
        .try_collect()
        .await
        .expect("into_row_stream");
        assert_eq!(rows.len(), 3);
    }

    // --- sqail patch: messages, row counts and errors in order -----------

    fn token_info(message: &str) -> ReceivedToken {
        ReceivedToken::Info(TokenInfo {
            number: 0,
            state: 1,
            class: 0,
            message: message.into(),
            server: String::new(),
            procedure: String::new(),
            line: 1,
        })
    }

    fn token_error(message: &str) -> ReceivedToken {
        ReceivedToken::Error(TokenError {
            code: 50000,
            state: 1,
            class: 16,
            message: message.into(),
            server: String::new(),
            procedure: String::new(),
            line: 2,
        })
    }

    const DONE_MORE: u16 = 0x1;
    const DONE_COUNT: u16 = 0x10;

    fn script() -> Vec<ReceivedToken> {
        vec![
            token_info("hello"),
            token_meta("a"),
            token_row(),
            ReceivedToken::Done(TokenDone::test_new(DONE_MORE | DONE_COUNT, 1)),
            token_error("boom"),
            ReceivedToken::DoneInProc(TokenDone::test_new(DONE_MORE | DONE_COUNT, 3)),
            token_meta("b"),
            ReceivedToken::Done(TokenDone::test_new(0, 0)),
        ]
    }

    fn outline(item: &QueryItem) -> String {
        match item {
            QueryItem::Metadata(m) => format!("meta:{}", m.columns()[0].name()),
            QueryItem::Row(_) => "row".into(),
            QueryItem::Message(m) => format!("msg:{}", m.message()),
            QueryItem::Error(e) => format!("error:{}:{}", e.code(), e.message()),
            QueryItem::Done(d) => format!("done:{:?}:{:?}", d.kind(), d.rows()),
        }
    }

    #[tokio::test]
    async fn with_messages_infos_dones_and_errors_come_in_order() {
        let mut stream = query_stream(script()).with_messages(true);
        // As the clients do before handing the stream out.
        stream.forward_to_metadata().await.unwrap();
        let items: Vec<String> = stream
            .map(|i| outline(&i.unwrap()))
            .collect::<Vec<_>>()
            .await;
        assert_eq!(
            items,
            [
                "msg:hello",
                "meta:a",
                "row",
                "done:Done:Some(1)",
                "error:50000:boom",
                "done:InProc:Some(3)",
                "meta:b",
                "done:Done:None",
            ]
        );
    }

    #[tokio::test]
    async fn without_messages_only_results_come() {
        let mut stream = query_stream(script());
        stream.forward_to_metadata().await.unwrap();
        let items: Vec<String> = stream
            .map(|i| outline(&i.unwrap()))
            .collect::<Vec<_>>()
            .await;
        assert_eq!(items, ["meta:a", "row", "meta:b"]);
    }

    #[tokio::test]
    async fn with_messages_collecting_still_fails_with_the_first_error() {
        let err = query_stream(script())
            .with_messages(true)
            .into_results()
            .await
            .expect_err("the error token");
        assert!(matches!(err, crate::Error::Server(e) if e.message() == "boom"));
        let err = query_stream(script())
            .with_messages(true)
            .into_row_stream()
            .try_collect::<Vec<_>>()
            .await
            .expect_err("the error token");
        assert!(matches!(err, crate::Error::Server(_)));
    }
}
