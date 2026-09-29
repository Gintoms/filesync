//! Two published revisions per path, with leases for unfinished uploads.
use crate::hub::{Hub, check_sha};
use anyhow::Result;
use rusqlite::params;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::UNIX_EPOCH,
};

pub(crate) struct BlobPin {
    readers: Arc<Mutex<HashMap<String, usize>>>,
    sha: String,
}

impl Drop for BlobPin {
    fn drop(&mut self) {
        let mut readers = self.readers.lock().unwrap();
        if let Some(count) = readers.get_mut(&self.sha) {
            *count -= 1;
            if *count == 0 {
                readers.remove(&self.sha);
            }
        }
    }
}

impl Hub {
    pub(crate) fn init_retention(&self) -> Result<()> {
        let mut db = self.db.lock().unwrap();
        let tx = db.transaction()?;
        tx.execute_batch("CREATE TABLE IF NOT EXISTS hub_previous(node TEXT REFERENCES nodes(id) ON DELETE CASCADE, path TEXT, sha TEXT NOT NULL, size INTEGER NOT NULL, PRIMARY KEY(node,path));
            CREATE TABLE IF NOT EXISTS hub_uploads(node TEXT REFERENCES nodes(id) ON DELETE CASCADE, sha TEXT NOT NULL, expires INTEGER NOT NULL, PRIMARY KEY(node,sha));
            CREATE TABLE IF NOT EXISTS hub_legacy_blobs(sha TEXT PRIMARY KEY, expires INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS hub_retention_meta(version INTEGER PRIMARY KEY);
            CREATE INDEX IF NOT EXISTS files_sha ON files(sha);
            CREATE INDEX IF NOT EXISTS hub_previous_sha ON hub_previous(sha);
            CREATE INDEX IF NOT EXISTS hub_uploads_sha ON hub_uploads(sha);")?;
        let initialized: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM hub_retention_meta WHERE version=1)",
            [],
            |r| r.get(0),
        )?;
        if !initialized {
            // Older hubs have no path/version history. Protect recent orphan uploads
            // once during upgrade; do not extend this grace period on every restart.
            for item in std::fs::read_dir(self.data_dir.join("blobs"))? {
                let item = item?;
                let sha = item.file_name().to_string_lossy().into_owned();
                if check_sha(&sha).is_err() || !item.file_type()?.is_file() {
                    continue;
                }
                let modified = item
                    .metadata()?
                    .modified()?
                    .duration_since(UNIX_EPOCH)?
                    .as_secs();
                tx.execute("INSERT OR IGNORE INTO hub_legacy_blobs SELECT ?1,MIN(?2,unixepoch())+86400 WHERE NOT EXISTS(SELECT 1 FROM files WHERE sha=?1)", params![sha, modified])?;
            }
            tx.execute("INSERT INTO hub_retention_meta VALUES(1)", [])?;
        }
        tx.commit()?;
        Ok(())
    }

    // Caller holds the DB mutex while opening the file and pinning it, excluding GC.
    pub(crate) fn pin_blob(&self, sha: &str) -> BlobPin {
        *self
            .active_reads
            .lock()
            .unwrap()
            .entry(sha.to_owned())
            .or_default() += 1;
        BlobPin {
            readers: self.active_reads.clone(),
            sha: sha.to_owned(),
        }
    }

    pub(crate) fn cleanup_blobs(&self) {
        match self.collect_blobs() {
            Ok(removed) if removed > 0 => tracing::info!(removed, "old file content reclaimed"),
            Ok(_) => {}
            Err(error) => tracing::warn!(%error, "blob cleanup deferred; will retry"),
        }
    }

    pub(crate) fn collect_blobs(&self) -> Result<u64> {
        {
            let db = self.db.lock().unwrap();
            db.execute("DELETE FROM hub_uploads WHERE expires<=unixepoch()", [])?;
            db.execute(
                "DELETE FROM hub_legacy_blobs WHERE expires<=unixepoch()",
                [],
            )?;
        }
        let mut removed = 0;
        for item in std::fs::read_dir(self.data_dir.join("blobs"))? {
            let item = item?;
            let sha = item.file_name().to_string_lossy().into_owned();
            if check_sha(&sha).is_err() || !item.file_type()?.is_file() {
                continue;
            }
            // Per-file lock keeps scans bounded and serializes the final reference
            // check + unlink with upload installation, publication and download open.
            let db = self.db.lock().unwrap();
            let retained: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM files WHERE sha=?1) OR EXISTS(SELECT 1 FROM hub_previous WHERE sha=?1) OR EXISTS(SELECT 1 FROM hub_uploads WHERE sha=?1) OR EXISTS(SELECT 1 FROM hub_legacy_blobs WHERE sha=?1)", [&sha], |r| r.get(0))?;
            if retained || self.active_reads.lock().unwrap().contains_key(&sha) {
                continue;
            }
            let bytes = match item.metadata() {
                Ok(metadata) => metadata.len(),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error.into()),
            };
            match std::fs::remove_file(item.path()) {
                Ok(()) => {
                    removed += 1;
                    tracing::info!(%sha, bytes, "unreferenced file content deleted");
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => tracing::warn!(%sha, %error, "could not reclaim content; will retry"),
            }
        }
        Ok(removed)
    }
}
