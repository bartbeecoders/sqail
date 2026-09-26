//! Query plans for each engine, normalised into `PlanNode` trees.

use serde_json::Value;
use sqail_proto::{Engine, Plan, PlanNode, PlanProp};

use super::{Conn, DbError, Result, fetch_results};

/// Explain `sql` on `conn`. With `analyze`, the statement runs inside a
/// transaction that is always rolled back (SQLite has no actual plans).
pub async fn explain(conn: &mut dyn Conn, sql: &str, analyze: bool) -> Result<Plan> {
    let sql = sql.trim().trim_end_matches(';');
    if sql.is_empty() {
        return Err(DbError::Invalid("nothing to explain".into()));
    }
    match conn.engine() {
        Engine::Postgres => {
            let opts = if analyze {
                "ANALYZE, BUFFERS, FORMAT JSON"
            } else {
                "FORMAT JSON"
            };
            let script = if analyze {
                format!("BEGIN; EXPLAIN ({opts}) {sql}; ROLLBACK")
            } else {
                format!("EXPLAIN ({opts}) {sql}")
            };
            let res = fetch_results(conn, &script).await;
            if analyze && res.is_err() {
                // Leave no transaction behind on this connection.
                let _ = fetch_results(conn, "ROLLBACK").await;
            }
            let res = res?;
            let raw =
                first_text(&res).ok_or_else(|| DbError::Internal("empty EXPLAIN output".into()))?;
            let json: Value =
                serde_json::from_str(&raw).map_err(|e| DbError::Internal(e.to_string()))?;
            let roots = json
                .as_array()
                .map(|stmts| {
                    stmts
                        .iter()
                        .filter_map(|s| s.get("Plan"))
                        .map(pg_node)
                        .collect()
                })
                .unwrap_or_default();
            Ok(Plan {
                roots,
                raw: serde_json::to_string_pretty(&json).unwrap_or(raw),
                analyzed: analyze,
            })
        }
        Engine::Mssql => {
            let set = if analyze {
                "STATISTICS XML"
            } else {
                "SHOWPLAN_XML"
            };
            if analyze {
                // Actual plans execute the statement: never keep its effects.
                fetch_results(conn, "BEGIN TRANSACTION").await?;
            }
            fetch_results(conn, &format!("SET {set} ON")).await?;
            // A lone DML statement would take the driver's row-count path,
            // which drops result sets (and so the plan); two statements do not.
            let res = fetch_results(conn, &format!("SET NOCOUNT ON; {sql}")).await;
            // Always switch it back off; this connection goes back to a pool.
            let _ = fetch_results(conn, &format!("SET {set} OFF")).await;
            if analyze {
                let _ = fetch_results(conn, "IF @@TRANCOUNT > 0 ROLLBACK").await;
            }
            let res = res?;
            // The plan is the last result set (after the query's own rows
            // when analyzing).
            let raw = res
                .iter()
                .rev()
                .find_map(|(cols, rows)| {
                    let is_plan = cols.len() == 1 && cols[0].name.contains("Showplan");
                    is_plan.then(|| {
                        rows.first()
                            .and_then(|r| r.first())
                            .and_then(Value::as_str)
                            .map(String::from)
                    })?
                })
                .ok_or_else(|| DbError::Internal("no showplan returned".into()))?;
            let roots = mssql_roots(&raw).map_err(DbError::Internal)?;
            Ok(Plan {
                roots,
                raw,
                analyzed: analyze,
            })
        }
        Engine::Sqlite => {
            let res = fetch_results(conn, &format!("EXPLAIN QUERY PLAN {sql}")).await?;
            let rows = res.into_iter().next().map(|(_, r)| r).unwrap_or_default();
            let flat: Vec<(i64, i64, String)> = rows
                .iter()
                .map(|r| {
                    (
                        r.first().and_then(Value::as_i64).unwrap_or(0),
                        r.get(1).and_then(Value::as_i64).unwrap_or(0),
                        r.get(3).and_then(Value::as_str).unwrap_or("").to_string(),
                    )
                })
                .collect();
            let raw = flat
                .iter()
                .map(|(id, parent, d)| format!("{id}|{parent}|{d}"))
                .collect::<Vec<_>>()
                .join("\n");
            Ok(Plan {
                roots: sqlite_tree(&flat, 0),
                raw,
                analyzed: false,
            })
        }
    }
}

fn first_text(res: &[(Vec<sqail_proto::Column>, Vec<Vec<Value>>)]) -> Option<String> {
    res.iter()
        .find_map(|(_, rows)| rows.first().and_then(|r| r.first()).cloned())
        .map(|v| match v {
            Value::String(s) => s,
            other => other.to_string(),
        })
}

