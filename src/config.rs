use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HubConfig {
    pub listen: SocketAddr,
    pub data_dir: PathBuf,
    pub admin_token_env: String,
    pub tls_cert: Option<PathBuf>,
    pub tls_key: Option<PathBuf>,
    #[serde(default)]
    pub allow_http: bool,
}

impl HubConfig {
    pub fn validate_transport(&self) -> Result<()> {
        ensure!(
            self.tls_cert.is_some() == self.tls_key.is_some(),
            "TLS certificate and key must be configured together"
        );
        ensure!(
            self.tls_cert.is_some() || self.listen.ip().is_loopback() || self.allow_http,
            "非本机监听需要配置 HTTPS；如需 HTTP，请在中心配置中设置 allow_http = true"
        );
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentConfig {
    pub hub: String,
    pub node_id: String,
    pub token_env: String,
    pub data_dir: PathBuf,
    pub ca_cert: Option<PathBuf>,
    #[serde(default = "default_interval")]
    pub interval_seconds: u64,
    #[serde(default)]
    pub source: Vec<Source>,
    #[serde(default)]
    pub map: Vec<Mapping>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub path: PathBuf,
    pub prefix: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Mapping {
    pub upstream: String,
    pub from: String,
    pub to: PathBuf,
}

fn default_interval() -> u64 {
    30
}

pub fn read<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let text = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    toml::from_str(&text).with_context(|| format!("invalid config: {}", path.display()))
}

impl AgentConfig {
    pub fn validate(&self) -> Result<()> {
        validate_url(&self.hub)?;
        ensure!(!self.node_id.is_empty(), "node_id is required");
        ensure!(self.data_dir.is_absolute(), "data_dir must be absolute");
        ensure!(
            self.interval_seconds > 0,
            "interval_seconds must be positive"
        );
        for source in &self.source {
            ensure!(source.path.is_absolute(), "source.path must be absolute");
            crate::transfer::validate_relative(&source.prefix)?;
        }
        for mapping in &self.map {
            ensure!(mapping.to.is_absolute(), "map.to must be absolute");
            crate::transfer::validate_relative(mapping.from.trim_end_matches('/'))?;
            ensure!(!mapping.upstream.is_empty(), "map.upstream is required");
        }
        Ok(())
    }
}

pub fn validate_url(value: &str) -> Result<()> {
    let url = reqwest::Url::parse(value)?;
    ensure!(
        matches!(url.scheme(), "http" | "https") && url.host_str().is_some(),
        "中心地址必须使用 http:// 或 https://，并包含主机地址"
    );
    ensure!(
        url.username().is_empty() && url.password().is_none(),
        "credentials must not be embedded in hub URL"
    );
    Ok(())
}

pub fn secret(name: &str) -> Result<String> {
    let value = std::env::var(name).with_context(|| format!("set environment variable {name}"))?;
    ensure!(
        value.len() >= 32,
        "{name} must contain at least 32 characters"
    );
    Ok(value)
}

/// Desktop enrollment stores its credential separately from hand-edited TOML.
pub fn node_secret(config: &AgentConfig) -> Result<String> {
    if std::env::var_os(&config.token_env).is_some() {
        return secret(&config.token_env);
    }
    let path = config.data_dir.join("node.token");
    let meta = std::fs::symlink_metadata(&path)
        .context("set the node token or connect using the desktop app")?;
    ensure!(
        meta.is_file() && !meta.file_type().is_symlink(),
        "invalid credential file"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        ensure!(
            meta.permissions().mode() & 0o077 == 0,
            "node.token must be private (chmod 600)"
        );
    }
    let token = std::fs::read_to_string(path)?.trim().to_owned();
    ensure!(token.len() >= 32, "invalid node credential");
    Ok(token)
}
