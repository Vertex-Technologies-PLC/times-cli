use std::fs;
use std::path::Path;

use anyhow::Context;
use serde_json::{json, Map, Value};

use super::{relative, str_at, LOCALES};
use crate::error::CliError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Level {
    Major,
    Minor,
    Patch,
}

pub fn next_version(current: &str, level: Level) -> anyhow::Result<String> {
    let mut v = semver::Version::parse(current).map_err(|_| {
        CliError::validation(format!("`{current}` is not a semantic version like 1.0.0."))
    })?;
    let was_pre = !v.pre.is_empty();
    v.pre = semver::Prerelease::EMPTY;
    v.build = semver::BuildMetadata::EMPTY;
    match level {
        Level::Major => {
            if !(was_pre && v.minor == 0 && v.patch == 0) {
                v.major += 1;
            }
            v.minor = 0;
            v.patch = 0;
        }
        Level::Minor => {
            if !(was_pre && v.patch == 0) {
                v.minor += 1;
            }
            v.patch = 0;
        }
        Level::Patch => {
            if !was_pre {
                v.patch += 1;
            }
        }
    }
    Ok(v.to_string())
}

pub fn changelog_prefix(version: &str) -> String {
    let safe: String = version
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!("changelog.v{safe}")
}

pub fn placeholder(locale: &str) -> (&'static str, &'static str) {
    match locale {
        "am" => ("የዚህን ልቀት ማጠቃለያ ይጻፉ።", "አዲስ የተጨመረውን ይግለጹ።"),
        "om" => (
            "Cuunfaa gadhiifama kanaa barreessi.",
            "Waan haaraa ida'ame ibsi.",
        ),
        "ti" => ("ናይዚ ሕታም ጽማቕ ጽሓፍ።", "እቲ ሓድሽ ዝተወሰኸ ግለጽ።"),
        _ => (
            "Describe this release.",
            "Describe what merchants can do now.",
        ),
    }
}

pub fn today() -> String {
    chrono::Utc::now().format("%Y-%m-%d").to_string()
}

pub fn apply(manifest: &mut Value, level: Level, date: &str) -> anyhow::Result<(String, String)> {
    let current = str_at(manifest, "/version")
        .ok_or_else(|| CliError::validation("times.aip.json has no version."))?
        .to_string();
    let next = next_version(&current, level)?;
    manifest["version"] = json!(next);

    let prefix = changelog_prefix(&next);
    let entry = json!({
        "version": next,
        "date": date,
        "summaryKey": format!("{prefix}.summary"),
        "highlightKeys": [format!("{prefix}.h1")]
    });
    match manifest.get_mut("changelog").and_then(Value::as_array_mut) {
        Some(list) => {
            list.retain(|e| str_at(e, "/version") != Some(next.as_str()));
            list.insert(0, entry);
        }
        None => manifest["changelog"] = json!([entry]),
    }
    Ok((current, next))
}

fn insert_if_missing(root: &mut Value, key: &str, value: &str) -> bool {
    let parts: Vec<&str> = key.split('.').collect();
    let mut cur = root;
    for (i, part) in parts.iter().enumerate() {
        if !cur.is_object() {
            return false;
        }
        let map: &mut Map<String, Value> = cur.as_object_mut().unwrap();
        if i + 1 == parts.len() {
            if map.contains_key(*part) {
                return false;
            }
            map.insert(part.to_string(), json!(value));
            return true;
        }
        cur = map.entry(part.to_string()).or_insert_with(|| json!({}));
    }
    false
}

pub fn add_catalog_keys(
    manifest: &Value,
    root: &Path,
    version: &str,
) -> anyhow::Result<Vec<String>> {
    let prefix = changelog_prefix(version);
    let mut touched = Vec::new();
    for l in LOCALES {
        let Some(rel) = str_at(manifest, &format!("/i18n/catalogs/{l}")) else {
            continue;
        };
        let path = relative(root, rel);
        let mut catalog: Value = match fs::read_to_string(&path) {
            Ok(t) => serde_json::from_str(&t)
                .map_err(|e| CliError::validation(format!("{rel} is not valid JSON: {e}")))?,
            Err(_) => json!({}),
        };
        let (summary, highlight) = placeholder(l);
        let a = insert_if_missing(&mut catalog, &format!("{prefix}.summary"), summary);
        let b = insert_if_missing(&mut catalog, &format!("{prefix}.h1"), highlight);
        if a || b {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut text = serde_json::to_string_pretty(&catalog)?;
            text.push('\n');
            fs::write(&path, text).with_context(|| format!("Could not write {rel}"))?;
            touched.push(rel.to_string());
        }
    }
    Ok(touched)
}

