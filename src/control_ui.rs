use crate::{
    agent::{self, Control},
    config::{self, AgentConfig, Mapping, Source},
    model::*,
    store, transfer,
};
use anyhow::{Result, ensure};
use axum::{
    Json, Router,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use rusqlite::params;
use serde_json::{Value, json};
use std::path::PathBuf;

type Error = (StatusCode, String);
fn error(e: anyhow::Error) -> Error {
    (StatusCode::BAD_REQUEST, e.to_string())
}
fn authorized(c: &Control, h: &HeaderMap) -> Result<(), Error> {
    if c.allowed(h) {
        Ok(())
    } else {
        Err((StatusCode::UNAUTHORIZED, "authentication required".into()))
    }
}
pub(crate) fn routes() -> Router<Control> {
    Router::new()
        .route("/info", get(info))
        .route("/files", get(files))
        .route(
            "/sources",
            get(sources).post(add_source).delete(remove_source),
        )
        .route("/maps", get(maps).delete(remove_map))
        .route("/relations", get(relations))
        .route("/deliveries", get(deliveries))
        .route("/sync", post(sync))
}

pub fn effective_sources(config: &AgentConfig) -> Result<Vec<Source>> {
    let db = store::open(&config.data_dir.join("state.db"))?;
    let mut sources = config.source.clone();
    let mut stmt = db.prepare("SELECT prefix,path FROM sources ORDER BY prefix")?;
    for row in stmt.query_map([], |r| {
        Ok(Source {
            prefix: r.get(0)?,
            path: PathBuf::from(r.get::<_, String>(1)?),
        })
    })? {
        let row = row?;
        if !sources.iter().any(|s| s.prefix == row.prefix) {
            sources.push(row)
        }
    }
    Ok(sources)
}
async fn info(State(c): State<Control>, h: HeaderMap) -> Result<Json<Value>, Error> {
    authorized(&c, &h)?;
    let cfg = c.config.lock().unwrap();
    Ok(Json(
        json!({"node_id":cfg.node_id,"hub":cfg.hub,"interval_seconds":cfg.interval_seconds,"data_dir":cfg.data_dir}),
    ))
}
async fn sync(State(c): State<Control>, h: HeaderMap) -> Result<StatusCode, Error> {
    authorized(&c, &h)?;
    c.notify.notify_one();
    Ok(StatusCode::NO_CONTENT)
}
async fn proxy(c: &Control, path: &str) -> Result<Value> {
    let cfg = c.config.lock().unwrap().clone();
    let response = agent::client(cfg.ca_cert.as_deref())?
        .get(format!("{}{path}", cfg.hub.trim_end_matches('/')))
        .bearer_auth(config::node_secret(&cfg)?)
        .send()
        .await?
        .error_for_status()?;
    Ok(response.json().await?)
}
async fn relations(State(c): State<Control>, h: HeaderMap) -> Result<Json<Value>, Error> {
    authorized(&c, &h)?;
    proxy(&c, "/v1/relations").await.map(Json).map_err(error)
}
async fn deliveries(
    State(c): State<Control>,
    h: HeaderMap,
    Query(q): Query<PageQuery>,
) -> Result<Json<Value>, Error> {
    authorized(&c, &h)?;
    let mut url = reqwest::Url::parse("http://localhost/v1/deliveries").unwrap();
    url.query_pairs_mut()
        .append_pair("offset", &q.offset.to_string())
        .append_pair("limit", &q.limit().to_string())
        .append_pair("search", &q.search);
    proxy(&c, &format!("{}?{}", url.path(), url.query().unwrap()))
        .await
        .map(Json)
        .map_err(error)
}
async fn sources(State(c): State<Control>, h: HeaderMap) -> Result<Json<Vec<Value>>, Error> {
    authorized(&c, &h)?;
    let cfg = c.config.lock().unwrap().clone();
    let sources = effective_sources(&cfg).map_err(error)?;
    Ok(Json(sources.iter().map(|s|json!({"prefix":s.prefix,"path":s.path,"origin":if cfg.source.iter().any(|v|v.prefix==s.prefix){"config"}else{"desktop"}})).collect()))
}
async fn add_source(
    State(c): State<Control>,
    h: HeaderMap,
    Json(source): Json<Source>,
) -> Result<StatusCode, Error> {
    authorized(&c, &h)?;
    let relations = proxy(&c, "/v1/relations").await.map_err(error)?;
    let result = (|| -> Result<()> {
        ensure!(
            relations["downstreams"]
                .as_array()
                .is_some_and(|a| !a.is_empty()),
            "请先在中心为本节点配置下游"
        );
        transfer::validate_relative(&source.prefix)?;
        ensure!(source.path.is_absolute(), "请选择绝对路径");
        let meta = std::fs::symlink_metadata(&source.path)?;
        ensure!(
            !meta.file_type().is_symlink() && (meta.is_dir() || meta.is_file()),
            "请选择普通文件或目录"
        );
        let cfg = c.config.lock().unwrap();
        ensure!(
            !cfg.source.iter().any(|s| s.prefix == source.prefix),
            "此发布名称由 TOML 配置管理，请修改配置文件"
        );
        let db = store::open(&c.db_path)?;
        let count: u64 = db.query_row("SELECT COUNT(*) FROM sources", [], |r| r.get(0))?;
        ensure!(count < 1000, "发布源数量已达上限");
        db.execute(
            "INSERT INTO sources(prefix,path) VALUES(?1,?2)",
            params![source.prefix, source.path.to_string_lossy()],
        )?;
        Ok(())
    })();
    result.map_err(error)?;
    c.notify.notify_one();
    Ok(StatusCode::NO_CONTENT)
}
async fn remove_source(
    State(c): State<Control>,
    h: HeaderMap,
    Json(source): Json<Source>,
) -> Result<StatusCode, Error> {
    authorized(&c, &h)?;
    (|| -> Result<()> {
        ensure!(
            !c.config
                .lock()
                .unwrap()
                .source
                .iter()
                .any(|s| s.prefix == source.prefix),
            "此发布源由 TOML 配置管理"
        );
        store::open(&c.db_path)?.execute("DELETE FROM sources WHERE prefix=?1", [source.prefix])?;
        Ok(())
    })()
    .map_err(error)?;
    c.notify.notify_one();
    Ok(StatusCode::NO_CONTENT)
}
async fn maps(State(c): State<Control>, h: HeaderMap) -> Result<Json<Vec<Value>>, Error> {
    authorized(&c, &h)?;
    (|| -> Result<_> {
        let cfg = c.config.lock().unwrap();
        let mut items: Vec<Value> = cfg
            .map
            .iter()
            .map(|m| json!({"upstream":m.upstream,"from":m.from,"to":m.to,"origin":"config"}))
            .collect();
        let db = store::open(&c.db_path)?;
        let mut stmt =
            db.prepare("SELECT upstream,source,target FROM claims ORDER BY upstream,source")?;
        for row in stmt.query_map([], |r| {
            Ok(Mapping {
                upstream: r.get(0)?,
                from: r.get(1)?,
                to: PathBuf::from(r.get::<_, String>(2)?),
            })
        })? {
            let m = row?;
            if !cfg
                .map
                .iter()
                .any(|v| v.upstream == m.upstream && v.from == m.from)
            {
                items.push(
                    json!({"upstream":m.upstream,"from":m.from,"to":m.to,"origin":"desktop"}),
                );
            }
        }
        Ok(Json(items))
    })()
    .map_err(error)
}
async fn remove_map(
    State(c): State<Control>,
    h: HeaderMap,
    Json(m): Json<Mapping>,
) -> Result<StatusCode, Error> {
    authorized(&c, &h)?;
    (|| -> Result<()> {
        ensure!(
            !c.config
                .lock()
                .unwrap()
                .map
                .iter()
                .any(|v| v.upstream == m.upstream && v.from == m.from),
            "此映射由 TOML 配置管理"
        );
        store::open(&c.db_path)?.execute(
            "DELETE FROM claims WHERE upstream=?1 AND source=?2",
            params![m.upstream, m.from],
        )?;
        Ok(())
    })()
    .map_err(error)?;
    c.notify.notify_one();
    Ok(StatusCode::NO_CONTENT)
}
async fn files(
    State(c): State<Control>,
    h: HeaderMap,
    Query(q): Query<PageQuery>,
) -> Result<Json<Page<Pending>>, Error> {
    authorized(&c, &h)?;
    (|| -> Result<_> {
        let db=store::open(&c.db_path)?;
        let condition=" FROM local_files WHERE (?1='' OR state=?1 OR (?1='attention' AND state IN ('pending','conflict','error'))) AND instr(lower(path),lower(?2))>0";
        let total=db.query_row(&format!("SELECT COUNT(*){condition}"),params![q.state,q.search],|r|r.get(0))?;
        let mut stmt=db.prepare(&format!("SELECT upstream,path,sha,size,target,state,error{condition} ORDER BY upstream,path LIMIT ?3 OFFSET ?4"))?;
        let items=stmt.query_map(params![q.state,q.search,q.limit(),q.offset],|r|Ok(Pending{upstream:r.get(0)?,path:r.get(1)?,sha256:r.get(2)?,size:r.get(3)?,target:r.get(4)?,state:r.get(5)?,error:r.get(6)?}))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(Json(Page{items,total}))
    })().map_err(error)
}
