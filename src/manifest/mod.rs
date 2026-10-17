pub mod bump;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::Context;
use serde::Serialize;
use serde_json::Value;

use crate::error::CliError;

pub const MANIFEST_FILE: &str = "times.aip.json";
pub const LOCALES: [&str; 4] = ["en", "am", "om", "ti"];
pub const SCHEMA_JSON: &str = include_str!("times.aip.schema.json");

pub const ALLOWED_SCOPES: &[&str] = &[
    "company:read",
    "company:write",
    "users:read",
    "users:write",
    "invoices:read",
    "invoices:write",
    "products:read",
    "products:write",
    "inventory:read",
    "inventory:write",
    "buyers:read",
    "buyers:write",
    "files:read",
    "files:write",
    "events:subscribe",
    "ui:overlay",
    "ui:notify",
    "state:read",
    "state:write",
    "tickets:read",
    "tickets:write",
    "tickets:admin",
    "hr:read",
    "hr:write",
    "hr:admin",
    "hr:self",
    "payroll:read",
    "payroll:write",
    "payroll:admin",
];

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Issue {
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Report {
    pub valid: bool,
    pub errors: Vec<Issue>,
    pub warnings: Vec<Issue>,
}

impl Report {
    fn error(&mut self, path: impl Into<String>, message: impl Into<String>) {
        self.errors.push(Issue {
            path: path.into(),
            message: message.into(),
        });
    }

    fn warn(&mut self, path: impl Into<String>, message: impl Into<String>) {
        self.warnings.push(Issue {
            path: path.into(),
            message: message.into(),
        });
    }
}

pub struct Loaded {
    pub root: PathBuf,
    pub path: PathBuf,
    pub value: Value,
}

pub fn find_root(dir: &Path) -> anyhow::Result<PathBuf> {
    let mut cur = Some(dir);
    while let Some(d) = cur {
        if d.join(MANIFEST_FILE).is_file() {
            return Ok(d.to_path_buf());
        }
        cur = d.parent();
    }
    Err(CliError::usage(format!(
        "No {MANIFEST_FILE} found in {} or its parents. Run `times init` to create one.",
        dir.display()
    )))
}

pub fn load(dir: &Path) -> anyhow::Result<Loaded> {
    let root = find_root(dir)?;
    let path = root.join(MANIFEST_FILE);
    let text =
        fs::read_to_string(&path).with_context(|| format!("Could not read {}", path.display()))?;
    let value: Value = serde_json::from_str(&text)
        .map_err(|e| CliError::validation(format!("{} is not valid JSON: {e}", path.display())))?;
    Ok(Loaded { root, path, value })
}

pub fn save(path: &Path, value: &Value) -> anyhow::Result<()> {
    let mut text = serde_json::to_string_pretty(value)?;
    text.push('\n');
    fs::write(path, text).with_context(|| format!("Could not write {}", path.display()))
}

fn validator() -> &'static jsonschema::Validator {
    static V: OnceLock<jsonschema::Validator> = OnceLock::new();
    V.get_or_init(|| {
        let schema: Value = serde_json::from_str(SCHEMA_JSON).expect("embedded schema is JSON");
        jsonschema::validator_for(&schema).expect("embedded schema compiles")
    })
}

pub fn str_at<'a>(v: &'a Value, pointer: &str) -> Option<&'a str> {
    v.pointer(pointer).and_then(Value::as_str)
}

pub fn is_semver(v: &str) -> bool {
    semver::Version::parse(v).is_ok()
}

pub fn validate(manifest: &Value, root: Option<&Path>) -> Report {
    let mut r = Report::default();

    for err in validator().iter_errors(manifest) {
        let path = err.instance_path.to_string();
        r.error(
            if path.is_empty() { "/".into() } else { path },
            err.to_string(),
        );
    }

    semantic_checks(manifest, &mut r);
    if let Some(root) = root {
        file_checks(manifest, root, &mut r);
    }

    r.errors.dedup();
    r.valid = r.errors.is_empty();
    r
}

