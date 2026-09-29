use crate::{
    hub::{ApiResult, Hub},
    model::*,
};
use anyhow::Result;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
};
use rusqlite::params;
use serde_json::{Value, json};

mod embedded {
    include!(concat!(env!("OUT_DIR"), "/web_assets.rs"));
}

fn asset(name: &str) -> Response {
    if let Some((mime, bytes)) = embedded::asset(name) {
        ([
            ("content-type", mime), ("cache-control", "no-store"),
            ("x-content-type-options", "nosniff"),
            ("content-security-policy", "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'"),
            ("referrer-policy", "no-referrer"),
        ], bytes).into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            "Web UI assets are missing. Build ui first, then rebuild filesync.",
        )
            .into_response()
    }
}

pub(crate) fn routes() -> Router<Hub> {
    Router::new()
        .route("/", get(|| async { asset("index.html") }))
        .route(
            "/assets/{name}",
            get(|Path(name): Path<String>| async move { asset(&format!("assets/{name}")) }),
        )
        .route("/v1/admin/nodes", get(nodes))
        .route(
            "/v1/admin/nodes/{id}",
            axum::routing::patch(rename).delete(remove_node),
        )
        .route("/v1/admin/edges", get(edges).delete(remove_edge))
        .route("/v1/admin/deliveries", get(admin_deliveries))
        .route("/v1/deliveries", get(node_deliveries))
        .route("/v1/me", get(me))
}

async fn me(State(hub): State<Hub>, h: HeaderMap) -> ApiResult<Json<Node>> {
    let id = hub.node(&h)?;
    let result = hub
        .db
        .lock()
        .unwrap()
        .query_row("SELECT id,name FROM nodes WHERE id=?1", [id], |r| {
            Ok(Node {
                id: r.get(0)?,
                name: r.get(1)?,
            })
        })
        .map_err(anyhow::Error::from)?;
    Ok(Json(result))
}

async fn nodes(State(hub): State<Hub>, h: HeaderMap) -> ApiResult<Json<Vec<Value>>> {
    hub.admin(&h)?;
    (|| -> Result<_> {
        let db = hub.db.lock().unwrap();
        let mut stmt = db.prepare("SELECT n.id,n.name,p.last_seen, EXISTS(SELECT 1 FROM edges WHERE upstream=n.id), EXISTS(SELECT 1 FROM edges WHERE downstream=n.id) FROM nodes n LEFT JOIN presence p ON p.node=n.id ORDER BY n.name,n.id")?;
        Ok(Json(stmt.query_map([], |r| Ok(json!({"id":r.get::<_,String>(0)?, "name":r.get::<_,String>(1)?, "last_seen":r.get::<_,Option<i64>>(2)?, "publisher":r.get::<_,bool>(3)?, "subscriber":r.get::<_,bool>(4)?})))?.collect::<rusqlite::Result<Vec<_>>>()?))
    })().map_err(Into::into)
}

async fn rename(
    State(hub): State<Hub>,
    h: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<NewNode>,
) -> ApiResult<StatusCode> {
    hub.admin(&h)?;
    if input.name.trim().is_empty() || input.name.len() > 128 {
        return Err(crate::hub::ApiError(anyhow::anyhow!("invalid node name")));
    }
    let n = hub
        .db
        .lock()
        .unwrap()
        .execute(
            "UPDATE nodes SET name=?1 WHERE id=?2",
            params![input.name.trim(), id],
        )
        .map_err(anyhow::Error::from)?;
    Ok(if n == 0 {
        StatusCode::NOT_FOUND
    } else {
        StatusCode::NO_CONTENT
    })
}

async fn edges(State(hub): State<Hub>, h: HeaderMap) -> ApiResult<Json<Vec<Edge>>> {
    hub.admin(&h)?;
    (|| -> Result<_> {
        let db = hub.db.lock().unwrap();
        let mut stmt =
            db.prepare("SELECT upstream,downstream FROM edges ORDER BY upstream,downstream")?;
        Ok(Json(
            stmt.query_map([], |r| {
                Ok(Edge {
                    upstream: r.get(0)?,
                    downstream: r.get(1)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?,
        ))
    })()
    .map_err(Into::into)
}

async fn remove_node(
    State(hub): State<Hub>,
    h: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    hub.admin(&h)?;
    (|| -> Result<StatusCode> {
        let mut db = hub.db.lock().unwrap();
        let tx = db.transaction()?;
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM nodes WHERE id=?1)",
            [&id],
            |r| r.get(0),
        )?;
        if !exists {
            return Ok(StatusCode::NOT_FOUND);
        }
        tx.execute(
            "DELETE FROM edges WHERE upstream=?1 OR downstream=?1",
            [&id],
        )?;
        tx.execute("DELETE FROM receipts WHERE node=?1 OR upstream=?1", [&id])?;
        tx.execute("DELETE FROM files WHERE node=?1", [&id])?;
        tx.execute("DELETE FROM presence WHERE node=?1", [&id])?;
        tx.execute("DELETE FROM nodes WHERE id=?1", [&id])?;
        tx.commit()?;
        Ok(StatusCode::NO_CONTENT)
    })()
    .map_err(Into::into)
}

async fn remove_edge(
    State(hub): State<Hub>,
    h: HeaderMap,
    Json(edge): Json<Edge>,
) -> ApiResult<StatusCode> {
    hub.admin(&h)?;
    hub.db
        .lock()
        .unwrap()
        .execute(
            "DELETE FROM edges WHERE upstream=?1 AND downstream=?2",
            params![edge.upstream, edge.downstream],
        )
        .map_err(anyhow::Error::from)?;
    Ok(StatusCode::NO_CONTENT)
}

fn deliveries(hub: &Hub, upstream: &str, q: &PageQuery) -> Result<Page<Value>> {
    let db = hub.db.lock().unwrap();
    let condition = " FROM files f JOIN edges e ON e.upstream=f.node JOIN nodes u ON u.id=f.node JOIN nodes d ON d.id=e.downstream LEFT JOIN receipts r ON r.node=e.downstream AND r.upstream=f.node AND r.path=f.path WHERE (?1='' OR f.node=?1) AND instr(lower(f.path),lower(?2))>0";
    let total = db.query_row(
        &format!("SELECT COUNT(*){condition}"),
        params![upstream, q.search],
        |r| r.get(0),
    )?;
    let mut stmt = db.prepare(&format!("SELECT u.name,d.name,f.node,e.downstream,f.path,f.size,COALESCE(r.sha=f.sha,0){condition} ORDER BY f.node,f.path,e.downstream LIMIT ?3 OFFSET ?4"))?;
    let items = stmt.query_map(params![upstream,q.search,q.limit(),q.offset], |r| Ok(json!({"upstream_name":r.get::<_,String>(0)?, "downstream_name":r.get::<_,String>(1)?, "upstream":r.get::<_,String>(2)?, "downstream":r.get::<_,String>(3)?, "path":r.get::<_,String>(4)?, "size":r.get::<_,u64>(5)?, "received":r.get::<_,bool>(6)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(Page { items, total })
}
async fn admin_deliveries(
    State(hub): State<Hub>,
    h: HeaderMap,
    Query(q): Query<PageQuery>,
) -> ApiResult<Json<Page<Value>>> {
    hub.admin(&h)?;
    Ok(Json(deliveries(&hub, "", &q)?))
}
async fn node_deliveries(
    State(hub): State<Hub>,
    h: HeaderMap,
    Query(q): Query<PageQuery>,
) -> ApiResult<Json<Page<Value>>> {
    let node = hub.node(&h)?;
    Ok(Json(deliveries(&hub, &node, &q)?))
}
