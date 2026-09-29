use crate::{config::Mapping, model::FileEntry};
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::{
    io::Read,
    path::{Path, PathBuf},
};
use tokio::io::AsyncWriteExt;

pub const BUFFER_SIZE: usize = 64 * 1024;
pub const MAX_FILE_SIZE: u64 = 100_000_000;

pub fn validate_relative(path: &str) -> Result<()> {
    ensure!(
        !path.is_empty() && !path.starts_with('/') && !path.contains(['\\', '\0', ':']),
        "invalid relative path: {path}"
    );
    for part in path.split('/') {
        ensure!(
            !matches!(part, "" | "." | ".."),
            "invalid path component: {path}"
        );
        ensure!(
            !part.ends_with(['.', ' ']) && !part.contains(['<', '>', '"', '|', '?', '*']),
            "non-portable filename: {path}"
        );
        let name = part.split('.').next().unwrap_or("").to_ascii_uppercase();
        ensure!(
            !matches!(name.as_str(), "CON" | "PRN" | "AUX" | "NUL")
                && !(name.len() == 4
                    && (name.starts_with("COM") || name.starts_with("LPT"))
                    && matches!(name.as_bytes()[3], b'1'..=b'9')),
            "reserved filename: {path}"
        );
    }
    Ok(())
}

pub fn hash_file(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut buf = vec![0; BUFFER_SIZE];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

pub fn token_hash(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

pub fn mapped_target(maps: &[Mapping], upstream: &str, path: &str) -> Result<Option<PathBuf>> {
    validate_relative(path)?;
    let best = maps
        .iter()
        .filter(|m| {
            m.upstream == upstream
                && (m.from == path || (m.from.ends_with('/') && path.starts_with(&m.from)))
        })
        .max_by_key(|m| (!m.from.ends_with('/'), m.from.len()));
    Ok(best.map(|m| {
        if m.from == path {
            m.to.clone()
        } else {
            m.to.join(&path[m.from.len()..])
        }
    }))
}

/// Reject links at every existing component; production still needs handle-relative
/// traversal to protect against a hostile process replacing a directory concurrently.
pub fn prepare_target(target: &Path) -> Result<()> {
    ensure!(target.is_absolute(), "target must be absolute");
    let mut current = PathBuf::new();
    for component in target.components() {
        ensure!(
            !matches!(component, std::path::Component::ParentDir),
            "target cannot contain .."
        );
        current.push(component);
        match std::fs::symlink_metadata(&current) {
            Ok(meta) => ensure!(
                !meta.file_type().is_symlink(),
                "symlink target rejected: {}",
                current.display()
            ),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                if current != target {
                    std::fs::create_dir(&current)?;
                }
            }
            Err(err) => return Err(err.into()),
        }
    }
    if target.exists() {
        ensure!(target.is_file(), "target is not a regular file");
    }
    Ok(())
}

pub fn install_verified(
    temp: tempfile::NamedTempFile,
    target: &Path,
    expected: &str,
) -> Result<()> {
    ensure!(
        hash_file(temp.path())? == expected,
        "download hash mismatch"
    );
    prepare_target(target)?;
    let parent = target.parent().context("target has no parent")?;
    let name = target
        .file_name()
        .context("target has no filename")?
        .to_string_lossy();
    let backup_dir = parent.join(".filesync-backups").join(token_hash(&name));
    if target.exists() {
        prepare_target(&backup_dir.join("check"))?;
        let backup = tempfile::NamedTempFile::new_in(&backup_dir)?;
        std::fs::copy(target, backup.path())?;
        backup.as_file().sync_all()?;
        let time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        backup.persist(backup_dir.join(format!("{time:032}-{}.bak", uuid::Uuid::new_v4())))?;
    }
    temp.as_file().sync_all()?;
    temp.persist(target)?; // tempfile performs platform-specific atomic replacement.
    #[cfg(unix)]
    std::fs::File::open(parent)?.sync_all()?;
    // Only delete old backups after a successful replacement.
    if backup_dir.exists() {
        let mut backups = std::fs::read_dir(&backup_dir)?
            .map(|e| e.map(|e| e.path()))
            .collect::<std::io::Result<Vec<_>>>()?;
        backups.retain(|p| p.extension().is_some_and(|x| x == "bak"));
        backups.sort();
        let extra = backups.len().saturating_sub(3);
        for old in backups.into_iter().take(extra) {
            std::fs::remove_file(old)?;
        }
    }
    Ok(())
}

pub async fn download(
    client: &reqwest::Client,
    url: &str,
    token: &str,
    entry: &FileEntry,
    target: &Path,
) -> Result<()> {
    prepare_target(target)?;
    let temp = tempfile::NamedTempFile::new_in(target.parent().context("missing parent")?)?;
    let mut out = tokio::fs::File::from_std(temp.reopen()?);
    let mut response = client
        .get(url)
        .bearer_auth(token)
        .send()
        .await?
        .error_for_status()?;
    let mut received = 0u64;
    while let Some(chunk) = response.chunk().await? {
        received += chunk.len() as u64;
        ensure!(
            received <= entry.size && received < MAX_FILE_SIZE,
            "download exceeds declared size"
        );
        out.write_all(&chunk).await?;
    }
    ensure!(received == entry.size, "incomplete download");
    out.sync_all().await?;
    drop(out);
    let target = target.to_path_buf();
    let sha = entry.sha256.clone();
    tokio::task::spawn_blocking(move || install_verified(temp, &target, &sha)).await??;
    Ok(())
}