fn semantic_checks(m: &Value, r: &mut Report) {
    let kind = str_at(m, "/kind").unwrap_or("");
    let mode = str_at(m, "/mode").unwrap_or("");

    if let Some(v) = str_at(m, "/version") {
        if !is_semver(v) {
            r.error(
                "/version",
                format!("`{v}` is not a semantic version like 1.0.0."),
            );
        }
    }

    for group in ["required", "optional"] {
        if let Some(list) = m
            .pointer(&format!("/scopes/{group}"))
            .and_then(Value::as_array)
        {
            for (i, s) in list.iter().enumerate() {
                if let Some(s) = s.as_str() {
                    if !ALLOWED_SCOPES.contains(&s) {
                        r.error(
                            format!("/scopes/{group}/{i}"),
                            format!("Unknown scope `{s}`."),
                        );
                    }
                }
            }
        }
    }

    if kind != "pack" && m.pointer("/ui/uses_times_tokens") != Some(&Value::Bool(true)) {
        r.error(
            "/ui/uses_times_tokens",
            "Apps and integrations must set ui.uses_times_tokens to true.",
        );
    }

    if mode == "platform" && str_at(m, "/ui/entry").unwrap_or("").is_empty() {
        r.error("/ui/entry", "Platform mode needs ui.entry.");
    }

    if mode == "runtime" {
        match str_at(m, "/runtime/workerUrl") {
            None | Some("") => r.error(
                "/runtime/workerUrl",
                "Runtime mode needs runtime.workerUrl.",
            ),
            Some(url) => {
                if !is_public_https(url) {
                    r.error(
                        "/runtime/workerUrl",
                        "runtime.workerUrl must be a public https URL.",
                    );
                }
            }
        }
    }

    let locales: Vec<&str> = m
        .pointer("/i18n/locales")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    for l in LOCALES {
        if !locales.contains(&l) {
            r.error("/i18n/locales", format!("Locale `{l}` is required."));
        }
        if str_at(m, &format!("/i18n/catalogs/{l}")).is_none() {
            r.error(
                format!("/i18n/catalogs/{l}"),
                format!("A catalog for `{l}` is required."),
            );
        }
    }

    if let Some(version) = str_at(m, "/version") {
        let has_entry = m
            .pointer("/changelog")
            .and_then(Value::as_array)
            .map(|a| a.iter().any(|e| str_at(e, "/version") == Some(version)))
            .unwrap_or(false);
        if !has_entry {
            r.error(
                "/changelog",
                format!("Add a changelog entry for version {version}."),
            );
        }
    }
}

fn is_public_https(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    let host_port = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = if host_port.starts_with('[') {
        host_port
            .split(']')
            .next()
            .map(|h| format!("{h}]"))
            .unwrap_or_default()
    } else {
        host_port.split(':').next().unwrap_or("").to_string()
    };
    let host = host.to_ascii_lowercase();
    !host.is_empty()
        && host != "localhost"
        && !host.ends_with(".localhost")
        && host != "127.0.0.1"
        && host != "[::1]"
        && host != "0.0.0.0"
}

pub fn relative(root: &Path, p: &str) -> PathBuf {
    root.join(p.trim_start_matches("./"))
}

fn lookup<'a>(catalog: &'a Value, key: &str) -> Option<&'a Value> {
    if let Some(v) = catalog.get(key) {
        return Some(v);
    }
    let mut cur = catalog;
    for part in key.split('.') {
        cur = cur.get(part)?;
    }
    Some(cur)
}

