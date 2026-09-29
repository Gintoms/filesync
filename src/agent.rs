use crate::{
    config::{self, AgentConfig, Mapping},
    model::*,
    store, transfer,
};
use anyhow::{Context, Result, ensure};
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use futures_util::TryStreamExt;
use rusqlite::{OptionalExtension, params};
use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio_util::io::{ReaderStream, StreamReader};

pub fn client(ca: Option<&Path>) -> Result<reqwest::Client> {
    crate::init_crypto();
    let mut builder = reqwest::Client::builder()
        .no_proxy()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(300))
        .pool_max_idle_per_host(2)
        .http1_only()
        .redirect(reqwest::redirect::Policy::none());
    if let Some(path) = ca {
        builder =
            builder.add_root_certificate(reqwest::Certificate::from_pem(&std::fs::read(path)?)?);
    }
    Ok(builder.build()?)
}

fn endpoint(config: &AgentConfig, path: &str) -> String {
    format!("{}{path}", config.hub.trim_end_matches('/'))
}

pub async fn sync_once(config: &AgentConfig) -> Result<SyncSummary> {
    config.validate()?;
    let token = config::node_secret(config)?;
    let client = client(config.ca_cert.as_deref())?;
    let relations: Relations = client
        .get(endpoint(config, "/v1/relations"))
        .bearer_auth(&token)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(
        relations.node_id == config.node_id,
        "node_id does not match the authenticated node"
    );
    ensure!(
        relations.upstreams.is_empty() || relations.downstreams.is_empty(),
        "invalid two-tier topology"
    );
    let db_path = config.data_dir.join("state.db");
    let mut summary = SyncSummary::default();
    if !relations.downstreams.is_empty() {
        let sources = crate::control_ui::effective_sources(config)?;
        let path = db_path.clone();
        summary.scanned = tokio::task::spawn_blocking(move || {
            crate::scan::scan(&mut store::open(&path)?, &sources)
        })
        .await??;
        let db = store::open(&db_path)?;
        let manifest = tempfile::NamedTempFile::new_in(&config.data_dir)?;
        let mut output = manifest.reopen()?;
        // SQLite groups identical content on disk, avoiding a growing in-memory set
        // and avoiding repeated uploads for many paths with identical content.
        db.execute("CREATE TEMP TABLE upload_queue AS SELECT sha,size,MIN(local_path) AS local_path FROM scan_queue GROUP BY sha,size", [])?;
        let mut cursor = String::new();
        loop {
            let next: Option<(String, String)> = db
                .query_row(
                    "SELECT sha,local_path FROM upload_queue WHERE sha>?1 ORDER BY sha LIMIT 1",
                    [&cursor],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            let Some((sha, local)) = next else {
                break;
            };
            let blob_url = endpoint(config, &format!("/v1/blobs/{sha}"));
            let exists = client.head(&blob_url).bearer_auth(&token).send().await?;
            if exists.status() == reqwest::StatusCode::NOT_FOUND {
                let file = tokio::fs::File::open(local).await?;
                client
                    .put(&blob_url)
                    .bearer_auth(&token)
                    .body(reqwest::Body::wrap_stream(ReaderStream::with_capacity(
                        file,
                        transfer::BUFFER_SIZE,
                    )))
                    .send()
                    .await?
                    .error_for_status()?;
                summary.published += 1;
            } else {
                exists.error_for_status()?;
            }
            cursor = sha;
        }
        {
            let mut stmt = db.prepare("SELECT path,sha,size FROM scan_queue ORDER BY path")?;
            let mut rows = stmt.query([])?;
            while let Some(row) = rows.next()? {
                let entry = FileEntry {
                    path: row.get(0)?,
                    sha256: row.get(1)?,
                    size: row.get(2)?,
                };
                serde_json::to_writer(&mut output, &ManifestEvent::File(entry))?;
                writeln!(output)?;
            }
        }
        serde_json::to_writer(&mut output, &ManifestEvent::Complete)?;
        writeln!(output)?;
        output.sync_all()?;
        client
            .put(endpoint(config, "/v1/manifest"))
            .bearer_auth(&token)
            .body(reqwest::Body::wrap_stream(ReaderStream::new(
                tokio::fs::File::from_std(manifest.reopen()?),
            )))
            .send()
            .await?
            .error_for_status()?;
    }
    let db = store::open(&db_path)?;
    let mut maps = config.map.clone();
    {
        let mut stmt = db.prepare("SELECT upstream,source,target FROM claims")?;
        for row in stmt.query_map([], |r| {
            Ok(Mapping {
                upstream: r.get(0)?,
                from: r.get(1)?,
                to: PathBuf::from(r.get::<_, String>(2)?),
            })
        })? {
            let row = row?;
            if !maps
                .iter()
                .any(|m| m.upstream == row.upstream && m.from == row.from)
            {
                maps.push(row);
            }
        }
    }
    let epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_micros() as i64;
    for upstream in &relations.upstreams {
        let response = client
            .get(endpoint(config, &format!("/v1/manifest/{}", upstream.id)))
            .bearer_auth(&token)
            .send()
            .await?
            .error_for_status()?;
        let stream = response.bytes_stream().map_err(std::io::Error::other);
        let mut reader = BufReader::new(StreamReader::new(stream));
        let mut line = Vec::with_capacity(1024);
        let mut complete = false;
        loop {
            line.clear();
            let n = (&mut reader)
                .take(16 * 1024)
                .read_until(b'\n', &mut line)
                .await?;
            if n == 0 {
                break;
            }
            ensure!(n < 16 * 1024 && !complete, "invalid manifest framing");
            match serde_json::from_slice::<ManifestEvent>(&line)? {
                ManifestEvent::Complete => complete = true,
                ManifestEvent::File(entry) => {
                    transfer::validate_relative(&entry.path)?;
                    let target = transfer::mapped_target(&maps, &upstream.id, &entry.path)?;
                    let target = target
                        .as_ref()
                        .map(|p| -> Result<String> {
                            transfer::prepare_target(p)?;
                            let canonical = p
                                .parent()
                                .context("target has no parent")?
                                .canonicalize()?
                                .join(p.file_name().context("target has no name")?);
                            Ok(canonical.to_string_lossy().into_owned())
                        })
                        .transpose()?;
                    db.execute("INSERT INTO local_files(upstream,path,sha,size,target,state,seen) VALUES(?1,?2,?3,?4,?5,?6,?7)
                        ON CONFLICT(upstream,path) DO UPDATE SET sha=excluded.sha,size=excluded.size,target=excluded.target,state=excluded.state,error=NULL,seen=excluded.seen", params![upstream.id,entry.path,entry.sha256,entry.size,target,if target.is_some(){"queued"}else{"pending"},epoch])?;
                }
            }
        }
        ensure!(
            complete,
            "incomplete manifest; do not mark missing files obsolete"
        );
        db.execute(
            "UPDATE local_files SET state='obsolete' WHERE upstream=?1 AND seen<>?2",
            params![upstream.id, epoch],
        )?;
    }
    // Only paths collected in this successful round may be downloaded. Detect
    // all collisions before writing either source into the destination.
    db.execute("UPDATE local_files SET state='conflict', error='multiple sources map to this target' WHERE seen=?1 AND target COLLATE NOCASE IN (SELECT target FROM local_files WHERE seen=?1 AND target IS NOT NULL GROUP BY target COLLATE NOCASE HAVING COUNT(*)>1)", [epoch])?;
    let mut after = (String::new(), String::new());
    loop {
        let next: Option<(String,FileEntry,String)> = db.query_row("SELECT upstream,path,sha,size,target FROM local_files WHERE seen=?1 AND state='queued' AND (upstream,path)>(?2,?3) ORDER BY upstream,path LIMIT 1", params![epoch,after.0,after.1], |r| Ok((r.get(0)?,FileEntry{path:r.get(1)?,sha256:r.get(2)?,size:r.get(3)?},r.get(4)?))).optional()?;
        let Some((upstream, entry, target)) = next else {
            break;
        };
        after = (upstream.clone(), entry.path.clone());
        let target_path = PathBuf::from(&target);
        transfer::prepare_target(&target_path)?;
        let existing = if target_path.is_file() {
            let p = target_path.clone();
            Some(tokio::task::spawn_blocking(move || transfer::hash_file(&p)).await??)
        } else {
            None
        };
        let result = if existing.as_deref() == Some(&entry.sha256) {
            summary.unchanged += 1;
            Ok(())
        } else {
            transfer::download(
                &client,
                &endpoint(config, &format!("/v1/content/{upstream}/{}", entry.sha256)),
                &token,
                &entry,
                &target_path,
            )
            .await
            .map(|()| {
                summary.downloaded += 1;
            })
        };
        match result {
            Ok(()) => {
                db.execute("UPDATE local_files SET state='ok',applied_sha=?3,error=NULL WHERE upstream=?1 AND path=?2", params![upstream,entry.path,entry.sha256])?;
                // Always resend on subsequent rounds: a lost receipt never forces a re-download.
                if let Err(error) = async {
                    client
                        .post(endpoint(config, "/v1/receipts"))
                        .bearer_auth(&token)
                        .json(&Receipt {
                            upstream: upstream.clone(),
                            path: entry.path.clone(),
                            sha256: entry.sha256,
                        })
                        .send()
                        .await?
                        .error_for_status()
                }
                .await
                {
                    tracing::warn!(%error, "receipt will be retried");
                }
            }
            Err(error) => {
                summary.failed += 1;
                db.execute(
                    "UPDATE local_files SET state='error',error=?3 WHERE upstream=?1 AND path=?2",
                    params![upstream, entry.path, error.to_string()],
                )?;
            }
        }
    }
    summary.pending = db.query_row(
        "SELECT COUNT(*) FROM local_files WHERE seen=?1 AND state='pending'",
        [epoch],
        |r| r.get(0),
    )?;
    let conflicts: u64 = db.query_row(
        "SELECT COUNT(*) FROM local_files WHERE seen=?1 AND state='conflict'",
        [epoch],
        |r| r.get(0),
    )?;
    summary.failed += conflicts;
    Ok(summary)
}

#[derive(Clone)]
pub(crate) struct Control {
    pub(crate) db_path: PathBuf,
    token_hash: String,
    pub(crate) notify: Arc<tokio::sync::Notify>,
    pub(crate) status: Arc<Mutex<serde_json::Value>>,
    pub(crate) config: Arc<Mutex<AgentConfig>>,
}

impl Control {
    pub(crate) fn allowed(&self, headers: &HeaderMap) -> bool {
        headers
            .get("authorization")
            .and_then(|h| h.to_str().ok())
            .and_then(|h| h.strip_prefix("Bearer "))
            .is_some_and(|token| transfer::token_hash(token) == self.token_hash)
    }
}

async fn status(
    State(c): State<Control>,
    h: HeaderMap,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !c.allowed(&h) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let mut value = c.status.lock().unwrap().clone();
    value["control_version"] = 2.into();
    value["node_id"] = c.config.lock().unwrap().node_id.clone().into();
    Ok(Json(value))
}

async fn pending(State(c): State<Control>, h: HeaderMap) -> Result<Json<Vec<Pending>>, StatusCode> {
    if !c.allowed(&h) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let result = (|| -> Result<_> {
        let db = store::open(&c.db_path)?;
        let mut stmt=db.prepare("SELECT upstream,path,sha,size,target,state,error FROM local_files WHERE state IN ('pending','conflict','error') ORDER BY upstream,path LIMIT 100")?;
        Ok(stmt
            .query_map([], |r| {
                Ok(Pending {
                    upstream: r.get(0)?,
                    path: r.get(1)?,
                    sha256: r.get(2)?,
                    size: r.get(3)?,
                    target: r.get(4)?,
                    state: r.get(5)?,
                    error: r.get(6)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    })();
    result
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

async fn claim(
    State(c): State<Control>,
    h: HeaderMap,
    Json(mapping): Json<Mapping>,
) -> Result<StatusCode, StatusCode> {
    if !c.allowed(&h) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    if !mapping.to.is_absolute()
        || mapping.upstream.is_empty()
        || transfer::validate_relative(mapping.from.trim_end_matches('/')).is_err()
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    let result = (|| -> Result<()> {
        let config = c.config.lock().unwrap();
        ensure!(
            !config
                .map
                .iter()
                .any(|m| m.upstream == mapping.upstream && m.from == mapping.from),
            "mapping is managed by TOML"
        );
        let db = store::open(&c.db_path)?;
        db.execute(
            "INSERT OR REPLACE INTO claims VALUES(?1,?2,?3)",
            params![mapping.upstream, mapping.from, mapping.to.to_string_lossy()],
        )?;
        Ok(())
    })();
    result.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    c.notify.notify_one();
    Ok(StatusCode::NO_CONTENT)
}

pub fn control_url(config: &AgentConfig) -> Result<String> {
    let address: std::net::SocketAddr =
        std::fs::read_to_string(config.data_dir.join("control.addr"))?
            .trim()
            .parse()?;
    ensure!(address.ip().is_loopback(), "invalid local control address");
    Ok(format!("http://{address}"))
}

pub async fn run(path: &Path, once: bool) -> Result<()> {
    let mut config: AgentConfig = config::read(path)?;
    config.validate()?;
    std::fs::create_dir_all(&config.data_dir)?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(config.data_dir.join("agent.lock"))?;
    fs2::FileExt::try_lock_exclusive(&lock)
        .context("an agent is already running in this data directory")?;
    if once {
        println!("{}", serde_json::to_string(&sync_once(&config).await?)?);
        return Ok(());
    }
    store::open(&config.data_dir.join("state.db"))?;
    let notify = Arc::new(tokio::sync::Notify::new());
    let status_value = Arc::new(Mutex::new(serde_json::json!({"state":"starting"})));
    let live_config = Arc::new(Mutex::new(config.clone()));
    let control = Control {
        db_path: config.data_dir.join("state.db"),
        token_hash: transfer::token_hash(&config::node_secret(&config)?),
        notify: notify.clone(),
        status: status_value.clone(),
        config: live_config.clone(),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address_path = config.data_dir.join("control.addr");
    std::fs::write(&address_path, listener.local_addr()?.to_string())?;
    let app = Router::new()
        .merge(crate::control_ui::routes())
        .route("/status", get(status))
        .route("/pending", get(pending))
        .route("/claim", post(claim))
        .with_state(control);
    let server = tokio::spawn(async move { axum::serve(listener, app).await });
    let shutdown = tokio::signal::ctrl_c();
    tokio::pin!(shutdown);
    loop {
        *status_value.lock().unwrap() = serde_json::json!({"state":"syncing"});
        match sync_once(&config).await {
            Ok(summary) => {
                *status_value.lock().unwrap() =
                    serde_json::json!({"state":"running","summary":summary})
            }
            Err(error) => {
                tracing::error!(%error,"sync failed; will retry");
                *status_value.lock().unwrap() =
                    serde_json::json!({"state":"error","error":error.to_string()});
            }
        }
        tokio::select! {
            _ = &mut shutdown => break,
            _ = notify.notified() => {},
            _ = tokio::time::sleep(Duration::from_secs(config.interval_seconds)) => {},
        }
        match config::read::<AgentConfig>(path).and_then(|new| {
            new.validate()?;
            ensure!(
                new.data_dir == config.data_dir
                    && new.node_id == config.node_id
                    && new.token_env == config.token_env,
                "changing agent identity/data_dir requires restart"
            );
            Ok(new)
        }) {
            Ok(new) => {
                config = new;
                *live_config.lock().unwrap() = config.clone();
            }
            Err(error) => {
                tracing::error!(%error,"invalid config; retaining last valid configuration")
            }
        }
    }
    server.abort();
    let _ = std::fs::remove_file(address_path);
    Ok(())
}
