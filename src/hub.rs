use crate::{
    config::{self, HubConfig},
    model::*,
    store, transfer,
};
use anyhow::{Context, Result, ensure};
use axum::{
    Json, Router,
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use futures_util::StreamExt;
use rusqlite::{Connection, OptionalExtension, params};
use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Read, Write},
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tokio_util::io::ReaderStream;

#[cfg(test)]
#[path = "hub_retention_tests.rs"]
mod retention_tests;

#[derive(Clone)]
pub struct Hub {
    pub(crate) db: Arc<Mutex<Connection>>,
    pub(crate) data_dir: PathBuf,
    admin_hash: String,
    pub(crate) active_reads: Arc<Mutex<HashMap<String, usize>>>,
    _storage_lock: Arc<std::fs::File>,
}

pub(crate) struct ApiError(pub(crate) anyhow::Error);
#[derive(Debug)]
struct AuthError(&'static str);
impl std::fmt::Display for AuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for AuthError {}
pub(crate) type ApiResult<T> = std::result::Result<T, ApiError>;
impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        Self(e)
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        tracing::warn!(error = %self.0, "request rejected");
        (
            if self.0.downcast_ref::<AuthError>().is_some() {
                StatusCode::UNAUTHORIZED
            } else {
                StatusCode::BAD_REQUEST
            },
            Json(serde_json::json!({"error": self.0.to_string()})),
        )
            .into_response()
    }
}

impl Hub {
    pub fn new(config: &HubConfig, admin: &str) -> Result<Self> {
        std::fs::create_dir_all(config.data_dir.join("blobs"))?;
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(config.data_dir.join("hub.lock"))?;
        fs2::FileExt::try_lock_exclusive(&lock).context("hub data directory is already in use")?;
        let hub = Self {
            db: Arc::new(Mutex::new(store::open(&config.data_dir.join("hub.db"))?)),
            data_dir: config.data_dir.clone(),
            admin_hash: transfer::token_hash(admin),
            active_reads: Arc::new(Mutex::new(HashMap::new())),
            _storage_lock: Arc::new(lock),
        };
        hub.init_retention()?;
        Ok(hub)
    }
    fn auth(&self, headers: &HeaderMap) -> Result<String> {
        let token = headers
            .get("authorization")
            .and_then(|h| h.to_str().ok())
            .and_then(|h| h.strip_prefix("Bearer "))
            .ok_or(AuthError("missing bearer token"))?;
        Ok(transfer::token_hash(token))
    }
    pub(crate) fn admin(&self, headers: &HeaderMap) -> Result<()> {
        ensure!(
            self.auth(headers)? == self.admin_hash,
            AuthError("admin authentication failed")
        );
        Ok(())
    }
    pub(crate) fn node(&self, headers: &HeaderMap) -> Result<String> {
        let hash = self.auth(headers)?;
        self.db
            .lock()
            .unwrap()
            .query_row("SELECT id FROM nodes WHERE token_hash=?1", [hash], |r| {
                r.get(0)
            })
            .optional()?
            .ok_or_else(|| AuthError("node authentication failed").into())
    }
    fn publisher(&self, headers: &HeaderMap) -> Result<String> {
        let node = self.node(headers)?;
        let count: u64 = self.db.lock().unwrap().query_row(
            "SELECT COUNT(*) FROM edges WHERE upstream=?1",
            [&node],
            |r| r.get(0),
        )?;
        ensure!(count > 0, "node is not a publisher");
        Ok(node)
    }
    fn subscriber(&self, headers: &HeaderMap, upstream: &str) -> Result<String> {
        let node = self.node(headers)?;
        let count: u64 = self.db.lock().unwrap().query_row(
            "SELECT COUNT(*) FROM edges WHERE upstream=?1 AND downstream=?2",
            params![upstream, node],
            |r| r.get(0),
        )?;
        ensure!(count == 1, "upstream access denied");
        Ok(node)
    }
}

pub fn router(hub: Hub) -> Router {
    Router::new()
        .merge(crate::hub_ui::routes())
        .route(
            "/health",
            get(|| async { Json(serde_json::json!({"status":"ok"})) }),
        )
        .route("/v1/nodes", post(new_node))
        .route("/v1/edges", post(add_edge))
        .route("/v1/relations", get(relations))
        .route("/v1/blobs/{sha}", put(upload).head(blob_exists))
        .route("/v1/manifest", put(publish))
        .route("/v1/manifest/{node}", get(manifest))
        .route("/v1/content/{upstream}/{sha}", get(content))
        .route("/v1/receipts", post(receipt))
        .with_state(hub)
}