fn file_checks(m: &Value, root: &Path, r: &mut Report) {
    let mut keys: Vec<String> = vec!["meta.name".into()];
    if let (Some(version), Some(entries)) = (
        str_at(m, "/version"),
        m.pointer("/changelog").and_then(Value::as_array),
    ) {
        for e in entries
            .iter()
            .filter(|e| str_at(e, "/version") == Some(version))
        {
            if let Some(k) = str_at(e, "/summaryKey") {
                keys.push(k.to_string());
            }
            if let Some(hs) = e.get("highlightKeys").and_then(Value::as_array) {
                keys.extend(hs.iter().filter_map(Value::as_str).map(str::to_string));
            }
        }
    }

    for l in LOCALES {
        let Some(rel) = str_at(m, &format!("/i18n/catalogs/{l}")) else {
            continue;
        };
        let pointer = format!("/i18n/catalogs/{l}");
        let path = relative(root, rel);
        let text = match fs::read_to_string(&path) {
            Ok(t) => t,
            Err(_) => {
                r.error(pointer, format!("Catalog file {rel} is missing."));
                continue;
            }
        };
        let catalog: Value = match serde_json::from_str(&text) {
            Ok(v @ Value::Object(_)) => v,
            _ => {
                r.error(pointer, format!("Catalog {rel} must be a JSON object."));
                continue;
            }
        };
        for key in &keys {
            match lookup(&catalog, key) {
                Some(Value::String(s)) if !s.trim().is_empty() => {}
                Some(Value::String(_)) => {
                    r.warn(pointer.clone(), format!("`{key}` is empty in {rel}."))
                }
                _ if key == "meta.name" => {
                    r.warn(pointer.clone(), format!("Add `meta.name` to {rel}."))
                }
                _ => r.error(pointer.clone(), format!("`{key}` is missing from {rel}.")),
            }
        }
    }

    if let Some(entry) = str_at(m, "/ui/entry") {
        if !relative(root, entry).is_file() {
            r.warn(
                "/ui/entry",
                format!("{entry} does not exist yet. Build your app before `times aip push`."),
            );
        }
    }
    if let Some(icon) = str_at(m, "/ui/icon") {
        if !relative(root, icon).is_file() {
            r.error("/ui/icon", format!("Icon {icon} is missing."));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    pub fn sample() -> Value {
        json!({
            "schemaVersion": "1",
            "slug": "my-app",
            "kind": "app",
            "version": "1.0.0",
            "name": "My App",
            "mode": "platform",
            "ui": {
                "framework": "vite",
                "runtime": "iframe",
                "surfaces": ["page"],
                "entry": "./dist/index.html",
                "uses_times_tokens": true
            },
            "scopes": { "required": ["company:read"] },
            "i18n": {
                "locales": ["en", "am", "om", "ti"],
                "catalogs": {
                    "en": "./i18n/messages/en.json",
                    "am": "./i18n/messages/am.json",
                    "om": "./i18n/messages/om.json",
                    "ti": "./i18n/messages/ti.json"
                }
            },
            "changelog": [{
                "version": "1.0.0",
                "date": "2026-10-17",
                "summaryKey": "changelog.v1_0_0.summary",
                "highlightKeys": ["changelog.v1_0_0.h1"]
            }]
        })
    }

    #[test]
    fn valid_manifest_passes() {
        let r = validate(&sample(), None);
        assert!(r.valid, "{:?}", r.errors);
    }

    #[test]
    fn schema_errors_are_reported() {
        let mut m = sample();
        m["slug"] = json!("Bad Slug");
        m["extra"] = json!(true);
        let r = validate(&m, None);
        assert!(!r.valid);
        assert!(r.errors.iter().any(|e| e.path == "/slug"));
    }

    #[test]
    fn missing_locale_fails() {
        let mut m = sample();
        m["i18n"]["locales"] = json!(["en", "am", "om"]);
        m["i18n"]["catalogs"].as_object_mut().unwrap().remove("ti");
        let r = validate(&m, None);
        assert!(r.errors.iter().any(|e| e.message.contains("`ti`")));
    }

    #[test]
    fn unknown_scope_and_bad_version() {
        let mut m = sample();
        m["scopes"]["required"] = json!(["root:all"]);
        m["version"] = json!("1.0");
        let r = validate(&m, None);
        assert!(r.errors.iter().any(|e| e.message.contains("root:all")));
        assert!(r.errors.iter().any(|e| e.path == "/version"));
    }

    #[test]
    fn changelog_must_cover_current_version() {
        let mut m = sample();
        m["version"] = json!("1.1.0");
        let r = validate(&m, None);
        assert!(r.errors.iter().any(|e| e.path == "/changelog"));
    }

    #[test]
    fn runtime_requires_public_https_worker() {
        let mut m = sample();
        m["mode"] = json!("runtime");
        m["runtime"] = json!({ "workerUrl": "https://localhost:8787" });
        let r = validate(&m, None);
        assert!(r.errors.iter().any(|e| e.path == "/runtime/workerUrl"));

        m["runtime"] = json!({ "workerUrl": "https://worker.example.com/x" });
        assert!(validate(&m, None).valid);
    }

    #[test]
    fn public_https_rules() {
        assert!(is_public_https("https://a.example.com"));
        assert!(!is_public_https("http://a.example.com"));
        assert!(!is_public_https("https://127.0.0.1:9000"));
        assert!(!is_public_https("https://[::1]/x"));
        assert!(!is_public_https("https://dev.localhost"));
    }

    #[test]
    fn catalog_keys_are_checked_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let msgs = dir.path().join("i18n/messages");
        fs::create_dir_all(&msgs).unwrap();
        let full = json!({
            "meta": { "name": "My App" },
            "changelog": { "v1_0_0": { "summary": "First", "h1": "Hello" } }
        });
        for l in ["en", "am", "om"] {
            fs::write(msgs.join(format!("{l}.json")), full.to_string()).unwrap();
        }
        fs::write(
            msgs.join("ti.json"),
            json!({"meta": {"name": "x"}}).to_string(),
        )
        .unwrap();

        let r = validate(&sample(), Some(dir.path()));
        assert!(!r.valid);
        assert!(r
            .errors
            .iter()
            .any(|e| e.path == "/i18n/catalogs/ti" && e.message.contains("summary")));
        assert!(r.warnings.iter().any(|w| w.path == "/ui/entry"));
    }

    #[test]
    fn find_root_walks_up() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(MANIFEST_FILE), "{}").unwrap();
        let nested = dir.path().join("src/deep");
        fs::create_dir_all(&nested).unwrap();
        assert_eq!(
            find_root(&nested).unwrap().canonicalize().unwrap(),
            dir.path().canonicalize().unwrap()
        );
    }
}
