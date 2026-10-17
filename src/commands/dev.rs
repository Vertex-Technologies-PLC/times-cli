use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use tiny_http::{Header, Request, Response, Server};

use crate::bundle::build_dir_for;
use crate::cli::DevArgs;
use crate::error::CliError;
use crate::manifest::{self, relative, str_at, LOCALES, MANIFEST_FILE};
use crate::ui::Output;

struct Site {
    root: PathBuf,
    serve: PathBuf,
    entry: String,
    extras: BTreeSet<String>,
}

pub async fn run(out: &Output, args: DevArgs) -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    let loaded = manifest::load(&cwd)?;
    let report = manifest::validate(&loaded.value, Some(&loaded.root));
    for e in &report.errors {
        out.warn(format!("{} {}", e.path, e.message));
    }

    let root = loaded.root.canonicalize()?;
    let dir = args.dir.clone().or_else(|| build_dir_for(&loaded.value));
    let serve = match &dir {
        Some(d) => relative(&root, d),
        None => root.clone(),
    };
    if !serve.is_dir() {
        return Err(CliError::usage(format!(
            "Build folder {} does not exist. Run your build first, for example `bun run build`.",
            dir.unwrap_or_default()
        )));
    }
    let serve = serve.canonicalize()?;
    let entry_path = str_at(&loaded.value, "/ui/entry")
        .map(|e| relative(&root, e))
        .unwrap_or_else(|| serve.join("index.html"));
    let entry = entry_path
        .strip_prefix(&serve)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| "index.html".into());

    let mut extras = BTreeSet::new();
    extras.insert(MANIFEST_FILE.to_string());
    for l in LOCALES {
        if let Some(c) = str_at(&loaded.value, &format!("/i18n/catalogs/{l}")) {
            extras.insert(c.trim_start_matches("./").to_string());
        }
    }
    if let Some(icon) = str_at(&loaded.value, "/ui/icon") {
        extras.insert(icon.trim_start_matches("./").to_string());
    }

    let addr = format!("{}:{}", args.host, args.port);
    let server = Server::http(&addr).map_err(|e| {
        CliError::usage(format!(
            "Could not listen on {addr}: {e}. Try another --port."
        ))
    })?;
    let url = format!("http://{addr}/");
    let slug = str_at(&loaded.value, "/slug").unwrap_or("app").to_string();

    if out.json {
        out.json(&serde_json::json!({ "url": url, "serving": serve, "entry": entry }))?;
    } else {
        out.success(format!("Serving {slug} at {url}"));
        out.info(format!("Folder: {}", serve.display()));
        out.info("Rebuild your app to see changes. Press Ctrl+C to stop.");
    }

    let site = Site {
        root,
        serve,
        entry,
        extras,
    };
    tokio::task::spawn_blocking(move || {
        for req in server.incoming_requests() {
            handle(&site, req);
        }
    })
    .await?;
    Ok(())
}

fn handle(site: &Site, req: Request) {
    let raw = req.url().split(['?', '#']).next().unwrap_or("/");
    let path = percent_decode(raw.trim_start_matches('/'));
    let resolved = resolve(site, &path);
    let response = match resolved.and_then(|p| fs::read(&p).ok().map(|b| (p, b))) {
        Some((p, bytes)) => Response::from_data(bytes)
            .with_header(header("Content-Type", content_type(&p)))
            .with_header(header("Cache-Control", "no-store"))
            .with_header(header("Access-Control-Allow-Origin", "*")),
        None => Response::from_data(b"Not found".to_vec())
            .with_status_code(404)
            .with_header(header("Content-Type", "text/plain; charset=utf-8")),
    };
    let _ = req.respond(response);
}

fn resolve(site: &Site, path: &str) -> Option<PathBuf> {
    if path.split('/').any(|p| p == "..") {
        return None;
    }
    let target = if path.is_empty() {
        site.entry.as_str()
    } else {
        path
    };
    let candidate = site.serve.join(target);
    let candidate = if candidate.is_dir() {
        candidate.join("index.html")
    } else {
        candidate
    };
    if let Some(p) = inside(&site.serve, &candidate) {
        return Some(p);
    }
    if site.extras.contains(path) {
        return inside(&site.root, &site.root.join(path));
    }
    None
}

fn inside(base: &Path, p: &Path) -> Option<PathBuf> {
    let c = p.canonicalize().ok()?;
    (c.starts_with(base) && c.is_file()).then_some(c)
}

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("valid header")
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
            if let Ok(b) = u8::from_str_radix(hex, 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn content_type(p: &Path) -> &'static str {
    match p
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "wasm" => "application/wasm",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_and_types() {
        assert_eq!(percent_decode("a%20b"), "a b");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(
            content_type(Path::new("x/app.JS")),
            "text/javascript; charset=utf-8"
        );
    }

    #[test]
    fn resolves_build_and_project_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::create_dir_all(root.join("dist")).unwrap();
        fs::create_dir_all(root.join("i18n/messages")).unwrap();
        fs::write(root.join("dist/index.html"), "<html/>").unwrap();
        fs::write(root.join("i18n/messages/en.json"), "{}").unwrap();
        fs::write(root.join("secret.txt"), "no").unwrap();
        let site = Site {
            root: root.clone(),
            serve: root.join("dist"),
            entry: "index.html".into(),
            extras: ["i18n/messages/en.json".to_string()].into_iter().collect(),
        };
        assert!(resolve(&site, "").unwrap().ends_with("dist/index.html"));
        assert!(resolve(&site, "i18n/messages/en.json").is_some());
        assert!(resolve(&site, "secret.txt").is_none());
        assert!(resolve(&site, "../secret.txt").is_none());
    }
}