async fn new_node(
    State(hub): State<Hub>,
    headers: HeaderMap,
    Json(input): Json<NewNode>,
) -> ApiResult<Json<NodeCredential>> {
    hub.admin(&headers)?;
    let result = (|| -> Result<_> {
        ensure!(
            !input.name.trim().is_empty() && input.name.len() <= 128,
            "invalid node name"
        );
        let credential = NodeCredential {
            id: uuid::Uuid::new_v4().to_string(),
            token: format!(
                "{}{}",
                uuid::Uuid::new_v4().simple(),
                uuid::Uuid::new_v4().simple()
            ),
        };
        hub.db.lock().unwrap().execute(
            "INSERT INTO nodes VALUES(?1,?2,?3)",
            params![
                credential.id,
                input.name,
                transfer::token_hash(&credential.token)
            ],
        )?;
        Ok(Json(credential))
    })();
    result.map_err(Into::into)
}

pub fn insert_edge(db: &mut Connection, edge: &Edge) -> Result<()> {
    let tx = db.transaction()?;
    ensure!(
        edge.upstream != edge.downstream,
        "a node cannot subscribe to itself"
    );
    let count: u64 = tx.query_row(
        "SELECT COUNT(*) FROM edges WHERE downstream=?1 OR upstream=?2",
        params![edge.upstream, edge.downstream],
        |r| r.get(0),
    )?;
    ensure!(
        count == 0,
        "only two tiers are allowed; a subscriber cannot publish"
    );
    tx.execute(
        "INSERT OR IGNORE INTO edges VALUES(?1,?2)",
        params![edge.upstream, edge.downstream],
    )?;
    tx.commit()?;
    Ok(())
}

async fn add_edge(
    State(hub): State<Hub>,
    headers: HeaderMap,
    Json(edge): Json<Edge>,
) -> ApiResult<StatusCode> {
    hub.admin(&headers)?;
    insert_edge(&mut hub.db.lock().unwrap(), &edge)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn relations(State(hub): State<Hub>, headers: HeaderMap) -> ApiResult<Json<Relations>> {
    let node = hub.node(&headers)?;
    (|| -> Result<_> {
        let db = hub.db.lock().unwrap();
        db.execute("INSERT INTO presence(node,last_seen) VALUES(?1,unixepoch()) ON CONFLICT(node) DO UPDATE SET last_seen=excluded.last_seen", [&node])?;
        let read = |sql: &str| -> Result<Vec<Node>> {
            Ok(db.prepare(sql)?.query_map([&node], |r| Ok(Node { id: r.get(0)?, name: r.get(1)? }))?.collect::<rusqlite::Result<_>>()?)
        };
        Ok(Json(Relations {
            node_id: node.clone(),
            upstreams: read("SELECT n.id,n.name FROM nodes n JOIN edges e ON n.id=e.upstream WHERE e.downstream=?1")?,
            downstreams: read("SELECT n.id,n.name FROM nodes n JOIN edges e ON n.id=e.downstream WHERE e.upstream=?1")?,
        }))
    })().map_err(Into::into)
}

pub(crate) fn check_sha(sha: &str) -> Result<()> {
    ensure!(
        sha.len() == 64
            && sha
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
        "invalid sha256"
    );
    Ok(())
}

async fn receive(hub: &Hub, body: Body, limit: u64) -> Result<tempfile::NamedTempFile> {
    use tokio::io::AsyncWriteExt;
    let temp = tempfile::NamedTempFile::new_in(&hub.data_dir)?;
    let mut file = tokio::fs::File::from_std(temp.reopen()?);
    let mut stream = body.into_data_stream();
    let mut size = 0u64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        size += chunk.len() as u64;
        ensure!(size <= limit, "request exceeds size limit");
        file.write_all(&chunk).await?;
    }
    file.sync_all().await?;
    drop(file);
    Ok(temp)
}

async fn upload(
    State(hub): State<Hub>,
    Path(sha): Path<String>,
    headers: HeaderMap,
    body: Body,
) -> ApiResult<StatusCode> {
    let node = hub.publisher(&headers)?;
    check_sha(&sha)?;
    let temp = receive(&hub, body, transfer::MAX_FILE_SIZE - 1).await?;
    let path = hub.data_dir.join("blobs").join(&sha);
    tokio::task::spawn_blocking(move || -> Result<()> {
        ensure!(
            transfer::hash_file(temp.path())? == sha,
            "upload hash mismatch"
        );
        let mut db = hub.db.lock().unwrap();
        let tx = db.transaction()?;
        // Serialize installation with GC; keep uploaded-but-unpublished data leased.
        tx.execute("INSERT INTO hub_uploads(node,sha,expires) VALUES(?1,?2,unixepoch()+86400) ON CONFLICT(node,sha) DO UPDATE SET expires=excluded.expires", params![node, sha])?;
        if !path.exists() {
            temp.persist(path)?;
        }
        tx.commit()?;
        Ok(())
    })
    .await
    .context("upload worker failed")??;
    Ok(StatusCode::NO_CONTENT)
}

