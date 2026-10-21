pub mod bump;
pub mod scopes;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckStatus {
    Pass,
    Warn,
    Fail,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Issue {
    pub path: String,
    pub message: String,
}

pub struct ValidationJob {
    pub name: &'static str,
    pub run: fn(&Value, Option<&Path>, &mut Report),
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

    pub fn check_status(&self) -> CheckStatus {
        if !self.errors.is_empty() {
            CheckStatus::Fail
        } else if !self.warnings.is_empty() {
            CheckStatus::Warn
        } else {
            CheckStatus::Pass
        }
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
    for job in validation_plan(manifest, root) {
        (job.run)(manifest, root, &mut r);
    }
    r.errors.dedup();
    r.valid = r.errors.is_empty();
    r
}

pub fn validation_plan(m: &Value, root: Option<&Path>) -> Vec<ValidationJob> {
    let kind = str_at(m, "/kind").unwrap_or("");
    let mut jobs = vec![
        ValidationJob {
            name: "Schema",
            run: schema_checks,
        },
        ValidationJob {
            name: "Version",
            run: version_checks,
        },
        ValidationJob {
            name: "Scopes",
            run: scope_checks,
        },
    ];
    if kind != "pack" {
        jobs.push(ValidationJob {
            name: "TIMES tokens",
            run: token_checks,
        });
    }
    if kind == "app" || kind == "integration" {
        jobs.push(ValidationJob {
            name: "Icons",
            run: icon_checks,
        });
    }
    jobs.push(ValidationJob {
        name: "Entry",
        run: entry_checks,
    });
    jobs.push(ValidationJob {
        name: "Locales",
        run: locale_checks,
    });
    jobs.push(ValidationJob {
        name: "Changelog",
        run: changelog_checks,
    });
    if str_at(m, "/hosting/provider").is_some() {
        jobs.push(ValidationJob {
            name: "Hosting",
            run: hosting_job,
        });
    }
    if root.is_some() {
        jobs.push(ValidationJob {
            name: "Catalogs",
            run: catalog_job,
        });
        if kind != "pack" {
            jobs.push(ValidationJob {
                name: "Theme",
                run: theme_job,
            });
        }
    }
    jobs
}

fn schema_checks(m: &Value, _: Option<&Path>, r: &mut Report) {
    for err in validator().iter_errors(m) {
        let path = err.instance_path.to_string();
        r.error(
            if path.is_empty() { "/".into() } else { path },
            err.to_string(),
        );
    }
}

fn version_checks(m: &Value, _: Option<&Path>, r: &mut Report) {
    if let Some(v) = str_at(m, "/version") {
        if !is_semver(v) {
            r.error(
                "/version",
                format!("`{v}` is not a semantic version like 1.0.0."),
            );
        }
    }
}

fn scope_checks(m: &Value, _: Option<&Path>, r: &mut Report) {
    for group in ["required", "optional"] {
        if let Some(list) = m
            .pointer(&format!("/scopes/{group}"))
            .and_then(Value::as_array)
        {
            for (i, s) in list.iter().enumerate() {
                if let Some(s) = s.as_str() {
                    if !scopes::allows(s) {
                        r.error(
                            format!("/scopes/{group}/{i}"),
                            format!("Unknown scope `{s}`."),
                        );
                    }
                }
            }
        }
    }
}

fn token_checks(m: &Value, _: Option<&Path>, r: &mut Report) {
    let kind = str_at(m, "/kind").unwrap_or("");
    if kind != "pack" && m.pointer("/ui/uses_times_tokens") != Some(&Value::Bool(true)) {
        r.error(
            "/ui/uses_times_tokens",
            "Apps and integrations must set ui.uses_times_tokens to true.",
        );
    }
}

fn icon_checks(m: &Value, root: Option<&Path>, r: &mut Report) {
    let kind = str_at(m, "/kind").unwrap_or("");
    if kind == "app" || kind == "integration" {
        if str_at(m, "/ui/icon").unwrap_or("").is_empty() {
            r.error(
                "/ui/icon",
                "Apps and integrations need a light-mode icon at ui.icon.",
            );
        }
        if str_at(m, "/ui/iconDark").unwrap_or("").is_empty() {
            r.error(
                "/ui/iconDark",
                "Apps and integrations need a dark-mode icon at ui.iconDark.",
            );
        }
    }
    let Some(root) = root else {
        return;
    };
    if let Some(icon) = str_at(m, "/ui/icon") {
        if !relative(root, icon).is_file() {
            r.error("/ui/icon", format!("Icon {icon} is missing."));
        }
    }
    if let Some(icon_dark) = str_at(m, "/ui/iconDark") {
        if !relative(root, icon_dark).is_file() {
            r.error("/ui/iconDark", format!("Dark icon {icon_dark} is missing."));
        }
        if str_at(m, "/ui/icon") == Some(icon_dark) {
            r.error(
                "/ui/iconDark",
                "ui.iconDark must be a different file from ui.icon.",
            );
        }
    }
}

fn entry_checks(m: &Value, root: Option<&Path>, r: &mut Report) {
    let mode = str_at(m, "/mode").unwrap_or("");
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

    let Some(root) = root else {
        return;
    };
    if let Some(entry) = str_at(m, "/ui/entry") {
        if !entry.starts_with("https://") && !relative(root, entry).is_file() {
            r.warn(
                "/ui/entry",
                format!("{entry} does not exist yet. Build your app before `times aip deploy`."),
            );
        }
    }
}

fn locale_checks(m: &Value, _: Option<&Path>, r: &mut Report) {
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
}

fn changelog_checks(m: &Value, _: Option<&Path>, r: &mut Report) {
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

fn hosting_job(m: &Value, _: Option<&Path>, r: &mut Report) {
    hosting_checks(m, r);
}

fn hosting_checks(m: &Value, r: &mut Report) {
    let Some(provider) = str_at(m, "/hosting/provider") else {
        return;
    };
    match provider {
        "developer" => match str_at(m, "/hosting/entryUrl") {
            None | Some("") => r.error(
                "/hosting/entryUrl",
                "Developer-hosted AIPs need hosting.entryUrl.",
            ),
            Some(url) if !is_public_https(url) => r.error(
                "/hosting/entryUrl",
                "hosting.entryUrl must be a public https URL.",
            ),
            _ => {}
        },
        "times" => {
            if let Some(url) = str_at(m, "/hosting/entryUrl") {
                if !url.is_empty() {
                    r.error(
                        "/hosting/entryUrl",
                        "Leave hosting.entryUrl empty when TIMES hosts the guest.",
                    );
                }
            }
        }
        other => r.error(
            "/hosting/provider",
            format!("Unknown hosting.provider `{other}`."),
        ),
    }
}

pub fn is_public_https(url: &str) -> bool {
    crate::netutil::is_public_https_url(url)
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

fn catalog_job(m: &Value, root: Option<&Path>, r: &mut Report) {
    let Some(root) = root else {
        return;
    };
    catalog_file_checks(m, root, r);
}

fn theme_job(m: &Value, root: Option<&Path>, r: &mut Report) {
    let Some(root) = root else {
        return;
    };
    theme_adoption_checks(m, root, r);
}

fn catalog_file_checks(m: &Value, root: &Path, r: &mut Report) {
    let mut keys: Vec<String> = vec![
        "meta.name".into(),
        "meta.summary".into(),
        "meta.description".into(),
    ];
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
                Some(Value::String(s)) if !s.trim().is_empty() => {
                    if s.contains('\u{2014}') {
                        r.error(
                            pointer.clone(),
                            format!(
                                "`{key}` in {rel} uses an em dash. Use a period, colon, or hyphen."
                            ),
                        );
                    }
                }
                Some(Value::String(_)) => {
                    r.warn(pointer.clone(), format!("`{key}` is empty in {rel}."))
                }
                _ if key.starts_with("meta.") => {
                    r.error(pointer.clone(), format!("Add `{key}` to {rel}."))
                }
                _ => r.error(pointer.clone(), format!("`{key}` is missing from {rel}.")),
            }
        }
        if l != "en" {
            if let Some(en_rel) = str_at(m, "/i18n/catalogs/en") {
                let en_path = relative(root, en_rel);
                if let Ok(en_text) = fs::read_to_string(&en_path) {
                    if text.trim() == en_text.trim() {
                        r.error(
                            pointer,
                            format!("Catalog {rel} is a copy of the English catalog. Write {l}."),
                        );
                    }
                }
            }
        }
    }
}

fn theme_adoption_checks(m: &Value, root: &Path, r: &mut Report) {
    let kind = str_at(m, "/kind").unwrap_or("");
    if kind == "pack" {
        return;
    }
    let Some(blob) = source_blob(root) else {
        return;
    };
    if !blob.contains("themes.css") {
        r.error(
            "/ui",
            "Import @times-aip/aip-sdk/themes.css, or link themes.css, so palette type and structure match TIMES.",
        );
    }
    let boots_palette = blob.contains("times-theme-boot")
        || blob.contains("THEME_BOOT_SCRIPT")
        || blob.contains("applyThemeFromLocation");
    if !boots_palette {
        r.error(
            "/ui",
            "Boot the TIMES palette with THEME_BOOT_SCRIPT or applyThemeFromLocation so the guest does not stay on the default sheet.",
        );
    }
    let follows_mode = blob.contains("colorMode")
        || blob.contains("@custom-variant dark")
        || blob.contains("classList.add('dark'")
        || blob.contains("classList.add(\"dark\"")
        || blob.contains("applyThemeToDocument");
    if !follows_mode {
        r.error(
            "/ui",
            "Follow the host color mode with the dark class or colorMode.",
        );
    }
    let loads_prefs = blob.contains("runtime/v1/preferences")
        || blob.contains("api.preferences")
        || blob.contains("applyUserPreferences")
        || blob.contains("syncPreferencesFromApi")
        || blob.contains("createTimesAipClient");
    if !loads_prefs {
        r.error(
            "/ui",
            "Load user preferences via createTimesAipClient (SDK fetches /api/v1/apps/runtime/v1/preferences) or call api.preferences / applyUserPreferences so locale, theme palette, and color mode match TIMES.",
        );
    }
    let supports_system = blob.contains("prefers-color-scheme")
        || blob.contains("colorMode === 'system'")
        || blob.contains("color_mode")
        || blob.contains("THEME_BOOT_SCRIPT")
        || blob.contains("times-theme-boot");
    if !supports_system {
        r.error(
            "/ui",
            "Support color mode light, dark, and system (prefers-color-scheme / THEME_BOOT_SCRIPT).",
        );
    }
    let framework = str_at(m, "/ui/framework").unwrap_or("");
    if framework != "static" && !blob.contains("@times-aip/aip-sdk") {
        r.error(
            "/ui",
            "Import @times-aip/aip-sdk so shared components match TIMES.",
        );
    }
    built_css_palette_checks(m, root, r);
}

fn built_css_palette_checks(m: &Value, root: &Path, r: &mut Report) {
    let entry = str_at(m, "/ui/entry").unwrap_or("");
    let candidates: [Option<std::path::PathBuf>; 4] = [
        Some(root.join("out")),
        Some(root.join("dist")),
        Some(root.join("build")),
        entry_parent(root, entry),
    ];
    let mut css_blobs = String::new();
    for dir in candidates.iter().flatten() {
        collect_built_css(dir, 0, &mut css_blobs);
    }
    if css_blobs.is_empty() {
        return;
    }
    if !css_blobs.contains("[data-theme") {
        r.error(
            "/ui",
            "Built CSS is missing [data-theme] palette rules. Import @times-aip/aip-sdk/themes.css and rebuild so Grove, Ember, and other TIMES palettes ship.",
        );
    }
    let loads_prefs_built = css_blobs.contains("runtime/v1/preferences")
        || css_blobs.contains("applyUserPreferences")
        || css_blobs.contains("syncPreferencesFromApi")
        || css_blobs.contains("createTimesAipClient")
        || css_blobs.contains("api.preferences");
    if !loads_prefs_built {
        r.error(
            "/ui",
            "Built bundle does not fetch user preferences. Use @times-aip/aip-sdk createTimesAipClient (prefs sync) or call /api/v1/apps/runtime/v1/preferences, then rebuild.",
        );
    }
    for id in [
        "claude",
        "discord",
        "supabase",
        "vercel",
        "mono",
        "notebook",
        "light-green",
        "zen",
        "astro-vista",
        "whatsapp",
    ] {
        let needle = format!("data-theme={id}");
        let needle_quoted = format!("data-theme='{id}'");
        let needle_dquoted = format!("data-theme=\"{id}\"");
        if !(css_blobs.contains(&needle)
            || css_blobs.contains(&needle_quoted)
            || css_blobs.contains(&needle_dquoted))
        {
            r.error(
                "/ui",
                format!(
                    "Built CSS is missing the `{id}` palette. Rebuilding without themes.css drops TIMES theme support."
                ),
            );
            break;
        }
    }
}

fn entry_parent(root: &Path, entry: &str) -> Option<std::path::PathBuf> {
    if entry.is_empty() {
        return None;
    }
    let path = relative(root, entry);
    path.parent().map(|p| p.to_path_buf())
}

fn collect_built_css(dir: &Path, depth: u8, out: &mut String) {
    if depth > 8 || !dir.is_dir() {
        return;
    }
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || name == "node_modules" {
            continue;
        }
        if path.is_dir() {
            collect_built_css(&path, depth + 1, out);
            continue;
        }
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if ext != "css" && ext != "js" && ext != "html" {
            continue;
        }
        if let Ok(text) = fs::read_to_string(&path) {
            out.push_str(&text);
            out.push('\n');
        }
    }
}

