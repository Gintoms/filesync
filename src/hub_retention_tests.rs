use super::*;
use sha2::{Digest, Sha256};

#[tokio::test]
async fn upgrade_preserves_current_files_and_legacy_grace_does_not_reset() -> Result<()> {
    let dir = tempfile::tempdir()?;
    std::fs::create_dir(dir.path().join("blobs"))?;
    let current = format!("{:x}", Sha256::digest(b"current"));
    let orphan = format!("{:x}", Sha256::digest(b"orphan"));
    std::fs::write(dir.path().join("blobs").join(&current), b"current")?;
    std::fs::write(dir.path().join("blobs").join(&orphan), b"orphan")?;
    let db = store::open(&dir.path().join("hub.db"))?;
    db.execute("INSERT INTO nodes VALUES('a','a','token')", [])?;
    db.execute("INSERT INTO files VALUES('a','f',?1,7)", [&current])?;
    drop(db);
    let cfg = HubConfig {
        listen: "127.0.0.1:0".parse()?,
        data_dir: dir.path().into(),
        admin_token_env: "unused".into(),
        tls_cert: None,
        tls_key: None,
        allow_http: false,
    };
    let hub = Hub::new(&cfg, "admin")?;
    hub.collect_blobs()?;
    assert!(exists(&hub, &current) && exists(&hub, &orphan));
    assert_eq!(revisions(&hub, "a", "f")?, (current.clone(), None));
    hub.db
        .lock()
        .unwrap()
        .execute("UPDATE hub_legacy_blobs SET expires=0", [])?;
    drop(hub);
    let hub = Hub::new(&cfg, "admin")?;
    hub.collect_blobs()?;
    assert!(exists(&hub, &current) && !exists(&hub, &orphan));
    Ok(())
}

#[tokio::test]
async fn garbage_collection_during_upload_does_not_remove_the_new_object() -> Result<()> {
    let (_dir, hub) = setup()?;
    let bytes = axum::body::Bytes::from_static(b"slow upload");
    let sha = format!("{:x}", Sha256::digest(&bytes));
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let (resume_tx, resume_rx) = tokio::sync::oneshot::channel();
    let body = Body::from_stream(futures_util::stream::once(async move {
        let _ = ready_tx.send(());
        let _ = resume_rx.await;
        Ok::<_, std::io::Error>(bytes)
    }));
    let worker_hub = hub.clone();
    let worker_sha = sha.clone();
    let worker = tokio::spawn(async move {
        upload(State(worker_hub), Path(worker_sha), headers("a"), body)
            .await
            .map_err(|e| e.0)
    });
    ready_rx.await?;
    hub.collect_blobs()?;
    assert!(!exists(&hub, &sha));
    resume_tx.send(()).unwrap();
    worker.await??;
    hub.collect_blobs()?;
    assert!(exists(&hub, &sha));
    commit(&hub, "a", &[("f", &sha)]).await?;
    Ok(())
}

