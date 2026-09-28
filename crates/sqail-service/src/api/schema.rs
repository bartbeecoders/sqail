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
use crate::engine::{DbError, Pooled, introspect};
use crate::error::{ApiError, ApiResult};
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

/// Map a catalog result; a lost connection is not returned to the pool.
fn finish<T>(c: &mut Pooled, r: crate::engine::Result<T>) -> ApiResult<Json<T>> {
    match r {
        Ok(v) => Ok(Json(v)),
        Err(e) => {
            if matches!(e, DbError::Connect(_)) {
                c.mark_broken();
            }
            Err(ApiError::from(e))
        }
    }
}

#[utoipa::path(get, path = "/v1/connections/{id}/schema/databases", tag = "schema",
    params(("id" = Uuid, Path)), responses((status = 200, body = Vec<NamedItem>)))]
pub async fn databases(
    State(s): State<AppState>,
    p: Principal,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Vec<NamedItem>>> {
    {
        let mut c = conn(&s, &p, id).await?;
        let r = introspect::databases(c.conn()).await;
        finish(&mut c, r)
    }
}

#[utoipa::path(get, path = "/v1/connections/{id}/schema/schemas", tag = "schema",
    params(("id" = Uuid, Path)), responses((status = 200, body = Vec<NamedItem>)))]
pub async fn schemas(
    State(s): State<AppState>,
    p: Principal,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Vec<NamedItem>>> {
    {
        let mut c = conn(&s, &p, id).await?;
        let r = introspect::schemas(c.conn()).await;
        finish(&mut c, r)
    }
}

#[utoipa::path(get, path = "/v1/connections/{id}/schema/tables", tag = "schema",
    params(("id" = Uuid, Path), SchemaFilter), responses((status = 200, body = Vec<TableInfo>)))]
pub async fn tables(
    State(s): State<AppState>,
    p: Principal,
    Path(id): Path<Uuid>,
    Query(q): Query<SchemaFilter>,
) -> ApiResult<Json<Vec<TableInfo>>> {
    {
        let mut c = conn(&s, &p, id).await?;
        let r = introspect::tables(c.conn(), q.schema.as_deref()).await;
        finish(&mut c, r)
    }
}

#[utoipa::path(get, path = "/v1/connections/{id}/schema/columns", tag = "schema",
    params(("id" = Uuid, Path), ObjectRef), responses((status = 200, body = Vec<ColumnInfo>)))]
pub async fn columns(
    State(s): State<AppState>,
    p: Principal,
    Path(id): Path<Uuid>,
    Query(q): Query<ObjectRef>,
) -> ApiResult<Json<Vec<ColumnInfo>>> {
    {
        let mut c = conn(&s, &p, id).await?;
        let r = introspect::columns(c.conn(), q.schema.as_deref(), &q.name).await;
        finish(&mut c, r)
    }
}

#[utoipa::path(get, path = "/v1/connections/{id}/schema/indexes", tag = "schema",
    params(("id" = Uuid, Path), ObjectRef), responses((status = 200, body = Vec<IndexInfo>)))]
pub async fn indexes(
    State(s): State<AppState>,
    p: Principal,
    Path(id): Path<Uuid>,
    Query(q): Query<ObjectRef>,
) -> ApiResult<Json<Vec<IndexInfo>>> {
    {
        let mut c = conn(&s, &p, id).await?;
        let r = introspect::indexes(c.conn(), q.schema.as_deref(), &q.name).await;
        finish(&mut c, r)
    }
}

#[utoipa::path(get, path = "/v1/connections/{id}/schema/foreign-keys", tag = "schema",
    params(("id" = Uuid, Path), ObjectRef), responses((status = 200, body = Vec<ForeignKeyInfo>)))]
pub async fn foreign_keys(
    State(s): State<AppState>,
    p: Principal,
    Path(id): Path<Uuid>,
    Query(q): Query<ObjectRef>,
) -> ApiResult<Json<Vec<ForeignKeyInfo>>> {
    {
        let mut c = conn(&s, &p, id).await?;
        let r = introspect::foreign_keys(c.conn(), q.schema.as_deref(), &q.name).await;
        finish(&mut c, r)
    }
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
    {
        let mut c = conn(&s, &p, id).await?;
        let r = introspect::privileges(c.conn(), q.schema.as_deref(), &q.name).await;
        finish(&mut c, r)
    }
}

#[utoipa::path(get, path = "/v1/connections/{id}/schema/routines", tag = "schema",
    params(("id" = Uuid, Path), SchemaFilter), responses((status = 200, body = Vec<RoutineInfo>)))]
pub async fn routines(
    State(s): State<AppState>,
    p: Principal,
    Path(id): Path<Uuid>,
    Query(q): Query<SchemaFilter>,
) -> ApiResult<Json<Vec<RoutineInfo>>> {
    {
        let mut c = conn(&s, &p, id).await?;
        let r = introspect::routines(c.conn(), q.schema.as_deref()).await;
        finish(&mut c, r)
    }
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
    {
        let mut c = conn(&s, &p, id).await?;
        let r = introspect::ddl(c.conn(), q.schema.as_deref(), &q.name).await;
        finish(&mut c, r)
    }
}
