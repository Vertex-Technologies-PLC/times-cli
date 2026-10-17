use std::collections::BTreeMap;
use std::fs;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};

use anyhow::Context;
use serde_json::Value;
use sha2::{Digest, Sha256};
use walkdir::WalkDir;
use zip::write::SimpleFileOptions;

use crate::error::CliError;
use crate::manifest::{relative, str_at, LOCALES, MANIFEST_FILE};

pub const MAX_BUNDLE_BYTES: u64 = 50 * 1024 * 1024;
pub const CONTENT_TYPE: &str = "application/zip";

const IGNORED: &[&str] = &["node_modules", ".git", "target", ".DS_Store", ".times"];

pub struct Bundle {
    pub bytes: Vec<u8>,
    pub sha256: String,
    pub files: Vec<String>,
}

fn archive_name(root: &Path, path: &Path) -> anyhow::Result<String> {
    let rel = path
        .strip_prefix(root)
        .with_context(|| format!("{} is outside the project", path.display()))?;
    Ok(rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/"))
}

fn ignored(name: &str) -> bool {
    IGNORED.contains(&name) || name.ends_with(".zip")
}

fn add_dir(root: &Path, dir: &Path, files: &mut BTreeMap<String, PathBuf>) -> anyhow::Result<()> {
    let walker = WalkDir::new(dir)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| e.depth() == 0 || !ignored(&e.file_name().to_string_lossy()));
    for entry in walker {
        let entry = entry?;
        if entry.file_type().is_file() {
            files.insert(
                archive_name(root, entry.path())?,
                entry.path().to_path_buf(),
            );
        }
    }
    Ok(())
}

pub fn build_dir_for(manifest: &Value) -> Option<String> {
    let entry = str_at(manifest, "/ui/entry")?;
    let trimmed = entry.trim_start_matches("./");
    let mut parts = trimmed.split('/');
    let first = parts.next()?;
    parts.next().map(|_| first.to_string())
}

pub fn collect(
    root: &Path,
    manifest: &Value,
    dir: Option<&str>,
) -> anyhow::Result<BTreeMap<String, PathBuf>> {
    let root = root.canonicalize()?;
    let mut files = BTreeMap::new();
    let build_dir = dir.map(str::to_string).or_else(|| build_dir_for(manifest));

    match build_dir {
        Some(d) => {
            let path = relative(&root, &d);
            if !path.is_dir() {
                return Err(CliError::usage(format!(
                    "Build folder {d} does not exist. Build your app first, or pass --dir."
                )));
            }
            let path = path.canonicalize()?;
            if !path.starts_with(&root) {
                return Err(CliError::usage("--dir must be inside the project folder."));
            }
            add_dir(&root, &path, &mut files)?;
        }
        None => add_dir(&root, &root, &mut files)?,
    }

    files.insert(MANIFEST_FILE.to_string(), root.join(MANIFEST_FILE));
    for l in LOCALES {
        if let Some(rel) = str_at(manifest, &format!("/i18n/catalogs/{l}")) {
            let p = relative(&root, rel);
            if p.is_file() {
                files.insert(archive_name(&root, &p.canonicalize()?)?, p);
            }
        }
    }
    if let Some(icon) = str_at(manifest, "/ui/icon") {
        let p = relative(&root, icon);
        if p.is_file() {
            files.insert(archive_name(&root, &p.canonicalize()?)?, p);
        }
    }

    if let Some(entry) = str_at(manifest, "/ui/entry") {
        let name = entry.trim_start_matches("./");
        if !files.contains_key(name) {
            return Err(CliError::usage(format!(
                "Entry {entry} is not in the bundle. Build your app first, or pass --dir."
            )));
        }
    }
    Ok(files)
}

pub fn pack(root: &Path, manifest: &Value, dir: Option<&str>) -> anyhow::Result<Bundle> {
    let files = collect(root, manifest, dir)?;
    let mut buf = Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buf);
        let opts = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .unix_permissions(0o644);
        for (name, path) in &files {
            zip.start_file(name.as_str(), opts)?;
            let data =
                fs::read(path).with_context(|| format!("Could not read {}", path.display()))?;
            zip.write_all(&data)?;
        }
        zip.finish()?;
    }
    let bytes = buf.into_inner();
    let sha256 = sha256_hex(&bytes);
    Ok(Bundle {
        bytes,
        sha256,
        files: files.into_keys().collect(),
    })
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn human_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let b = bytes as f64;
    if b >= KB * KB {
        format!("{:.1} MB", b / KB / KB)
    } else if b >= KB {
        format!("{:.1} KB", b / KB)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn project() -> (tempfile::TempDir, Value) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("dist/assets")).unwrap();
        fs::create_dir_all(root.join("i18n/messages")).unwrap();
        fs::create_dir_all(root.join("node_modules/x")).unwrap();
        fs::write(root.join("dist/index.html"), "<html></html>").unwrap();
        fs::write(root.join("dist/assets/app.js"), "console.log(1)").unwrap();
        fs::write(root.join("node_modules/x/a.js"), "x").unwrap();
        fs::write(root.join("icon.svg"), "<svg/>").unwrap();
        for l in LOCALES {
            fs::write(root.join(format!("i18n/messages/{l}.json")), "{}").unwrap();
        }
        let m = json!({
            "ui": {"entry": "./dist/index.html", "icon": "./icon.svg"},
            "i18n": {"catalogs": {
                "en": "./i18n/messages/en.json", "am": "./i18n/messages/am.json",
                "om": "./i18n/messages/om.json", "ti": "./i18n/messages/ti.json"
            }}
        });
        fs::write(root.join(MANIFEST_FILE), m.to_string()).unwrap();
        (dir, m)
    }

    #[test]
    fn bundle_contains_manifest_build_and_catalogs() {
        let (dir, m) = project();
        let b = pack(dir.path(), &m, None).unwrap();
        assert!(b.files.contains(&MANIFEST_FILE.to_string()));
        assert!(b.files.contains(&"dist/index.html".to_string()));
        assert!(b.files.contains(&"dist/assets/app.js".to_string()));
        assert!(b.files.contains(&"i18n/messages/ti.json".to_string()));
        assert!(b.files.contains(&"icon.svg".to_string()));
        assert!(!b.files.iter().any(|f| f.contains("node_modules")));
        assert_eq!(b.sha256.len(), 64);

        let reader = zip::ZipArchive::new(Cursor::new(b.bytes)).unwrap();
        assert_eq!(reader.len(), b.files.len());
    }

    #[test]
    fn missing_build_dir_is_an_error() {
        let (dir, m) = project();
        fs::remove_dir_all(dir.path().join("dist")).unwrap();
        assert!(pack(dir.path(), &m, None).is_err());
    }

    #[test]
    fn root_entry_bundles_project_without_ignored() {
        let (dir, mut m) = project();
        m["ui"]["entry"] = json!("./index.html");
        fs::write(dir.path().join("index.html"), "<html></html>").unwrap();
        let files = collect(dir.path(), &m, None).unwrap();
        assert!(files.contains_key("index.html"));
        assert!(!files.keys().any(|f| f.starts_with("node_modules")));
    }

    #[test]
    fn helpers() {
        assert_eq!(
            build_dir_for(&json!({"ui": {"entry": "./dist/index.html"}})).as_deref(),
            Some("dist")
        );
        assert_eq!(
            build_dir_for(&json!({"ui": {"entry": "./index.html"}})),
            None
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(human_size(2048), "2.0 KB");
    }
}
