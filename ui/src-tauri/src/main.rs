#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use anyhow::{Context, Result, ensure};
use filesync::{
    agent,
    config::{self, AgentConfig},
    model::Node,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Mutex,
    time::Duration,
};
use tauri::{
    Manager, State,
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
};

struct Desktop {
    config: Mutex<PathBuf>,
    transition: tokio::sync::Mutex<()>,
}
fn string_error(e: anyhow::Error) -> String {
    format!("{e:#}")
}

fn probe_config(state: &Desktop) -> Result<AgentConfig> {
    let cfg: AgentConfig = config::read(&state.config.lock().unwrap())?;
    cfg.validate()?;
    Ok(cfg)
}
async fn local_request(cfg: &AgentConfig, path: &str, method: &str, body: Value) -> Result<Value> {
    let url = agent::control_url(cfg)?;
    let method = reqwest::Method::from_bytes(method.as_bytes())?;
    let client = agent::client(None)?;
    let mut request = client
        .request(method, format!("{url}{path}"))
        .timeout(Duration::from_secs(15))
        .bearer_auth(config::node_secret(cfg)?);
    if !body.is_null() {
        request = request.json(&body);
    }
    let response = request.send().await.context("后台尚未启动或无法连接")?;
    let status = response.status();
    let text = response.text().await?;
    ensure!(
        status.is_success(),
        "{}",
        if text.is_empty() {
            status.to_string()
        } else {
            text.clone()
        }
    );
    if text.is_empty() {
        Ok(Value::Null)
    } else {
        Ok(serde_json::from_str(&text)?)
    }
}
fn allowed_request(path: &str, method: &str) -> bool {
    let base = path.split('?').next().unwrap_or("");
    matches!(
        (method, base),
        (
            "GET",
            "/status" | "/info" | "/relations" | "/files" | "/sources" | "/maps" | "/deliveries"
        ) | ("POST", "/sources" | "/claim" | "/sync")
            | ("DELETE", "/sources" | "/maps")
    ) && !path.contains('#')
        && path.len() < 4096
}
#[tauri::command]
async fn agent_request(
    state: State<'_, Desktop>,
    path: String,
    method: String,
    body: Value,
) -> Result<Value, String> {
    if !allowed_request(&path, &method) {
        return Err("unsupported desktop operation".into());
    }
    let cfg = probe_config(&state).map_err(string_error)?;
    local_request(&cfg, &path, &method, body)
        .await
        .map_err(string_error)
}
fn agent_binary() -> Result<PathBuf> {
    let name = if cfg!(windows) {
        "filesync.exe"
    } else {
        "filesync"
    };
    let sibling = std::env::current_exe()?
        .parent()
        .context("missing executable directory")?
        .join(name);
    if sibling.is_file() {
        return Ok(sibling);
    }
    // Only development builds may use the repository binary.
    #[cfg(debug_assertions)]
    {
        let dev = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/release")
            .join(name);
        if dev.is_file() {
            return Ok(dev.canonicalize()?);
        }
    }
    anyhow::bail!("找不到后台程序 filesync，请使用完整的桌面安装包")
}
async fn connect(path: &Path) -> Result<Value> {
    let cfg: AgentConfig = config::read(path)?;
    cfg.validate()?;
    let token = config::node_secret(&cfg)?;
    let status = local_request(&cfg, "/status", "GET", Value::Null).await;
    if let Ok(value) = status {
        ensure!(
            value["control_version"].as_u64().unwrap_or(0) >= 2,
            "后台版本较旧，请先停止旧 Agent，再打开桌面客户端"
        );
        ensure!(value["node_id"] == cfg.node_id, "本机后台身份与配置不一致");
    } else {
        std::fs::create_dir_all(&cfg.data_dir)?;
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(cfg.data_dir.join("agent.log"))?;
        let mut command = Command::new(agent_binary()?);
        command
            .arg("--config")
            .arg(path)
            .arg("agent")
            .env(&cfg.token_env, token)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(log));
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000 | 0x00000200);
        }
        let mut child = command.spawn().context("启动后台同步进程失败")?;
        let mut started = false;
        for _ in 0..60 {
            tokio::time::sleep(Duration::from_millis(150)).await;
            if let Ok(s) = local_request(&cfg, "/status", "GET", Value::Null).await
                && s["node_id"] == cfg.node_id
                && s["control_version"].as_u64().unwrap_or(0) >= 2
            {
                started = true;
                break;
            }
            if let Some(exit) = child.try_wait()? {
                anyhow::bail!(
                    "后台启动失败 ({exit})，请查看 {}",
                    cfg.data_dir.join("agent.log").display()
                )
            }
        }
        if !started {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("后台启动超时，请查看 agent.log");
        }
        // Reap the child if it exits while the desktop app is still alive.
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
    Ok(json!({"node_id":cfg.node_id,"hub":cfg.hub,"config_path":path}))
}
#[tauri::command]
fn desktop_has_config(state: State<'_, Desktop>) -> bool {
    state.config.lock().unwrap().is_file()
}

