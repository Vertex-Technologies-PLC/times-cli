use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::bundle;
use crate::error::CliError;
use crate::hosting::Provider;

pub const DIR: &str = ".times";
pub const FILE: &str = "push-state.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PushState {
    pub slug: String,
    pub version: String,
    pub provider: String,
    pub sha256: String,
    pub file_count: u32,
    pub uploaded_at: String,
}

#[derive(Debug)]
pub enum ChangeCheck {
    Unchanged,
    VersionBumpRequired { last_version: String },
    Proceed,
}

pub fn path(root: &Path) -> PathBuf {
    root.join(DIR).join(FILE)
}

pub fn load(root: &Path) -> anyhow::Result<Option<PushState>> {
    let p = path(root);
    if !p.is_file() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(&p)?;
    let state = serde_json::from_str(&raw).map_err(|e| {
        CliError::validation(format!(
            "{} is not valid JSON ({e}). Delete it or pass --force.",
            p.display()
        ))
    })?;
    Ok(Some(state))
}

pub fn save(root: &Path, state: &PushState) -> anyhow::Result<()> {
    let dir = root.join(DIR);
    std::fs::create_dir_all(&dir)?;
    let raw = serde_json::to_string_pretty(state)?;
    std::fs::write(path(root), format!("{raw}\n"))?;
    Ok(())
}

pub fn classify(
    last: Option<&PushState>,
    slug: &str,
    version: &str,
    sha256: &str,
    force: bool,
) -> ChangeCheck {
    if force {
        return ChangeCheck::Proceed;
    }
    let Some(last) = last else {
        return ChangeCheck::Proceed;
    };
    if last.slug != slug {
        return ChangeCheck::Proceed;
    }
    if last.version == version && last.sha256 == sha256 {
        return ChangeCheck::Unchanged;
    }
    if last.version == version && last.sha256 != sha256 {
        return ChangeCheck::VersionBumpRequired {
            last_version: last.version.clone(),
        };
    }
    ChangeCheck::Proceed
}

pub fn hash_developer(entry_url: &str, manifest: &serde_json::Value) -> String {
    let canonical = serde_json::to_string(manifest).unwrap_or_default();
    let mut buf = Vec::with_capacity(entry_url.len() + canonical.len() + 1);
    buf.extend_from_slice(entry_url.as_bytes());
    buf.push(b'\n');
    buf.extend_from_slice(canonical.as_bytes());
    bundle::sha256_hex(&buf)
}

pub fn now_rfc3339() -> String {
    Utc::now().to_rfc3339()
}

pub fn provider_str(p: Provider) -> String {
    p.as_str().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(version: &str, sha: &str) -> PushState {
        PushState {
            slug: "my-app".into(),
            version: version.into(),
            provider: "times".into(),
            sha256: sha.into(),
            file_count: 1,
            uploaded_at: "2026-01-01T00:00:00Z".into(),
        }
    }

    #[test]
    fn skip_when_hash_and_version_match() {
        let last = state("1.0.0", "abc");
        assert!(matches!(
            classify(Some(&last), "my-app", "1.0.0", "abc", false),
            ChangeCheck::Unchanged
        ));
    }

    #[test]
    fn refuse_same_version_new_files() {
        let last = state("1.0.0", "abc");
        assert!(matches!(
            classify(Some(&last), "my-app", "1.0.0", "def", false),
            ChangeCheck::VersionBumpRequired { .. }
        ));
    }

    #[test]
    fn new_version_always_proceeds() {
        let last = state("1.0.0", "abc");
        assert!(matches!(
            classify(Some(&last), "my-app", "1.0.1", "abc", false),
            ChangeCheck::Proceed
        ));
    }
}