fn setup() -> Result<(tempfile::TempDir, Hub)> {
    let dir = tempfile::tempdir()?;
    let config = HubConfig {
        listen: "127.0.0.1:0".parse()?,
        data_dir: dir.path().into(),
        admin_token_env: "unused".into(),
        tls_cert: None,
        tls_key: None,
        allow_http: false,
    };
    let hub = Hub::new(&config, "admin")?;
    {
        let db = hub.db.lock().unwrap();
        for id in ["a", "b", "receiver"] {
            db.execute(
                "INSERT INTO nodes VALUES(?1,?1,?2)",
                params![id, transfer::token_hash(id)],
            )?;
        }
        for id in ["a", "b"] {
            db.execute("INSERT INTO edges VALUES(?1,'receiver')", [id])?;
        }
    }
    Ok((dir, hub))
}
fn headers(id: &str) -> HeaderMap {
    let mut h = HeaderMap::new();
    h.insert("authorization", format!("Bearer {id}").parse().unwrap());
    h
}
async fn put(hub: &Hub, node: &str, bytes: &str) -> Result<String> {
    let sha = format!("{:x}", Sha256::digest(bytes.as_bytes()));
    upload(
        State(hub.clone()),
        Path(sha.clone()),
        headers(node),
        Body::from(bytes.to_owned()),
    )
    .await
    .map_err(|e| e.0)?;
    Ok(sha)
}
async fn commit(hub: &Hub, node: &str, files: &[(&str, &str)]) -> Result<()> {
    let mut body = Vec::new();
    for (path, sha) in files {
        let size = std::fs::metadata(hub.data_dir.join("blobs").join(sha))?.len();
        serde_json::to_writer(
            &mut body,
            &ManifestEvent::File(FileEntry {
                path: (*path).into(),
                sha256: (*sha).into(),
                size,
            }),
        )?;
        body.push(b'\n');
    }
    serde_json::to_writer(&mut body, &ManifestEvent::Complete)?;
    body.push(b'\n');
    publish(State(hub.clone()), headers(node), Body::from(body))
        .await
        .map_err(|e| e.0)?;
    Ok(())
}
fn exists(hub: &Hub, sha: &str) -> bool {
    hub.data_dir.join("blobs").join(sha).is_file()
}
fn revisions(hub: &Hub, node: &str, path: &str) -> Result<(String, Option<String>)> {
    Ok(hub.db.lock().unwrap().query_row("SELECT f.sha,p.sha FROM files f LEFT JOIN hub_previous p ON p.node=f.node AND p.path=f.path WHERE f.node=?1 AND f.path=?2", params![node,path], |r| Ok((r.get(0)?,r.get(1)?)))?)
}

#[tokio::test]
async fn retains_two_versions_noop_does_not_rotate_and_invalid_manifest_rolls_back() -> Result<()> {
    let (_dir, hub) = setup()?;
    let a = put(&hub, "a", "version one").await?;
    commit(&hub, "a", &[("f", &a)]).await?;
    let b = put(&hub, "a", "version two").await?;
    commit(&hub, "a", &[("f", &b)]).await?;
    assert_eq!(revisions(&hub, "a", "f")?, (b.clone(), Some(a.clone())));
    commit(&hub, "a", &[("f", &b)]).await?;
    assert_eq!(revisions(&hub, "a", "f")?, (b.clone(), Some(a.clone())));
    // Missing complete marker must not rotate history or release content.
    assert!(
        publish(State(hub.clone()), headers("a"), Body::from(""))
            .await
            .is_err()
    );
    assert_eq!(revisions(&hub, "a", "f")?, (b.clone(), Some(a.clone())));
    let c = put(&hub, "a", "version three").await?;
    commit(&hub, "a", &[("f", &c)]).await?;
    assert!(!exists(&hub, &a));
    assert!(exists(&hub, &b) && exists(&hub, &c));
    // Returning to previous content swaps the two revisions, preserving order.
    commit(&hub, "a", &[("f", &b)]).await?;
    assert_eq!(revisions(&hub, "a", "f")?, (b, Some(c)));
    Ok(())
}

#[tokio::test]
async fn shared_paths_publishers_and_unfinished_uploads_protect_content() -> Result<()> {
    let (_dir, hub) = setup()?;
    let a = put(&hub, "a", "shared").await?;
    commit(&hub, "a", &[("f", &a), ("other", &a)]).await?;
    // Another publisher uploads the same object but has not committed its manifest.
    put(&hub, "b", "shared").await?;
    let b = put(&hub, "a", "two").await?;
    commit(&hub, "a", &[("f", &b), ("other", &a)]).await?;
    let c = put(&hub, "a", "three").await?;
    commit(&hub, "a", &[("f", &c), ("other", &a)]).await?;
    assert!(exists(&hub, &a));
    commit(&hub, "a", &[("f", &c)]).await?;
    assert!(exists(&hub, &a)); // B's upload lease still protects the content.
    commit(&hub, "b", &[("f", &a)]).await?;
    commit(&hub, "a", &[]).await?;
    assert!(exists(&hub, &a));
    assert!(!exists(&hub, &b) && !exists(&hub, &c));
    commit(&hub, "b", &[]).await?;
    assert!(!exists(&hub, &a));
    Ok(())
}

