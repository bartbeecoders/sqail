//! Catalog browsing. Each call borrows a pooled connection.

use axum::Json;
use axum::extract::{Path, Query, State};
use serde::Deserialize;
use sqail_proto::{
    ColumnInfo, Ddl, ForeignKeyInfo, IndexInfo, NamedItem, RoutineInfo, TableInfo, TablePrivileges,
};
use utoipa::IntoParams;
use uuid::Uuid;

use super::pool_for;
use crate::auth::Principal;
use crate::engine::{DbError, Pooled, introspect, with_deadline};
use crate::error::ApiResult;
use crate::state::AppState;

#[derive(Debug, Deserialize, IntoParams)]
pub struct SchemaFilter {
    /// Limit to one schema.
    schema: Option<String>,
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct ObjectRef {
    /// Defaults to the connection's current schema.
    schema: Option<String>,
    /// Table, view or routine name.
    name: String,
}

async fn conn(state: &AppState, p: &Principal, id: Uuid) -> ApiResult<Pooled> {
    // Catalog reads are allowed for `read` tokens on every profile.
    let read_view = Principal {
        scope: sqail_proto::Scope::Query.max(p.scope),
        ..p.clone()
    };
    let (_, pool) = pool_for(state, &read_view, id)?;
    Ok(pool.get().await?)
}

/// Map a catalog result and hand the connection back. Catalog SQL leaves no
/// session state, so it goes back as it is unless it was lost or cut off.
fn finish<T>(c: Pooled, (r, broken): (crate::engine::Result<T>, bool)) -> ApiResult<Json<T>> {
    if broken || matches!(r, Err(DbError::Connect(_) | DbError::Timeout)) {
        c.discard();
    } else {
        c.release();
    }
    Ok(Json(r?))
}

/// Borrow a pooled connection, run one catalog call on it (`$c`) within the
/// call deadline, and hand the connection back.
macro_rules! catalog {
    ($s:expr, $p:expr, $id:expr, |$c:ident| $call:block) => {{
        let mut pooled = conn(&$s, &$p, $id).await?;
        let (limit, grace) = $s.config.limits.call_deadline();
        let canceller = pooled.conn().canceller();
        let r = {
            let $c = pooled.conn();
            with_deadline(canceller, limit, grace, $call).await
        };
        finish(pooled, r)
    }};
}

#[utoipa::path(get, path = "/v1/connections/{id}/schema/databases", tag = "schema",
    params(("id" = Uuid, Path)), responses((status = 200, body = Vec<NamedItem>)))]
pub async fn databases(
    State(s): State<AppState>,
    p: Principal,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Vec<NamedItem>>> {
    catalog!(s, p, id, |c| { introspect::databases(c) })
}

#[utoipa::path(get, path = "/v1/connections/{id}/schema/schemas", tag = "schema",
    params(("id" = Uuid, Path)), responses((status = 200, body = Vec<NamedItem>)))]
pub async fn schemas(
    State(s): State<AppState>,
    p: Principal,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Vec<NamedItem>>> {
    catalog!(s, p, id, |c| { introspect::schemas(c) })
}

#[utoipa::path(get, path = "/v1/connections/{id}/schema/tables", tag = "schema",
    params(("id" = Uuid, Path), SchemaFilter), responses((status = 200, body = Vec<TableInfo>)))]
pub async fn tables(
    State(s): State<AppState>,
    p: Principal,
    Path(id): Path<Uuid>,
    Query(q): Query<SchemaFilter>,
) -> ApiResult<Json<Vec<TableInfo>>> {
    catalog!(s, p, id, |c| { introspect::tables(c, q.schema.as_deref()) })
}

#[utoipa::path(get, path = "/v1/connections/{id}/schema/columns", tag = "schema",
    params(("id" = Uuid, Path), ObjectRef), responses((status = 200, body = Vec<ColumnInfo>)))]
pub async fn columns(
    State(s): State<AppState>,
    p: Principal,
    Path(id): Path<Uuid>,
    Query(q): Query<ObjectRef>,
) -> ApiResult<Json<Vec<ColumnInfo>>> {
    catalog!(s, p, id, |c| {
        introspect::columns(c, q.schema.as_deref(), &q.name)
    })
}

#[utoipa::path(get, path = "/v1/connections/{id}/schema/indexes", tag = "schema",
    params(("id" = Uuid, Path), ObjectRef), responses((status = 200, body = Vec<IndexInfo>)))]
pub async fn indexes(
    State(s): State<AppState>,
    p: Principal,
    Path(id): Path<Uuid>,
    Query(q): Query<ObjectRef>,
) -> ApiResult<Json<Vec<IndexInfo>>> {
    catalog!(s, p, id, |c| {
        introspect::indexes(c, q.schema.as_deref(), &q.name)
    })
}

#[utoipa::path(get, path = "/v1/connections/{id}/schema/foreign-keys", tag = "schema",
    params(("id" = Uuid, Path), ObjectRef), responses((status = 200, body = Vec<ForeignKeyInfo>)))]
pub async fn foreign_keys(
    State(s): State<AppState>,
    p: Principal,
    Path(id): Path<Uuid>,
    Query(q): Query<ObjectRef>,
) -> ApiResult<Json<Vec<ForeignKeyInfo>>> {
    catalog!(s, p, id, |c| {
        introspect::foreign_keys(c, q.schema.as_deref(), &q.name)
    })
}

/// Grants on a table, and the roles and users that could receive them.
#[utoipa::path(get, path = "/v1/connections/{id}/schema/privileges", tag = "schema",
    params(("id" = Uuid, Path), ObjectRef), responses((status = 200, body = TablePrivileges)))]
pub async fn privileges(
    State(s): State<AppState>,
    p: Principal,
    Path(id): Path<Uuid>,
    Query(q): Query<ObjectRef>,
) -> ApiResult<Json<TablePrivileges>> {
    catalog!(s, p, id, |c| {
        introspect::privileges(c, q.schema.as_deref(), &q.name)
    })
}

#[utoipa::path(get, path = "/v1/connections/{id}/schema/routines", tag = "schema",
    params(("id" = Uuid, Path), SchemaFilter), responses((status = 200, body = Vec<RoutineInfo>)))]
pub async fn routines(
    State(s): State<AppState>,
    p: Principal,
    Path(id): Path<Uuid>,
    Query(q): Query<SchemaFilter>,
) -> ApiResult<Json<Vec<RoutineInfo>>> {
    catalog!(s, p, id, |c| {
        introspect::routines(c, q.schema.as_deref())
    })
}

/// Best-effort CREATE script for a table, view or routine.
#[utoipa::path(get, path = "/v1/connections/{id}/ddl", tag = "schema",
    params(("id" = Uuid, Path), ObjectRef), responses((status = 200, body = Ddl)))]
pub async fn ddl(
    State(s): State<AppState>,
    p: Principal,
    Path(id): Path<Uuid>,
    Query(q): Query<ObjectRef>,
) -> ApiResult<Json<Ddl>> {
    catalog!(s, p, id, |c| {
        introspect::ddl(c, q.schema.as_deref(), &q.name)
    })
}