async fn blob_exists(
    State(hub): State<Hub>,
    Path(sha): Path<String>,
    headers: HeaderMap,
) -> ApiResult<StatusCode> {
    let node = hub.publisher(&headers)?;
    check_sha(&sha)?;
    let db = hub.db.lock().unwrap();
    let exists = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM files WHERE node=?1 AND sha=?2)",
            params![node, sha],
            |r| r.get::<_, bool>(0),
        )
        .map_err(anyhow::Error::from)?;
    Ok(
        if exists && hub.data_dir.join("blobs").join(&sha).is_file() {
            db.execute("INSERT INTO hub_uploads(node,sha,expires) VALUES(?1,?2,unixepoch()+86400) ON CONFLICT(node,sha) DO UPDATE SET expires=excluded.expires", params![node, sha]).map_err(anyhow::Error::from)?;
            StatusCode::NO_CONTENT
        } else {
            StatusCode::NOT_FOUND
        },
    )
}

async fn publish(State(hub): State<Hub>, headers: HeaderMap, body: Body) -> ApiResult<StatusCode> {
    let node = hub.publisher(&headers)?;
    let temp = receive(&hub, body, 64 * 1024 * 1024).await?;
    tokio::task::spawn_blocking(move || -> Result<()> {
        let mut db = hub.db.lock().unwrap();
        let tx = db.transaction()?;
        tx.execute_batch("CREATE TEMP TABLE IF NOT EXISTS incoming_files(path TEXT PRIMARY KEY, sha TEXT NOT NULL, size INTEGER NOT NULL); DELETE FROM incoming_files;")?;
        let mut input = BufReader::new(temp.reopen()?);
        let mut line = Vec::with_capacity(1024);
        let mut complete = false;
        loop {
            line.clear();
            let n = std::io::Read::by_ref(&mut input)
                .take(16 * 1024)
                .read_until(b'\n', &mut line)?;
            if n == 0 {
                break;
            }
            ensure!(n < 16 * 1024 && !complete, "invalid manifest framing");
            match serde_json::from_slice::<ManifestEvent>(&line)? {
                ManifestEvent::Complete => complete = true,
                ManifestEvent::File(entry) => {
                    transfer::validate_relative(&entry.path)?;
                    check_sha(&entry.sha256)?;
                    let size =
                        std::fs::metadata(hub.data_dir.join("blobs").join(&entry.sha256))?.len();
                    ensure!(
                        entry.size == size && size < transfer::MAX_FILE_SIZE,
                        "blob size mismatch"
                    );
                    tx.execute(
                        "INSERT INTO incoming_files VALUES(?1,?2,?3)",
                        params![entry.path, entry.sha256, entry.size],
                    )?;
                }
            }
        }
        ensure!(
            complete,
            "incomplete manifest: current publication retained"
        );
        tx.execute("INSERT INTO hub_previous(node,path,sha,size) SELECT f.node,f.path,f.sha,f.size FROM files f JOIN incoming_files i ON i.path=f.path WHERE f.node=?1 AND f.sha<>i.sha ON CONFLICT(node,path) DO UPDATE SET sha=excluded.sha,size=excluded.size", [&node])?;
        tx.execute("DELETE FROM hub_previous WHERE node=?1 AND path NOT IN (SELECT path FROM incoming_files)", [&node])?;
        tx.execute("DELETE FROM files WHERE node=?1", [&node])?;
        tx.execute("INSERT INTO files SELECT ?1,path,sha,size FROM incoming_files", [&node])?;
        tx.execute("DELETE FROM hub_uploads WHERE node=?1 AND sha IN (SELECT sha FROM incoming_files)", [&node])?;
        tx.commit()?;
        drop(db);
        hub.cleanup_blobs();
        Ok(())
    })
    .await
    .context("publish worker failed")??;
    Ok(StatusCode::NO_CONTENT)
}