#[tokio::test]
async fn in_flight_download_survives_retirement_until_body_is_dropped() -> Result<()> {
    let (_dir, hub) = setup()?;
    let a = put(&hub, "a", "download in progress").await?;
    commit(&hub, "a", &[("f", &a)]).await?;
    let body = content(
        State(hub.clone()),
        Path(("a".into(), a.clone())),
        headers("receiver"),
    )
    .await
    .map_err(|e| e.0)?;
    for value in ["two", "three"] {
        let sha = put(&hub, "a", value).await?;
        commit(&hub, "a", &[("f", &sha)]).await?;
    }
    assert!(exists(&hub, &a));
    assert!(
        content(
            State(hub.clone()),
            Path(("a".into(), a.clone())),
            headers("receiver")
        )
        .await
        .is_err()
    );
    assert_eq!(
        axum::body::to_bytes(body, 1024).await?.as_ref(),
        b"download in progress"
    );
    hub.collect_blobs()?;
    assert!(!exists(&hub, &a));
    Ok(())
}

#[tokio::test]
async fn failed_uploads_and_expired_leases_do_not_accumulate() -> Result<()> {
    let (_dir, hub) = setup()?;
    let invalid = "a".repeat(64);
    assert!(
        upload(
            State(hub.clone()),
            Path(invalid.clone()),
            headers("a"),
            Body::from("wrong hash")
        )
        .await
        .is_err()
    );
    assert!(!exists(&hub, &invalid));
    let sha = put(&hub, "a", "abandoned upload").await?;
    hub.collect_blobs()?;
    assert!(exists(&hub, &sha));
    hub.db
        .lock()
        .unwrap()
        .execute("UPDATE hub_uploads SET expires=0", [])?;
    hub.collect_blobs()?;
    assert!(!exists(&hub, &sha));
    Ok(())
}

#[tokio::test]
async fn node_deletion_cascades_history_and_preserves_other_nodes() -> Result<()> {
    let (_dir, hub) = setup()?;
    let a = put(&hub, "a", "shared").await?;
    commit(&hub, "a", &[("f", &a)]).await?;
    put(&hub, "b", "shared").await?;
    commit(&hub, "b", &[("f", &a)]).await?;
    let b = put(&hub, "a", "new").await?;
    commit(&hub, "a", &[("f", &b)]).await?;
    let orphan = put(&hub, "a", "uncommitted").await?;
    let db = hub.db.lock().unwrap();
    db.execute("DELETE FROM edges WHERE upstream='a'", [])?;
    db.execute("DELETE FROM files WHERE node='a'", [])?;
    db.execute("DELETE FROM nodes WHERE id='a'", [])?;
    drop(db);
    hub.collect_blobs()?;
    assert!(exists(&hub, &a));
    assert!(!exists(&hub, &b) && !exists(&hub, &orphan));
    Ok(())
}

#[tokio::test]
async fn history_survives_restart_and_data_directory_is_exclusive() -> Result<()> {
    let (dir, hub) = setup()?;
    let a = put(&hub, "a", "one").await?;
    commit(&hub, "a", &[("f", &a)]).await?;
    let b = put(&hub, "a", "two").await?;
    commit(&hub, "a", &[("f", &b)]).await?;
    let cfg = HubConfig {
        listen: "127.0.0.1:0".parse()?,
        data_dir: dir.path().into(),
        admin_token_env: "unused".into(),
        tls_cert: None,
        tls_key: None,
        allow_http: false,
    };
    assert!(Hub::new(&cfg, "admin").is_err());
    drop(hub);
    let hub = Hub::new(&cfg, "admin")?;
    assert_eq!(revisions(&hub, "a", "f")?, (b, Some(a.clone())));
    let c = put(&hub, "a", "three").await?;
    commit(&hub, "a", &[("f", &c)]).await?;
    assert!(!exists(&hub, &a));
    Ok(())
}