// ------------------------------------------------------------- postgres --

fn pg_node(v: &Value) -> PlanNode {
    let s = |k: &str| v.get(k).and_then(Value::as_str).map(String::from);
    let f = |k: &str| v.get(k).and_then(Value::as_f64);
    let mut label = s("Node Type").unwrap_or_else(|| "?".into());
    if (label.contains("Join") || label == "Nested Loop")
        && let Some(j) = s("Join Type").filter(|j| j != "Inner")
    {
        label = format!("{label} ({j})");
    }
    let object = s("Relation Name")
        .map(|r| match s("Alias") {
            Some(a) if a != r => format!("{r} {a}"),
            _ => r,
        })
        .or_else(|| s("Index Name"))
        .or_else(|| s("CTE Name"))
        .or_else(|| s("Function Name"));
    let skip = [
        "Node Type",
        "Plans",
        "Total Cost",
        "Plan Rows",
        "Actual Total Time",
        "Actual Rows",
        "Relation Name",
        "Alias",
        "Parallel Aware",
        "Async Capable",
        "Parent Relationship",
    ];
    let props = v
        .as_object()
        .map(|o| {
            o.iter()
                .filter(|(k, _)| !skip.contains(&k.as_str()))
                .filter_map(|(k, val)| {
                    let value = match val {
                        Value::String(s) => s.clone(),
                        Value::Number(n) => n.to_string(),
                        Value::Bool(b) => b.to_string(),
                        Value::Array(a) => a
                            .iter()
                            .map(|x| x.as_str().map(String::from).unwrap_or(x.to_string()))
                            .collect::<Vec<_>>()
                            .join(", "),
                        _ => return None,
                    };
                    Some(PlanProp {
                        key: k.clone(),
                        value,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let loops = f("Actual Loops").unwrap_or(1.0);
    PlanNode {
        label,
        object,
        cost: f("Total Cost"),
        rows: f("Plan Rows"),
        actual_rows: f("Actual Rows").map(|r| r * loops),
        actual_ms: f("Actual Total Time").map(|t| t * loops),
        props,
        children: v
            .get("Plans")
            .and_then(Value::as_array)
            .map(|c| c.iter().map(pg_node).collect())
            .unwrap_or_default(),
    }
}

// ----------------------------------------------------------- sql server --

fn mssql_roots(xml: &str) -> std::result::Result<Vec<PlanNode>, String> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| format!("showplan XML: {e}"))?;
    let roots = doc
        .descendants()
        .filter(|n| n.has_tag_name("StmtSimple"))
        .filter_map(|stmt| {
            let plan = stmt.descendants().find(|n| n.has_tag_name("QueryPlan"))?;
            let rel = plan.children().find(|n| n.has_tag_name("RelOp"))?;
            Some(relop(rel))
        })
        .collect();
    Ok(roots)
}

/// Child RelOps whose nearest RelOp ancestor is `node`.
fn child_relops<'a>(node: roxmltree::Node<'a, 'a>) -> Vec<roxmltree::Node<'a, 'a>> {
    node.descendants()
        .skip(1)
        .filter(|d| d.has_tag_name("RelOp"))
        .filter(|d| d.ancestors().skip(1).find(|a| a.has_tag_name("RelOp")) == Some(node))
        .collect()
}

fn relop(n: roxmltree::Node<'_, '_>) -> PlanNode {
    let attr = |k: &str| n.attribute(k).map(String::from);
    let num = |k: &str| n.attribute(k).and_then(|v| v.parse::<f64>().ok());
    let physical = attr("PhysicalOp").unwrap_or_else(|| "?".into());
    let logical = attr("LogicalOp").unwrap_or_default();
    let label = if logical.is_empty() || logical == physical {
        physical
    } else {
        format!("{physical} ({logical})")
    };
    // The first Object belonging to this operator (not to a nested RelOp).
    let object = n
        .descendants()
        .find(|d| {
            d.has_tag_name("Object")
                && d.ancestors().skip(1).find(|a| a.has_tag_name("RelOp")) == Some(n)
        })
        .map(|o| {
            let part = |k: &str| {
                o.attribute(k)
                    .map(|s| s.trim_matches(|c| c == '[' || c == ']').to_string())
            };
            let mut s = [part("Schema"), part("Table")]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(".");
            if let Some(i) = part("Index") {
                s.push_str(&format!(" · {i}"));
            }
            s
        });
    let actual_rows = n
        .children()
        .find(|c| c.has_tag_name("RunTimeInformation"))
        .map(|rti| {
            rti.children()
                .filter(|c| c.has_tag_name("RunTimeCountersPerThread"))
                .filter_map(|c| {
                    c.attribute("ActualRows")
                        .and_then(|v| v.parse::<f64>().ok())
                })
                .sum()
        });
    let props = [
        "EstimateIO",
        "EstimateCPU",
        "EstimatedExecutionMode",
        "Parallel",
        "EstimateRebinds",
    ]
    .iter()
    .filter_map(|k| {
        attr(k).map(|v| PlanProp {
            key: k.to_string(),
            value: v,
        })
    })
    .collect();
    PlanNode {
        label,
        object,
        cost: num("EstimatedTotalSubtreeCost"),
        rows: num("EstimateRows"),
        actual_rows,
        actual_ms: None,
        props,
        children: child_relops(n).into_iter().map(relop).collect(),
    }
}

// --------------------------------------------------------------- sqlite --

fn sqlite_tree(flat: &[(i64, i64, String)], parent: i64) -> Vec<PlanNode> {
    flat.iter()
        .filter(|(_, p, _)| *p == parent)
        .map(|(id, _, detail)| {
            let (label, object) = match detail.split_once(' ') {
                Some(("SCAN" | "SEARCH", rest)) => {
                    let table = rest.split_whitespace().next().map(String::from);
                    (detail.clone(), table)
                }
                _ => (detail.clone(), None),
            };
            PlanNode {
                label,
                object,
                cost: None,
                rows: None,
                actual_rows: None,
                actual_ms: None,
                props: Vec::new(),
                children: sqlite_tree(flat, *id),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn postgres_json_becomes_a_tree() {
        let json: Value = serde_json::from_str(
            r#"[{"Plan": {"Node Type": "Hash Join", "Join Type": "Left", "Total Cost": 10.5, "Plan Rows": 7,
                 "Plans": [{"Node Type": "Seq Scan", "Relation Name": "orders", "Alias": "o", "Total Cost": 4.0, "Plan Rows": 100},
                           {"Node Type": "Hash", "Total Cost": 3.0, "Plan Rows": 5}]}}]"#,
        )
        .unwrap();
        let root = pg_node(&json[0]["Plan"]);
        assert_eq!(root.label, "Hash Join (Left)");
        assert_eq!(root.cost, Some(10.5));
        assert_eq!(root.children.len(), 2);
        assert_eq!(root.children[0].object.as_deref(), Some("orders o"));
    }

    #[test]
    fn showplan_xml_becomes_a_tree() {
        let xml = r#"<ShowPlanXML xmlns="http://schemas.microsoft.com/sqlserver/2004/07/showplan"><BatchSequence><Batch><Statements>
          <StmtSimple StatementText="select"><QueryPlan>
            <RelOp PhysicalOp="Hash Match" LogicalOp="Inner Join" EstimateRows="10" EstimatedTotalSubtreeCost="0.5">
              <Hash>
                <RelOp PhysicalOp="Clustered Index Scan" LogicalOp="Clustered Index Scan" EstimateRows="100" EstimatedTotalSubtreeCost="0.2">
                  <IndexScan><Object Schema="[sales]" Table="[orders]" Index="[PK_orders]"/></IndexScan>
                </RelOp>
                <RelOp PhysicalOp="Table Scan" LogicalOp="Table Scan" EstimateRows="5" EstimatedTotalSubtreeCost="0.1">
                  <TableScan><Object Schema="[sales]" Table="[customers]"/></TableScan>
                </RelOp>
              </Hash>
            </RelOp>
          </QueryPlan></StmtSimple></Statements></Batch></BatchSequence></ShowPlanXML>"#;
        let roots = mssql_roots(xml).unwrap();
        assert_eq!(roots.len(), 1);
        let r = &roots[0];
        assert_eq!(r.label, "Hash Match (Inner Join)");
        assert_eq!(r.children.len(), 2);
        assert_eq!(
            r.children[0].object.as_deref(),
            Some("sales.orders · PK_orders")
        );
        assert!(r.object.is_none(), "nested objects belong to the children");
    }

    #[test]
    fn sqlite_rows_become_a_tree() {
        let flat = vec![
            (2, 0, "SCAN o".to_string()),
            (
                5,
                0,
                "SEARCH c USING INTEGER PRIMARY KEY (rowid=?)".to_string(),
            ),
            (7, 5, "child".to_string()),
        ];
        let t = sqlite_tree(&flat, 0);
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].object.as_deref(), Some("o"));
        assert_eq!(t[1].children[0].label, "child");
    }
}