#[tauri::command]
async fn desktop_connect(
    state: State<'_, Desktop>,
    config_path: Option<PathBuf>,
) -> Result<Value, String> {
    let _guard = state.transition.lock().await;
    let path = config_path.unwrap_or_else(|| state.config.lock().unwrap().clone());
    let path = path.canonicalize().map_err(|e| e.to_string())?;
    let result = connect(&path).await.map_err(string_error)?;
    *state.config.lock().unwrap() = path;
    Ok(result)
}
#[derive(Deserialize)]
struct SetupInput {
    hub: String,
    node_id: String,
    token: String,
    ca_cert: Option<PathBuf>,
}
fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}
#[tauri::command]
async fn desktop_setup(state: State<'_, Desktop>, input: SetupInput) -> Result<Value, String> {
    let _guard = state.transition.lock().await;
    async {
        let path = state.config.lock().unwrap().clone();
        ensure!(!path.exists(), "已经存在配置，请使用已有配置连接");
        ensure!(input.token.len() >= 32, "节点令牌至少需要 32 个字符");
        config::validate_url(&input.hub)?;
        let me: Node = agent::client(input.ca_cert.as_deref())?
            .get(format!("{}/v1/me", input.hub.trim_end_matches('/')))
            .bearer_auth(&input.token)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        ensure!(me.id == input.node_id, "节点 ID 与令牌不匹配");
        let parent = path.parent().context("invalid configuration path")?;
        std::fs::create_dir_all(parent)?;
        let data = parent.canonicalize()?.join("agent-state");
        std::fs::create_dir_all(&data)?;
        let cfg = AgentConfig {
            hub: input.hub,
            node_id: input.node_id,
            token_env: "FILESYNC_NODE_TOKEN".into(),
            data_dir: data.clone(),
            ca_cert: input.ca_cert,
            interval_seconds: 30,
            source: vec![],
            map: vec![],
        };
        cfg.validate()?;
        let toml = toml::to_string_pretty(&cfg)?;
        let token_path = data.join("node.token");
        write_private(&token_path, input.token.as_bytes())?;
        if let Err(e) = write_private(&path, toml.as_bytes()) {
            let _ = std::fs::remove_file(token_path);
            return Err(e);
        }
        let mut result = connect(&path).await?;
        result["name"] = me.name.into();
        Ok(result)
    }
    .await
    .map_err(string_error)
}
fn show(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}
fn main() {
    filesync::init_crypto();
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| show(app)))
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let args: Vec<_> = std::env::args().collect();
            let path = args
                .windows(2)
                .find(|p| p[0] == "--config")
                .map(|p| PathBuf::from(&p[1]))
                .unwrap_or(app.path().app_data_dir()?.join("agent.toml"));
            app.manage(Desktop {
                config: Mutex::new(path),
                transition: tokio::sync::Mutex::new(()),
            });
            let open = MenuItem::with_id(app, "open", "打开 FileSync", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "退出界面（继续同步）", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &quit])?;
            let mut tray = TrayIconBuilder::new()
                .menu(&menu)
                .tooltip("FileSync · 后台文件同步")
                .on_menu_event(|app, e| match e.id.as_ref() {
                    "open" => show(app),
                    "quit" => app.exit(0),
                    _ => {}
                });
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.build(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            desktop_has_config,
            desktop_connect,
            desktop_setup,
            agent_request
        ])
        .run(tauri::generate_context!())
        .expect("FileSync desktop failed");
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bridge_rejects_arbitrary_paths() {
        assert!(allowed_request("/files?offset=50", "GET"));
        for path in [
            "http://evil.invalid/status",
            "//evil.invalid/status",
            "/status/../../v1/nodes",
            "/v1/nodes",
            "/status#x",
        ] {
            assert!(!allowed_request(path, "GET"));
        }
        assert!(!allowed_request("/sources", "PUT"));
    }
}
