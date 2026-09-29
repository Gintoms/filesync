use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileEntry {
    pub path: String,
    pub sha256: String,
    pub size: u64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum ManifestEvent {
    File(FileEntry),
    Complete,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeCredential {
    pub id: String,
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewNode {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    pub upstream: String,
    pub downstream: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Relations {
    pub node_id: String,
    pub upstreams: Vec<Node>,
    pub downstreams: Vec<Node>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pending {
    pub upstream: String,
    pub path: String,
    pub sha256: String,
    pub size: u64,
    pub target: Option<String>,
    pub state: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Receipt {
    pub upstream: String,
    pub path: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SyncSummary {
    pub scanned: u64,
    pub published: u64,
    pub pending: u64,
    pub downloaded: u64,
    pub unchanged: u64,
    pub failed: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub total: u64,
}

#[derive(Debug, Default, Deserialize)]
pub struct PageQuery {
    #[serde(default)]
    pub offset: u32,
    pub limit: Option<u32>,
    #[serde(default)]
    pub search: String,
    #[serde(default)]
    pub state: String,
}
impl PageQuery {
    pub fn limit(&self) -> u32 {
        self.limit.unwrap_or(50).clamp(1, 100)
    }
}
