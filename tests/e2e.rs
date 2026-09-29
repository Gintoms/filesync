use anyhow::{Result, ensure};
use filesync::{
    agent,
    config::HubConfig,
    hub,
    model::{Edge, NewNode, NodeCredential, Pending, SyncSummary},
    store,
};
use std::{
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::Duration,
};

struct Background(Child);
impl Drop for Background {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn quoted(path: &Path) -> String {
    serde_json::to_string(&path.to_string_lossy()).unwrap()
}

fn write_config(
    path: &Path,
    hub: &str,
    ca: &Path,
    node: &str,
    data: &Path,
    extra: &str,
) -> Result<()> {
    std::fs::write(
        path,
        format!(
            "hub = {hub:?}\nnode_id = {node:?}\ntoken_env = \"FILESYNC_TEST_NODE_TOKEN\"\ndata_dir = {}\nca_cert = {}\n{extra}",
            quoted(data),
            quoted(ca)
        ),
    )?;
    Ok(())
}

async fn once(path: PathBuf, token: String) -> Result<SyncSummary> {
    tokio::task::spawn_blocking(move || -> Result<_> {
        let result = Command::new(env!("CARGO_BIN_EXE_filesync"))
            .arg("--config")
            .arg(path)
            .args(["agent", "--once"])
            .env("FILESYNC_TEST_NODE_TOKEN", token)
            .output()?;
        ensure!(
            result.status.success(),
            "agent failed: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        Ok(serde_json::from_slice(&result.stdout)?)
    })
    .await?
}

#[tokio::test]
async fn https_publish_claim_latest_backup_delete_and_conflict() -> Result<()> {
    filesync::init_crypto();
    let temp = tempfile::tempdir()?;
    let canonical_root = temp.path().canonicalize()?;
    let root = canonical_root.as_path();
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".into(), "127.0.0.1".into()])?;
    let cert_path = root.join("ca.pem");
    std::fs::write(&cert_path, cert.cert.pem())?;
    let tls = axum_server::tls_rustls::RustlsConfig::from_pem(
        cert.cert.pem().into_bytes(),
        cert.signing_key.serialize_pem().into_bytes(),
    )
    .await?;
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let addr = listener.local_addr()?;
    let url = format!("https://{addr}");
    let admin = "a-test-only-secret-with-more-than-32-characters";
    let hub_config = HubConfig {
        listen: addr,
        data_dir: root.join("hub"),
        admin_token_env: "unused".into(),
        tls_cert: None,
        tls_key: None,
        allow_http: false,
    };
    let app = hub::router(hub::Hub::new(&hub_config, admin)?);
    let server =
        tokio::spawn(axum_server::from_tcp_rustls(listener, tls).serve(app.into_make_service()));
    let client = agent::client(Some(&cert_path))?;
    let new_node = |name: &str| {
        client
            .post(format!("{url}/v1/nodes"))
            .bearer_auth(admin)
            .json(&NewNode { name: name.into() })
    };
    let a: NodeCredential = new_node("upstream-A")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let b: NodeCredential = new_node("downstream-B")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let c: NodeCredential = new_node("upstream-C")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    for upstream in [&a.id, &c.id] {
        client
            .post(format!("{url}/v1/edges"))
            .bearer_auth(admin)
            .json(&Edge {
                upstream: upstream.clone(),
                downstream: b.id.clone(),
            })
            .send()
            .await?
            .error_for_status()?;
    }
    let invalid = client
        .post(format!("{url}/v1/edges"))
        .bearer_auth(admin)
        .json(&Edge {
            upstream: b.id.clone(),
            downstream: c.id.clone(),
        })
        .send()
        .await?;
    assert!(!invalid.status().is_success());
    let source = root.join("source");
    std::fs::create_dir(&source)?;
    let file = source.join("quote.txt");
    std::fs::write(&file, "v0")?;
    let a_config = root.join("a.toml");
    let b_config = root.join("b.toml");
    let c_config = root.join("c.toml");
    let sources = format!(
        "[[source]]\npath = {}\nprefix = \"quotes\"\n",
        quoted(&source)
    );
    write_config(
        &a_config,
        &url,
        &cert_path,
        &a.id,
        &root.join("a"),
        &sources,
    )?;
    write_config(&b_config, &url, &cert_path, &b.id, &root.join("b"), "")?;
    write_config(
        &c_config,
        &url,
        &cert_path,
        &c.id,
        &root.join("c"),
        &sources,
    )?;
    assert_eq!(once(a_config.clone(), a.token.clone()).await?.published, 1);
    assert_eq!(
        once(a_config.clone(), a.token.clone()).await?.published,
        0,
        "unchanged content should not upload again"
    );
    let unclaimed = once(b_config.clone(), b.token.clone()).await?;
    assert_eq!(unclaimed.pending, 1);
    assert_eq!(unclaimed.downloaded, 0);
    let destination = root.join("received");
    assert!(!destination.exists());
    // Exercise the real daemon/control API used by the TUI, not only --once.
    {
        let mut daemon = Background(
            Command::new(env!("CARGO_BIN_EXE_filesync"))
                .arg("--config")
                .arg(&b_config)
                .arg("agent")
                .env("FILESYNC_TEST_NODE_TOKEN", &b.token)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()?,
        );
        let control_path = root.join("b/control.addr");
        let mut control = None;
        for _ in 0..100 {
            if let Ok(address) = std::fs::read_to_string(&control_path) {
                let local = format!("http://{}", address.trim());
                if let Ok(response) = client
                    .get(format!("{local}/pending"))
                    .bearer_auth(&b.token)
                    .send()
                    .await
                    && response.status().is_success()
                {
                    control = Some(local);
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let control = control.expect("local agent API did not start");
        assert_eq!(
            client
                .get(format!("{control}/status"))
                .send()
                .await?
                .status(),
            reqwest::StatusCode::UNAUTHORIZED
        );
        let pending: Vec<Pending> = client
            .get(format!("{control}/pending"))
            .bearer_auth(&b.token)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        assert_eq!(pending.len(), 1);
        client
            .post(format!("{control}/claim"))
            .bearer_auth(&b.token)
            .json(&filesync::config::Mapping {
                upstream: a.id.clone(),
                from: "quotes/".into(),
                to: destination.clone(),
            })
            .send()
            .await?
            .error_for_status()?;
        for _ in 0..100 {
            if destination.join("quote.txt").exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert_eq!(
            std::fs::read_to_string(destination.join("quote.txt"))?,
            "v0"
        );
        assert!(daemon.0.try_wait()?.is_none(), "agent exited after claim");
    }
    let mappings = format!(
        "[[map]]\nupstream = {:?}\nfrom = \"quotes/\"\nto = {}\n",
        a.id,
        quoted(&destination)
    );
    write_config(
        &b_config,
        &url,
        &cert_path,
        &b.id,
        &root.join("b"),
        &mappings,
    )?;
    assert_eq!(once(b_config.clone(), b.token.clone()).await?.unchanged, 1);
    assert_eq!(
        std::fs::read_to_string(destination.join("quote.txt"))?,
        "v0"
    );
    // Multiple upstream revisions while B is offline: only the latest is applied.
    for n in 1..=2 {
        std::fs::write(&file, format!("revision-{n}-{}", "x".repeat(n)))?;
        once(a_config.clone(), a.token.clone()).await?;
    }
    once(b_config.clone(), b.token.clone()).await?;
    assert_eq!(
        std::fs::read_to_string(destination.join("quote.txt"))?,
        "revision-2-xx"
    );
    for n in 3..=5 {
        std::fs::write(&file, format!("revision-{n}-{}", "x".repeat(n)))?;
        once(a_config.clone(), a.token.clone()).await?;
        once(b_config.clone(), b.token.clone()).await?;
    }
    let backup_dir = destination
        .join(".filesync-backups")
        .join(filesync::transfer::token_hash("quote.txt"));
    assert_eq!(std::fs::read_dir(backup_dir)?.count(), 3);
    // New files covered by the claimed prefix do not re-enter pending.
    std::fs::write(source.join("new.txt"), "new")?;
    once(a_config.clone(), a.token.clone()).await?;
    let new = once(b_config.clone(), b.token.clone()).await?;
    assert_eq!(new.pending, 0);
    assert_eq!(new.downloaded, 1);
    // Truncated publication must not remove previously published files.
    assert!(
        !client
            .put(format!("{url}/v1/manifest"))
            .bearer_auth(&a.token)
            .body("")
            .send()
            .await?
            .status()
            .is_success()
    );
    assert!(
        client
            .get(format!("{url}/v1/manifest/{}", a.id))
            .bearer_auth(&c.token)
            .send()
            .await?
            .status()
            .is_client_error()
    );
    // Two upstreams must not race to overwrite the same destinations.
    once(c_config.clone(), c.token.clone()).await?;
    let conflicting = format!(
        "{mappings}\n[[map]]\nupstream = {:?}\nfrom = \"quotes/\"\nto = {}\n",
        c.id,
        quoted(&destination)
    );
    write_config(
        &b_config,
        &url,
        &cert_path,
        &b.id,
        &root.join("b"),
        &conflicting,
    )?;
    let conflicts = once(b_config.clone(), b.token.clone()).await?;
    assert_eq!(conflicts.downloaded, 0);
    assert_eq!(conflicts.failed, 4);
    assert_eq!(
        std::fs::read_to_string(destination.join("quote.txt"))?,
        "revision-5-xxxxx"
    );
    // Withdrawal preserves downstream files and records obsolete metadata.
    std::fs::remove_file(&file)?;
    std::fs::remove_file(source.join("new.txt"))?;
    once(a_config.clone(), a.token.clone()).await?;
    once(c_config, c.token.clone()).await?;
    once(b_config, b.token).await?;
    assert!(destination.join("quote.txt").exists());
    let db = store::open(&root.join("b/state.db"))?;
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM local_files WHERE state='obsolete'",
            [],
            |r| r.get::<_, u64>(0)
        )?,
        4
    );
    server.abort();
    Ok(())
}