pub fn sync_package_json(root: &Path, version: &str) -> anyhow::Result<bool> {
    let path = root.join("package.json");
    if !path.exists() {
        return Ok(false);
    }
    let text =
        fs::read_to_string(&path).with_context(|| format!("Could not read {}", path.display()))?;
    let mut pkg: Value = serde_json::from_str(&text)
        .map_err(|e| CliError::validation(format!("package.json is not valid JSON: {e}")))?;
    if str_at(&pkg, "/version") == Some(version) {
        return Ok(false);
    }
    pkg["version"] = json!(version);
    let mut out = serde_json::to_string_pretty(&pkg)?;
    out.push('\n');
    fs::write(&path, out).with_context(|| format!("Could not write {}", path.display()))?;
    Ok(true)
}

#[derive(Debug, Clone)]
pub struct BumpReport {
    pub from: String,
    pub to: String,
    pub catalogs: Vec<String>,
    pub package_json: bool,
}

pub fn bump_project(dir: &Path, level: Level) -> anyhow::Result<BumpReport> {
    let mut loaded = super::load(dir)?;
    let (from, to) = apply(&mut loaded.value, level, &today())?;
    super::save(&loaded.path, &loaded.value)?;
    let catalogs = add_catalog_keys(&loaded.value, &loaded.root, &to)?;
    let package_json = sync_package_json(&loaded.root, &to)?;
    Ok(BumpReport {
        from,
        to,
        catalogs,
        package_json,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bumps_follow_semver() {
        assert_eq!(next_version("1.2.3", Level::Patch).unwrap(), "1.2.4");
        assert_eq!(next_version("1.2.3", Level::Minor).unwrap(), "1.3.0");
        assert_eq!(next_version("1.2.3", Level::Major).unwrap(), "2.0.0");
        assert_eq!(next_version("1.2.3-beta.1", Level::Patch).unwrap(), "1.2.3");
        assert_eq!(next_version("1.3.0-rc.1", Level::Minor).unwrap(), "1.3.0");
        assert_eq!(next_version("2.0.0-rc.1", Level::Major).unwrap(), "2.0.0");
        assert!(next_version("1.2", Level::Patch).is_err());
    }

    #[test]
    fn prefix_is_key_safe() {
        assert_eq!(changelog_prefix("1.0.0"), "changelog.v1_0_0");
        assert_eq!(changelog_prefix("1.0.0-beta.2"), "changelog.v1_0_0_beta_2");
    }

    #[test]
    fn apply_updates_version_and_changelog() {
        let mut m = json!({"version": "1.0.0", "changelog": [{"version": "1.0.0"}]});
        let (from, to) = apply(&mut m, Level::Minor, "2026-10-17").unwrap();
        assert_eq!((from.as_str(), to.as_str()), ("1.0.0", "1.1.0"));
        assert_eq!(m["version"], "1.1.0");
        let first = &m["changelog"][0];
        assert_eq!(first["version"], "1.1.0");
        assert_eq!(first["date"], "2026-10-17");
        assert_eq!(first["summaryKey"], "changelog.v1_1_0.summary");
        assert_eq!(m["changelog"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn catalog_keys_added_once_per_locale() {
        let dir = tempfile::tempdir().unwrap();
        let m = json!({"i18n": {"catalogs": {
            "en": "./i18n/en.json", "am": "./i18n/am.json",
            "om": "./i18n/om.json", "ti": "./i18n/ti.json"
        }}});
        fs::create_dir_all(dir.path().join("i18n")).unwrap();
        fs::write(
            dir.path().join("i18n/en.json"),
            r#"{"changelog":{"v1_1_0":{"summary":"Kept"}}}"#,
        )
        .unwrap();

        let touched = add_catalog_keys(&m, dir.path(), "1.1.0").unwrap();
        assert_eq!(touched.len(), 4);
        let en: Value =
            serde_json::from_str(&fs::read_to_string(dir.path().join("i18n/en.json")).unwrap())
                .unwrap();
        assert_eq!(en["changelog"]["v1_1_0"]["summary"], "Kept");
        assert!(en["changelog"]["v1_1_0"]["h1"].is_string());
        let am: Value =
            serde_json::from_str(&fs::read_to_string(dir.path().join("i18n/am.json")).unwrap())
                .unwrap();
        assert_ne!(
            am["changelog"]["v1_1_0"]["summary"],
            en["changelog"]["v1_1_0"]["summary"]
        );

        assert!(add_catalog_keys(&m, dir.path(), "1.1.0")
            .unwrap()
            .is_empty());
    }

    #[test]
    fn sync_package_json_updates_version() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("package.json"),
            r#"{"name":"demo","version":"1.0.0"}"#,
        )
        .unwrap();
        assert!(sync_package_json(dir.path(), "1.0.1").unwrap());
        let pkg: Value =
            serde_json::from_str(&fs::read_to_string(dir.path().join("package.json")).unwrap())
                .unwrap();
        assert_eq!(pkg["version"], "1.0.1");
        assert!(!sync_package_json(dir.path(), "1.0.1").unwrap());
    }
}
