use crate::{
    config::Source,
    transfer::{MAX_FILE_SIZE, hash_file, validate_relative},
};
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OptionalExtension, params};
use std::{path::Path, time::UNIX_EPOCH};

pub fn scan(db: &mut Connection, sources: &[Source]) -> Result<u64> {
    // Transaction rolls back the queue on any unreadable/unstable source.
    let tx = db.transaction()?;
    tx.execute("DELETE FROM scan_queue", [])?;
    let mut count = 0;
    for source in sources {
        validate_relative(&source.prefix)?;
        visit(&tx, &source.path, &source.prefix, &mut count)?;
    }
    tx.commit()?;
    Ok(count)
}

fn visit(db: &Connection, path: &Path, logical: &str, count: &mut u64) -> Result<()> {
    let meta =
        std::fs::symlink_metadata(path).with_context(|| format!("scan {}", path.display()))?;
    ensure!(
        !meta.file_type().is_symlink(),
        "source symlink rejected: {}",
        path.display()
    );
    if meta.is_dir() {
        // Iterator, no directory-wide collect/sort; memory depends on depth.
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_str().context("non-UTF8 filename is unsupported")?;
            if matches!(name, ".filesync-backups" | ".git" | ".DS_Store") {
                continue;
            }
            visit(db, &entry.path(), &format!("{logical}/{name}"), count)?;
        }
    } else {
        ensure!(meta.is_file(), "source is not a regular file");
        ensure!(
            meta.len() < MAX_FILE_SIZE,
            "file must be smaller than 100 MB"
        );
        validate_relative(logical)?;
        let local = path.to_str().context("non-UTF8 path")?;
        let modified = meta
            .modified()?
            .duration_since(UNIX_EPOCH)?
            .as_nanos()
            .to_string();
        let cached: Option<String> = db
            .query_row(
                "SELECT sha FROM scan_cache WHERE local_path=?1 AND size=?2 AND modified=?3",
                params![local, meta.len(), modified],
                |r| r.get(0),
            )
            .optional()?;
        let sha = match cached {
            Some(sha) => sha,
            None => {
                let sha = hash_file(path)?;
                let after = std::fs::metadata(path)?;
                ensure!(
                    after.len() == meta.len() && after.modified()? == meta.modified()?,
                    "source changed during scan"
                );
                db.execute(
                    "INSERT OR REPLACE INTO scan_cache VALUES(?1,?2,?3,?4)",
                    params![local, meta.len(), modified, sha],
                )?;
                sha
            }
        };
        db.execute(
            "INSERT INTO scan_queue VALUES(?1,?2,?3,?4)",
            params![logical, local, sha, meta.len()],
        )
        .with_context(|| format!("duplicate published path: {logical}"))?;
        *count += 1;
    }
    Ok(())
}
