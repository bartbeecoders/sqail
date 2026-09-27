//! Run SQL through sqail-service from the command line.
//!
//!     SQAIL_TOKEN=sq2_… cargo run -p sqail-client --example query -- "<connection name>" "SELECT 1"
//!
//! SQAIL_URL defaults to https://127.0.0.1:7443. The certificate is pinned to
//! SQAIL_FINGERPRINT, or (for local development) whatever the service presents.

use futures::StreamExt;
use sqail_client::proto::{QueryEvent, QueryRequest};
use sqail_client::{Client, On, Target, Trust};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let (Some(conn), Some(sql)) = (args.next(), args.next()) else {
        anyhow::bail!("usage: query <connection name or id> <sql>");
    };
    let url = std::env::var("SQAIL_URL").unwrap_or_else(|_| "https://127.0.0.1:7443".into());
    let token = std::env::var("SQAIL_TOKEN").map_err(|_| anyhow::anyhow!("set SQAIL_TOKEN"))?;
    let fingerprint = match std::env::var("SQAIL_FINGERPRINT") {
        Ok(fp) => fp,
        Err(_) => sqail_client::probe(&url).await?.0,
    };
    let client = Client::new(&Target {
        url,
        token,
        trust: Trust::Pinned(fingerprint),
        identity: None,
    })?;

    let conns = client.connections().await?;
    let target = conns
        .iter()
        .find(|c| c.name == conn || c.id.to_string() == conn)
        .ok_or_else(|| {
            let names: Vec<_> = conns.iter().map(|c| c.name.as_str()).collect();
            anyhow::anyhow!("no connection '{conn}'; have: {names:?}")
        })?;

    let mut q = client
        .query(On::Connection(target.id), &QueryRequest::new(sql))
        .await?;
    while let Some(ev) = q.events.next().await {
        match ev? {
            QueryEvent::ResultStart { columns, .. } => {
                let names: Vec<_> = columns.iter().map(|c| c.name.as_str()).collect();
                println!("{}", names.join("\t"));
            }
            QueryEvent::Rows { rows, .. } => {
                for row in rows {
                    let cells: Vec<String> = row
                        .iter()
                        .map(|v| match v {
                            serde_json::Value::String(s) => s.clone(),
                            serde_json::Value::Null => "NULL".into(),
                            other => other.to_string(),
                        })
                        .collect();
                    println!("{}", cells.join("\t"));
                }
            }
            QueryEvent::ResultEnd {
                row_count,
                truncated,
                ..
            } => {
                println!(
                    "({row_count} rows{})\n",
                    if truncated { ", truncated" } else { "" }
                );
            }
            QueryEvent::RowsAffected { count } => println!("({count} rows affected)"),
            QueryEvent::Message { severity, text } => println!("{severity}: {text}"),
            QueryEvent::Error { message, .. } => eprintln!("error: {message}"),
            QueryEvent::Done { elapsed_ms, .. } => println!("-- {elapsed_ms} ms"),
            QueryEvent::Started { .. } => {}
        }
    }
    Ok(())
}
