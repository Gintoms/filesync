use anyhow::{Result, ensure};
use filesync::{
    agent,
    model::{NodeCredential, Page, Pending},
};
use serde_json::{Value, json};
use std::{
    path::Path,
    process::{Child, Command, Stdio},
    time::Duration,
};

struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn daemon(path: &Path, token: &str) -> Result<Process> {
    Ok(Process(
        Command::new(env!("CARGO_BIN_EXE_filesync"))
            .arg("--config")
            .arg(path)
            .arg("agent")
            .env("FILESYNC_UI_TEST_TOKEN", token)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?,
    ))
}
async fn control(path: &Path) -> Result<String> {
    for _ in 0..100 {
        if let Ok(addr) = std::fs::read_to_string(path.join("control.addr")) {
            return Ok(format!("http://{}", addr.trim()));
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    anyhow::bail!("agent did not start")
}
async fn exists(path: &Path) -> Result<()> {
    for _ in 0..200 {
        if path.is_file() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    anyhow::bail!("file not received: {}", path.display())
}

#[tokio::test]
async fn graphical_management_publishes_claims_and_reports_receipts() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let root = temp.path().canonicalize()?;
    let listener = tokio::net::TcpListener::bind("0.0.0.0:0").await?;
    let addr = listener.local_addr()?;
    let url = format!("http://127.0.0.1:{}", addr.port());
    let admin = "ui-test-admin-token-long-enough-00000000";
    let hub_config = root.join("hub.toml");
    std::fs::write(
        &hub_config,
        format!(
            "listen={:?}\ndata_dir={:?}\nadmin_token_env=\"FILESYNC_UI_TEST_ADMIN\"\nallow_http=true\n",
            addr.to_string(),
            root.join("hub").to_string_lossy()
        ),
    )?;
    drop(listener);
    let _hub = Process(
        Command::new(env!("CARGO_BIN_EXE_filesync"))
            .args(["--config", hub_config.to_str().unwrap(), "hub"])
            .env("FILESYNC_UI_TEST_ADMIN", admin)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?,
    );
    let client = agent::client(None)?;
    let mut healthy = false;
    for _ in 0..100 {
        if client
            .get(format!("{url}/health"))
            .send()
            .await
            .is_ok_and(|r| r.status().is_success())
        {
            healthy = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    ensure!(healthy, "HTTP hub failed to start on 0.0.0.0");
    assert_eq!(
        client
            .get(format!("{url}/v1/admin/nodes"))
            .send()
            .await?
            .status(),
        401
    );
    let page = client.get(format!("{url}/")).send().await?;
    if page.status().is_success() {
        assert!(page.headers().contains_key("content-security-policy"));
        assert!(page.text().await?.contains("id=\"app\""));
    }
    let mut credentials = Vec::new();
    for name in ["UI publisher", "UI receiver"] {
        credentials.push(
            client
                .post(format!("{url}/v1/nodes"))
                .bearer_auth(admin)
                .json(&json!({"name":name}))
                .send()
                .await?
                .error_for_status()?
                .json::<NodeCredential>()
                .await?,
        );
    }
    let a = &credentials[0];
    let b = &credentials[1];
    client
        .post(format!("{url}/v1/edges"))
        .bearer_auth(admin)
        .json(&json!({"upstream":a.id,"downstream":b.id}))
        .send()
        .await?
        .error_for_status()?;
    client
        .patch(format!("{url}/v1/admin/nodes/{}", b.id))
        .bearer_auth(admin)
        .json(&json!({"name":"Renamed receiver"}))
        .send()
        .await?
        .error_for_status()?;
    let nodes: Vec<Value> = client
        .get(format!("{url}/v1/admin/nodes"))
        .bearer_auth(admin)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert!(nodes.iter().any(|v| v["name"] == "Renamed receiver"));
    assert!(!serde_json::to_string(&nodes)?.contains(&a.token));
    let mut config_paths = Vec::new();
    for (i, node) in credentials.iter().enumerate() {
        let path = root.join(format!("node-{i}.toml"));
        std::fs::write(
            &path,
            format!(
                "hub={url:?}\nnode_id={:?}\ntoken_env=\"FILESYNC_UI_TEST_TOKEN\"\ndata_dir={:?}\ninterval_seconds=1\n",
                node.id,
                root.join(format!("data-{i}")).to_string_lossy()
            ),
        )?;
        config_paths.push(path);
    }
    let original = std::fs::read(&config_paths[0])?;
    let _a = daemon(&config_paths[0], &a.token)?;
    let _b = daemon(&config_paths[1], &b.token)?;
    let aurl = control(&root.join("data-0")).await?;
    let burl = control(&root.join("data-1")).await?;
    let source = root.join("source");
    std::fs::create_dir(&source)?;
    std::fs::write(source.join("hello.txt"), "graphical publishing")?;
    let body = json!({"path":source,"prefix":"team"});
    assert_eq!(
        client
            .post(format!("{aurl}/sources"))
            .json(&body)
            .send()
            .await?
            .status(),
        401
    );
    assert!(
        !client
            .post(format!("{burl}/sources"))
            .bearer_auth(&b.token)
            .json(&body)
            .send()
            .await?
            .status()
            .is_success()
    );
    client
        .post(format!("{aurl}/sources"))
        .bearer_auth(&a.token)
        .json(&body)
        .send()
        .await?
        .error_for_status()?;
    let mut pending = false;
    for _ in 0..200 {
        let files: Page<Pending> = client
            .get(format!("{burl}/files?state=pending&limit=1"))
            .bearer_auth(&b.token)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        if files.total == 1 {
            assert_eq!(files.items[0].path, "team/hello.txt");
            pending = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    ensure!(pending, "pending metadata not available");
    let target = root.join("received");
    assert!(!target.exists());
    client
        .post(format!("{burl}/claim"))
        .bearer_auth(&b.token)
        .json(&json!({"upstream":a.id,"from":"team/","to":target}))
        .send()
        .await?
        .error_for_status()?;
    exists(&target.join("hello.txt")).await?;
    assert_eq!(
        std::fs::read_to_string(target.join("hello.txt"))?,
        "graphical publishing"
    );
    std::fs::write(source.join("new.txt"), "new file auto received")?;
    client
        .post(format!("{aurl}/sync"))
        .bearer_auth(&a.token)
        .send()
        .await?
        .error_for_status()?;
    exists(&target.join("new.txt")).await?;
    let mut received = false;
    for _ in 0..100 {
        let p: Page<Value> = client
            .get(format!("{url}/v1/admin/deliveries?search=hello"))
            .bearer_auth(admin)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        if p.items.first().is_some_and(|v| v["received"] == true) {
            received = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    assert!(received);
    let p: Page<Value> = client
        .get(format!("{url}/v1/deliveries"))
        .bearer_auth(&b.token)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(
        p.total, 0,
        "downstream must not see publisher delivery records"
    );
    client
        .delete(format!("{aurl}/sources"))
        .bearer_auth(&a.token)
        .json(&body)
        .send()
        .await?
        .error_for_status()?;
    let mut obsolete = false;
    for _ in 0..200 {
        let p: Page<Pending> = client
            .get(format!("{burl}/files?state=obsolete"))
            .bearer_auth(&b.token)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        if p.total == 2 {
            obsolete = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    assert!(obsolete);
    assert!(target.join("hello.txt").is_file());
    assert_eq!(std::fs::read(&config_paths[0])?, original);
    client
        .delete(format!("{url}/v1/admin/edges"))
        .bearer_auth(admin)
        .json(&json!({"upstream":a.id,"downstream":b.id}))
        .send()
        .await?
        .error_for_status()?;
    let r: Value = client
        .get(format!("{url}/v1/relations"))
        .bearer_auth(&b.token)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert!(r["upstreams"].as_array().unwrap().is_empty());

    // Restore an active relationship and manifest before deleting the receiver.
    client
        .post(format!("{url}/v1/edges"))
        .bearer_auth(admin)
        .json(&json!({"upstream":a.id,"downstream":b.id}))
        .send()
        .await?
        .error_for_status()?;
    client
        .post(format!("{aurl}/sources"))
        .bearer_auth(&a.token)
        .json(&body)
        .send()
        .await?
        .error_for_status()?;
    let mut published = false;
    for _ in 0..100 {
        let p: Page<Value> = client
            .get(format!("{url}/v1/admin/deliveries"))
            .bearer_auth(admin)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        if p.total == 2 {
            published = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    ensure!(published, "manifest was not restored before node deletion");
    let delete_receiver = format!("{url}/v1/admin/nodes/{}", b.id);
    assert_eq!(client.delete(&delete_receiver).send().await?.status(), 401);
    assert_eq!(
        client
            .delete(&delete_receiver)
            .bearer_auth(&a.token)
            .send()
            .await?
            .status(),
        401
    );
    assert_eq!(
        client
            .delete(&delete_receiver)
            .bearer_auth(admin)
            .send()
            .await?
            .status(),
        204
    );
    assert_eq!(
        client
            .get(format!("{url}/v1/me"))
            .bearer_auth(&b.token)
            .send()
            .await?
            .status(),
        401
    );
    let db = filesync::store::open(&root.join("hub/hub.db"))?;
    for sql in [
        "SELECT COUNT(*) FROM edges WHERE upstream=?1 OR downstream=?1",
        "SELECT COUNT(*) FROM receipts WHERE node=?1 OR upstream=?1",
        "SELECT COUNT(*) FROM presence WHERE node=?1",
    ] {
        assert_eq!(db.query_row(sql, [&b.id], |r| r.get::<_, u64>(0))?, 0);
    }
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM files WHERE node=?1", [&a.id], |r| r
            .get::<_, u64>(
            0
        ))?,
        2
    );
    assert_eq!(
        client
            .delete(format!("{url}/v1/admin/nodes/{}", a.id))
            .bearer_auth(admin)
            .send()
            .await?
            .status(),
        204
    );
    assert_eq!(
        client
            .get(format!("{url}/v1/me"))
            .bearer_auth(&a.token)
            .send()
            .await?
            .status(),
        401
    );
    assert_eq!(
        client
            .delete(&delete_receiver)
            .bearer_auth(admin)
            .send()
            .await?
            .status(),
        404
    );
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM files", [], |r| r.get::<_, u64>(0))?,
        0
    );
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM nodes", [], |r| r.get::<_, u64>(0))?,
        0
    );
    assert_eq!(
        std::fs::read_to_string(target.join("hello.txt"))?,
        "graphical publishing"
    );
    assert_eq!(
        std::fs::read_to_string(target.join("new.txt"))?,
        "new file auto received"
    );
    Ok(())
}
