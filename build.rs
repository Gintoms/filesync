use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=ui/dist");
    let mut out = String::from(
        "pub fn asset(path: &str) -> Option<(&'static str, &'static [u8])> { match path {\n",
    );
    if env::var_os("CARGO_FEATURE_WEB").is_some() {
        let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("ui/dist");
        if root.join("index.html").exists() {
            let mut files = vec![root.join("index.html")];
            if let Ok(entries) = fs::read_dir(root.join("assets")) {
                files.extend(entries.map(|e| e.unwrap().path()).filter(|p| p.is_file()));
            }
            for file in files {
                let path = file
                    .strip_prefix(&root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                let mime = match file.extension().and_then(|e| e.to_str()) {
                    Some("html") => "text/html; charset=utf-8",
                    Some("js") => "text/javascript; charset=utf-8",
                    Some("css") => "text/css; charset=utf-8",
                    _ => "application/octet-stream",
                };
                out += &format!("{path:?} => Some(({mime:?}, include_bytes!({file:?}))),\n");
            }
        } else {
            println!(
                "cargo:warning=Web assets missing; run npm --prefix ui ci && npm --prefix ui run build, then rebuild filesync"
            );
        }
    }
    out += "_ => None, }}\n";
    fs::write(
        PathBuf::from(env::var("OUT_DIR").unwrap()).join("web_assets.rs"),
        out,
    )
    .unwrap();
}
