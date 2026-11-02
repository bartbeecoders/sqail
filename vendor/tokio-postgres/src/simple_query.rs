use crate::client::{InnerClient, Responses};
use crate::codec::FrontendMessage;
use crate::connection::RequestMessages;
use crate::query::extract_row_affected;
use crate::{Error, SimpleQueryMessage, SimpleQueryRow};
use bytes::Bytes;
use fallible_iterator::FallibleIterator;
use futures_util::Stream;
use log::debug;
use pin_project_lite::pin_project;
use postgres_protocol::message::backend::Message;
use postgres_protocol::message::frontend;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, ready};

/// Information about a column of a single query row.
#[derive(Debug)]
pub struct SimpleColumn {
    name: String,
    type_oid: u32,
}

impl SimpleColumn {
    pub(crate) fn new(name: String, type_oid: u32) -> SimpleColumn {
        SimpleColumn { name, type_oid }
    }

    /// Returns the OID of the column's type, as sent in `RowDescription`.
    /// (sqail patch: upstream drops this, but simple-query callers need it to
    /// interpret the text-format values.)
    pub fn type_oid(&self) -> u32 {
        self.type_oid
    }

    /// Returns the name of the column.
    pub fn name(&self) -> &str {
        &self.name
    }
}

pub async fn simple_query(client: &InnerClient, query: &str) -> Result<SimpleQueryStream, Error> {
    debug!("executing simple query: {query}");

    let buf = encode(client, query)?;
    let responses = client.send(RequestMessages::Single(FrontendMessage::Raw(buf)))?;

    Ok(SimpleQueryStream {
        responses,
        columns: None,
    })
}

/// sqail patch: run one statement through the extended protocol with text
/// results and a row limit on `Execute` (0 = no limit). The server stops
/// executing once it has sent `max_rows` rows, so a big `SELECT` costs no
/// more than the rows asked for. Messages come back as for `simple_query`;
/// the portal is closed before `Sync`, which leaves transaction state as a
/// simple query would.
pub async fn simple_query_limited(
    client: &InnerClient,
    query: &str,
    max_rows: i32,
) -> Result<SimpleQueryStream, Error> {
    debug!("executing limited query ({max_rows} rows): {query}");

    let buf = client.with_buf(|buf| {
        frontend::parse("", query, std::iter::empty(), buf).map_err(Error::encode)?;
        let r = frontend::bind(
            "",
            "",
            std::iter::empty(),
            std::iter::empty::<()>(),
            |(), _| Ok(postgres_protocol::IsNull::No),
            Some(0),
            buf,
        );
        match r {
            Ok(()) => {}
            Err(frontend::BindError::Conversion(e)) => return Err(Error::to_sql(e, 0)),
            Err(frontend::BindError::Serialization(e)) => return Err(Error::encode(e)),
        }
        frontend::describe(b'P', "", buf).map_err(Error::encode)?;
        frontend::execute("", max_rows, buf).map_err(Error::encode)?;
        frontend::close(b'P', "", buf).map_err(Error::encode)?;
        frontend::sync(buf);
        Ok(buf.split().freeze())
    })?;
    let responses = client.send(RequestMessages::Single(FrontendMessage::Raw(buf)))?;

    Ok(SimpleQueryStream {
        responses,
        columns: None,
    })
}

pub async fn batch_execute(client: &InnerClient, query: &str) -> Result<(), Error> {
    debug!("executing statement batch: {query}");

    let buf = encode(client, query)?;
    let mut responses = client.send(RequestMessages::Single(FrontendMessage::Raw(buf)))?;

    loop {
        match responses.next().await? {
            Message::ReadyForQuery(_) => return Ok(()),
            Message::CommandComplete(_)
            | Message::EmptyQueryResponse
            | Message::RowDescription(_)
            | Message::DataRow(_) => {}
            _ => return Err(Error::unexpected_message()),
        }
    }
}

fn encode(client: &InnerClient, query: &str) -> Result<Bytes, Error> {
    client.with_buf(|buf| {
        frontend::query(query, buf).map_err(Error::encode)?;
        Ok(buf.split().freeze())
    })
}

pin_project! {
    /// A stream of simple query results.
    #[project(!Unpin)]
    pub struct SimpleQueryStream {
        responses: Responses,
        columns: Option<Arc<[SimpleColumn]>>,
    }
}

impl Stream for SimpleQueryStream {
    type Item = Result<SimpleQueryMessage, Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.project();
        loop {
            let item = match ready!(this.responses.poll_next(cx)?) {
                // Extended-protocol acknowledgements (`simple_query_limited`).
                Message::ParseComplete
                | Message::BindComplete
                | Message::NoData
                | Message::PortalSuspended
                | Message::CloseComplete => continue,
                Message::CommandComplete(body) => {
                    let rows = extract_row_affected(&body)?;
                    SimpleQueryMessage::CommandComplete(rows)
                }
                Message::EmptyQueryResponse => SimpleQueryMessage::CommandComplete(0),
                Message::RowDescription(body) => {
                    let columns: Arc<[SimpleColumn]> = body
                        .fields()
                        .map(|f| Ok(SimpleColumn::new(f.name().to_string(), f.type_oid())))
                        .collect::<Vec<_>>()
                        .map_err(Error::parse)?
                        .into();

                    *this.columns = Some(columns.clone());
                    SimpleQueryMessage::RowDescription(columns)
                }
                Message::DataRow(body) => match &this.columns {
                    Some(columns) => {
                        SimpleQueryMessage::Row(SimpleQueryRow::new(columns.clone(), body)?)
                    }
                    None => return Poll::Ready(Some(Err(Error::unexpected_message()))),
                },
                Message::ReadyForQuery(_) => return Poll::Ready(None),
                _ => return Poll::Ready(Some(Err(Error::unexpected_message()))),
            };
            return Poll::Ready(Some(Ok(item)));
        }
    }
}
