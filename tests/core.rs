use anyhow::Result;
use filesync::{
    config::{Mapping, Source},
    hub::insert_edge,
    model::Edge,
    scan, store, transfer,
};
use std::{io::Write, path::PathBuf};

#[test]
fn hub_http_requires_explicit_opt_in_on_network_listener() -> Result<()> {
    let mut config: filesync::config::HubConfig =
        toml::from_str("listen='0.0.0.0:8787'\ndata_dir='/unused'\nadmin_token_env='unused'")?;
    assert!(!config.allow_http);
    assert!(config.validate_transport().is_err());
    config.allow_http = true;
    config.validate_transport()?;
    config.tls_cert = Some("server.crt".into());
    assert!(config.validate_transport().is_err());
    config.tls_key = Some("server.key".into());
    config.allow_http = false;
    config.validate_transport()?;
    config.tls_cert = None;
    config.tls_key = None;
    config.listen = "127.0.0.1:8787".parse()?;
    config.validate_transport()?;
    Ok(())
}

#[test]
fn clients_accept_http_and_https_but_reject_other_schemes_and_embedded_credentials() {
    for url in [
        "http://192.0.2.8:8787",
        "http://127.0.0.1:8787",
        "http://[::1]:8787",
        "https://hub.example.com",
    ] {
        assert!(filesync::config::validate_url(url).is_ok(), "{url}");
    }
    for url in [
        "ftp://192.0.2.8",
        "file:///tmp/hub",
        "http://user:secret@192.0.2.8",
    ] {
        assert!(filesync::config::validate_url(url).is_err(), "{url}");
    }
}

#[test]
fn rejects_third_tier_and_self_edge() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let mut db = store::open(&temp.path().join("test.db"))?;
    for id in ["a", "b", "c"] {
        db.execute("INSERT INTO nodes VALUES(?1,?1,?1)", [id])?;
    }
    insert_edge(
        &mut db,
        &Edge {
            upstream: "a".into(),
            downstream: "b".into(),
        },
    )?;
    assert!(
        insert_edge(
            &mut db,
            &Edge {
                upstream: "b".into(),
                downstream: "c".into()
            }
        )
        .is_err()
    );
    assert!(
        insert_edge(
            &mut db,
            &Edge {
                upstream: "c".into(),
                downstream: "a".into()
            }
        )
        .is_err()
    );
    assert!(
        insert_edge(
            &mut db,
            &Edge {
                upstream: "c".into(),
                downstream: "c".into()
            }
        )
        .is_err()
    );
    insert_edge(
        &mut db,
        &Edge {
            upstream: "c".into(),
            downstream: "b".into(),
        },
    )?;
    Ok(())
}

#[test]
fn exact_and_longest_prefix_are_scoped_to_upstream() -> Result<()> {
    let maps = vec![
        Mapping {
            upstream: "a".into(),
            from: "docs/".into(),
            to: PathBuf::from("/general"),
        },
        Mapping {
            upstream: "a".into(),
            from: "docs/work/".into(),
            to: PathBuf::from("/work"),
        },
        Mapping {
            upstream: "a".into(),
            from: "docs/work/a.txt".into(),
            to: PathBuf::from("/special.txt"),
        },
    ];
    assert_eq!(
        transfer::mapped_target(&maps, "a", "docs/work/a.txt")?,
        Some(PathBuf::from("/special.txt"))
    );
    assert_eq!(
        transfer::mapped_target(&maps, "a", "docs/work/new.txt")?,
        Some(PathBuf::from("/work/new.txt"))
    );
    assert!(transfer::mapped_target(&maps, "b", "docs/work/new.txt")?.is_none());
    for path in [
        "../escape",
        "a/../b",
        "/root",
        "C:/bad",
        "a\\b",
        "a//b",
        "CON.txt",
    ] {
        assert!(transfer::validate_relative(path).is_err(), "{path}");
    }
    Ok(())
}

#[test]
fn replacement_keeps_three_backups_and_rejects_corruption() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let target = temp.path().canonicalize()?.join("document.txt");
    for n in 0..6 {
        let mut staged = tempfile::NamedTempFile::new_in(temp.path())?;
        write!(staged, "version-{n}")?;
        let sha = transfer::hash_file(staged.path())?;
        transfer::install_verified(staged, &target, &sha)?;
    }
    assert_eq!(std::fs::read_to_string(&target)?, "version-5");
    let dir = temp
        .path()
        .join(".filesync-backups")
        .join(transfer::token_hash("document.txt"));
    let mut contents = std::fs::read_dir(dir)?
        .map(|e| std::fs::read_to_string(e?.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    contents.sort();
    assert_eq!(contents, ["version-2", "version-3", "version-4"]);
    let mut corrupt = tempfile::NamedTempFile::new_in(temp.path())?;
    write!(corrupt, "corrupt")?;
    assert!(transfer::install_verified(corrupt, &target, &"0".repeat(64)).is_err());
    assert_eq!(std::fs::read_to_string(target)?, "version-5");
    Ok(())
}

#[test]
fn failed_scan_retains_previous_publication_queue() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let source = temp.path().join("source");
    std::fs::create_dir(&source)?;
    std::fs::write(source.join("one.txt"), "one")?;
    let mut db = store::open(&temp.path().join("test.db"))?;
    assert_eq!(
        scan::scan(
            &mut db,
            &[Source {
                path: source.clone(),
                prefix: "docs".into()
            }]
        )?,
        1
    );
    assert!(
        scan::scan(
            &mut db,
            &[Source {
                path: source.join("missing"),
                prefix: "docs".into()
            }]
        )
        .is_err()
    );
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM scan_queue", [], |r| r
            .get::<_, u64>(0))?,
        1
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn rejects_symlink_destination() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let outside = tempfile::tempdir()?;
    let root = temp.path().canonicalize()?;
    std::os::unix::fs::symlink(outside.path(), root.join("link"))?;
    assert!(transfer::prepare_target(&root.join("link/escape.txt")).is_err());
    assert!(!outside.path().join("escape.txt").exists());
    Ok(())
}