fn source_blob(root: &Path) -> Option<String> {
    let mut files = Vec::new();
    collect_sources(root, 0, &mut files);
    if files.is_empty() {
        None
    } else {
        Some(files.join("\n"))
    }
}

fn collect_sources(dir: &Path, depth: u8, out: &mut Vec<String>) {
    if depth > 6 {
        return;
    }
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || name == "node_modules" || name == "dist" || name == "out" {
            continue;
        }
        if path.is_dir() {
            collect_sources(&path, depth + 1, out);
            continue;
        }
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if !matches!(ext, "css" | "tsx" | "ts" | "html" | "jsx" | "js") {
            continue;
        }
        if let Ok(text) = fs::read_to_string(&path) {
            out.push(text);
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
                "icon": "./icon.svg",
                "iconDark": "./icon-dark.svg",
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
    fn plan_lists_the_checks_that_apply() {
        let names: Vec<_> = validation_plan(&sample(), None)
            .into_iter()
            .map(|job| job.name)
            .collect();
        assert_eq!(
            names,
            vec![
                "Schema",
                "Version",
                "Scopes",
                "TIMES tokens",
                "Icons",
                "Entry",
                "Locales",
                "Changelog"
            ]
        );
    }

    #[test]
    fn check_status_follows_the_issues() {
        let mut report = Report::default();
        assert_eq!(report.check_status(), CheckStatus::Pass);
        report.warn("/", "missing name");
        assert_eq!(report.check_status(), CheckStatus::Warn);
        report.error("/", "broken");
        assert_eq!(report.check_status(), CheckStatus::Fail);
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
        assert!(!is_public_https("https://10.0.0.4/app"));
        assert!(!is_public_https("https://169.254.169.254/"));
        assert!(!is_public_https("https://a.example.com:8443/"));
    }

    #[test]
    fn catalog_keys_are_checked_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let msgs = dir.path().join("i18n/messages");
        fs::create_dir_all(&msgs).unwrap();
        let full = json!({
            "meta": {
                "name": "My App",
                "summary": "A short summary for the store.",
                "description": "A longer description used for the marketplace listing detail page."
            },
            "changelog": { "v1_0_0": { "summary": "First", "h1": "Hello" } }
        });
        fs::write(msgs.join("en.json"), full.to_string()).unwrap();
        let am = json!({
            "meta": {
                "name": "መተግበሪያ",
                "summary": "ለመደብሩ አጭር ማጠቃለያ።",
                "description": "በገበያ ዝርዝር ገጽ ላይ የሚታይ ረዘም ያለ መግለጫ።"
            },
            "changelog": { "v1_0_0": { "summary": "መጀመሪያ", "h1": "ሰላም" } }
        });
        fs::write(msgs.join("am.json"), am.to_string()).unwrap();
        let om = json!({
            "meta": {
                "name": "Appii",
                "summary": "Cuunfaa gabaabaa gabaaf.",
                "description": "Ibsa dheeraa fuula tarree gabaatii irratti mul'atu."
            },
            "changelog": { "v1_0_0": { "summary": "Jalqaba", "h1": "Nagaa" } }
        });
        fs::write(msgs.join("om.json"), om.to_string()).unwrap();
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
            .any(|e| e.path == "/i18n/catalogs/ti" && e.message.contains("meta.summary")));
        assert!(r.warnings.iter().any(|w| w.path == "/ui/entry"));
    }

    #[test]
    fn copied_catalog_and_em_dash_fail() {
        let dir = tempfile::tempdir().unwrap();
        let msgs = dir.path().join("i18n/messages");
        fs::create_dir_all(&msgs).unwrap();
        let en = json!({
            "meta": {
                "name": "My App",
                "summary": "A short summary for the store.",
                "description": "A longer description used for the marketplace listing detail page."
            },
            "changelog": { "v1_0_0": { "summary": "First — note", "h1": "Hello" } }
        });
        for l in ["en", "am", "om", "ti"] {
            fs::write(msgs.join(format!("{l}.json")), en.to_string()).unwrap();
        }
        let r = validate(&sample(), Some(dir.path()));
        assert!(r.errors.iter().any(|e| e.message.contains("em dash")));
        assert!(r
            .errors
            .iter()
            .any(|e| e.message.contains("copy of the English")));
    }

    #[test]
    fn light_and_dark_icons_are_required() {
        let mut m = sample();
        m["ui"].as_object_mut().unwrap().remove("iconDark");
        let r = validate(&m, None);
        assert!(r.errors.iter().any(|e| e.path == "/ui/iconDark"));
    }

    #[test]
    fn palette_css_and_color_mode_are_required_when_source_exists() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("index.html"), "<html></html>").unwrap();
        let r = validate(&sample(), Some(dir.path()));
        assert!(r.errors.iter().any(|e| e.message.contains("themes.css")));
        assert!(r.errors.iter().any(|e| e.message.contains("color mode")));
        assert!(r
            .errors
            .iter()
            .any(|e| e.message.contains("THEME_BOOT_SCRIPT")));
    }

    #[test]
    fn palette_boot_is_required_even_when_themes_css_is_linked() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("index.html"),
            "<link rel=\"stylesheet\" href=\"./themes.css\" />",
        )
        .unwrap();
        fs::write(dir.path().join("app.ts"), "colorMode").unwrap();
        let r = validate(&sample(), Some(dir.path()));
        assert!(r
            .errors
            .iter()
            .any(|e| e.message.contains("THEME_BOOT_SCRIPT")));
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
