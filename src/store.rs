use anyhow::Result;
use rusqlite::Connection;
use std::{path::Path, time::Duration};

pub fn open(path: &Path) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let db = Connection::open(path)?;
    db.busy_timeout(Duration::from_secs(5))?;
    db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
        PRAGMA foreign_keys=ON; PRAGMA cache_size=-512;
        PRAGMA mmap_size=0; PRAGMA temp_store=FILE;
        CREATE TABLE IF NOT EXISTS nodes(id TEXT PRIMARY KEY, name TEXT NOT NULL, token_hash TEXT NOT NULL UNIQUE);
        CREATE TABLE IF NOT EXISTS edges(upstream TEXT REFERENCES nodes(id), downstream TEXT REFERENCES nodes(id), PRIMARY KEY(upstream,downstream));
        CREATE TABLE IF NOT EXISTS files(node TEXT REFERENCES nodes(id), path TEXT, sha TEXT NOT NULL, size INTEGER NOT NULL, PRIMARY KEY(node,path));
        CREATE TABLE IF NOT EXISTS receipts(node TEXT, upstream TEXT, path TEXT, sha TEXT, PRIMARY KEY(node,upstream,path));
        CREATE TABLE IF NOT EXISTS local_files(upstream TEXT, path TEXT, sha TEXT, size INTEGER, target TEXT, applied_sha TEXT, state TEXT NOT NULL, error TEXT, seen INTEGER NOT NULL DEFAULT 0, PRIMARY KEY(upstream,path));
        CREATE TABLE IF NOT EXISTS claims(upstream TEXT, source TEXT, target TEXT, PRIMARY KEY(upstream,source));
        CREATE TABLE IF NOT EXISTS scan_queue(path TEXT PRIMARY KEY, local_path TEXT NOT NULL, sha TEXT NOT NULL, size INTEGER NOT NULL);
        CREATE TABLE IF NOT EXISTS scan_cache(local_path TEXT PRIMARY KEY, size INTEGER NOT NULL, modified TEXT NOT NULL, sha TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS sources(prefix TEXT PRIMARY KEY, path TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS presence(node TEXT PRIMARY KEY REFERENCES nodes(id), last_seen INTEGER NOT NULL);")?;
    Ok(db)
}