async fn manifest(
    State(hub): State<Hub>,
    Path(node): Path<String>,
    headers: HeaderMap,
) -> ApiResult<Body> {
    hub.subscriber(&headers, &node)?;
    // Generate a consistent SQLite snapshot on disk, then stream it. No whole
    // manifest in the resident heap; temporary file is removed when body drops.
    let temp = tokio::task::spawn_blocking(move || -> Result<_> {
        let mut temp = tempfile::NamedTempFile::new_in(&hub.data_dir)?;
        let db = hub.db.lock().unwrap();
        let mut stmt = db.prepare("SELECT path,sha,size FROM files WHERE node=?1 ORDER BY path")?;
        let mut rows = stmt.query([node])?;
        while let Some(row) = rows.next()? {
            let entry = FileEntry {
                path: row.get(0)?,
                sha256: row.get(1)?,
                size: row.get(2)?,
            };
            serde_json::to_writer(&mut temp, &ManifestEvent::File(entry))?;
            writeln!(temp)?;
        }
        serde_json::to_writer(&mut temp, &ManifestEvent::Complete)?;
        writeln!(temp)?;
        Ok(temp)
    })
    .await
    .context("manifest worker failed")??;
    let file = tokio::fs::File::from_std(temp.reopen().map_err(anyhow::Error::from)?);
    let stream = ReaderStream::with_capacity(file, transfer::BUFFER_SIZE).map(move |chunk| {
        let _keep_alive = &temp;
        chunk
    });
    Ok(Body::from_stream(stream))
}

async fn content(
    State(hub): State<Hub>,
    Path((upstream, sha)): Path<(String, String)>,
    headers: HeaderMap,
) -> ApiResult<Body> {
    hub.subscriber(&headers, &upstream)?;
    check_sha(&sha)?;
    let (file, pin) = (|| -> Result<_> {
        let db = hub.db.lock().unwrap();
        let exists: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM files WHERE node=?1 AND sha=?2)",
            params![upstream, sha],
            |r| r.get(0),
        )?;
        ensure!(
            exists,
            "content is not currently published by this upstream"
        );
        let file = std::fs::File::open(hub.data_dir.join("blobs").join(&sha))?;
        let pin = hub.pin_blob(&sha);
        Ok((file, pin))
    })()?;
    Ok(Body::from_stream(
        ReaderStream::with_capacity(tokio::fs::File::from_std(file), transfer::BUFFER_SIZE).map(
            move |chunk| {
                let _keep_alive = &pin;
                chunk
            },
        ),
    ))
}

async fn receipt(
    State(hub): State<Hub>,
    headers: HeaderMap,
    Json(input): Json<Receipt>,
) -> ApiResult<StatusCode> {
    let node = hub.subscriber(&headers, &input.upstream)?;
    (|| -> Result<_> {
        let db = hub.db.lock().unwrap();
        let current: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM files WHERE node=?1 AND path=?2 AND sha=?3)",
            params![input.upstream, input.path, input.sha256],
            |r| r.get(0),
        )?;
        ensure!(current, "receipt is for an obsolete version");
        db.execute(
            "INSERT OR REPLACE INTO receipts VALUES(?1,?2,?3,?4)",
            params![node, input.upstream, input.path, input.sha256],
        )?;
        Ok(StatusCode::NO_CONTENT)
    })()
    .map_err(Into::into)
}

pub async fn serve(config: HubConfig) -> Result<()> {
    config.validate_transport()?;
    crate::init_crypto();
    let token = config::secret(&config.admin_token_env)?;
    let hub = Hub::new(&config, &token)?;
    let collector_hub = hub.clone();
    let collector = tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            tick.tick().await;
            let hub = collector_hub.clone();
            if let Err(error) = tokio::task::spawn_blocking(move || hub.cleanup_blobs()).await {
                tracing::warn!(%error, "blob cleanup worker failed");
            }
        }
    });
    let app = router(hub);
    tracing::info!(listen = %config.listen, "hub starting");
    match (config.tls_cert, config.tls_key) {
        (Some(cert), Some(key)) => {
            let tls = axum_server::tls_rustls::RustlsConfig::from_pem_file(cert, key).await?;
            let handle = axum_server::Handle::new();
            let shutdown = handle.clone();
            tokio::spawn(async move {
                let _ = tokio::signal::ctrl_c().await;
                shutdown.graceful_shutdown(Some(std::time::Duration::from_secs(5)));
            });
            axum_server::bind_rustls(config.listen, tls)
                .handle(handle)
                .serve(app.into_make_service())
                .await?;
        }
        _ => {
            if !config.listen.ip().is_loopback() {
                tracing::warn!(
                    "HTTP enabled: tokens and file contents are sent without encryption"
                );
            }
            axum::serve(tokio::net::TcpListener::bind(config.listen).await?, app)
                .with_graceful_shutdown(async {
                    let _ = tokio::signal::ctrl_c().await;
                })
                .await?;
        }
    }
    collector.abort();
    Ok(())
}
