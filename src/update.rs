use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use chrono::{DateTime, Utc};
use semver::Version;
use serde::{Deserialize, Serialize};

use crate::ui::Output;

pub const CHANGELOG_URL: &str = "https://developers.times.et/docs/cli-changelog";
pub const INSTALL_SH: &str = "https://developers.times.et/install.sh";
pub const INSTALL_PS1: &str = "https://developers.times.et/install.ps1";
const RELEASES_API: &str =
    "https://api.github.com/repos/Vertex-Technologies-PLC/times-cli/releases/latest";
const CACHE_TTL_SECS: i64 = 24 * 60 * 60;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Cache {
    checked_at: DateTime<Utc>,
    latest: Option<String>,
}

pub fn current_version() -> Version {
    Version::parse(env!("CARGO_PKG_VERSION")).unwrap_or_else(|_| Version::new(0, 0, 0))
}

pub fn parse_tag(tag: &str) -> Option<Version> {
    let t = tag.trim().trim_start_matches('v');
    Version::parse(t).ok()
}

pub fn is_newer(latest: &Version, current: &Version) -> bool {
    latest > current
}

fn skip_check() -> bool {
    std::env::var_os("TIMES_NO_UPDATE_CHECK").is_some() || std::env::var_os("CI").is_some()
}

fn cache_path() -> Option<PathBuf> {
    crate::config::config_dir()
        .ok()
        .map(|d| d.join("update-check.json"))
}

fn read_cache() -> Option<Cache> {
    let path = cache_path()?;
    let raw = fs::read_to_string(path).ok()?;
    let cache: Cache = serde_json::from_str(&raw).ok()?;
    let age = Utc::now().signed_duration_since(cache.checked_at);
    if age.num_seconds() > CACHE_TTL_SECS {
        return None;
    }
    Some(cache)
}

fn write_cache(latest: Option<&str>) {
    let Some(path) = cache_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let cache = Cache {
        checked_at: Utc::now(),
        latest: latest.map(str::to_string),
    };
    if let Ok(bytes) = serde_json::to_vec_pretty(&cache) {
        let _ = fs::write(path, bytes);
    }
}

async fn fetch_latest_tag() -> Option<String> {
    let http = reqwest::Client::builder()
        .user_agent(concat!("times-cli/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(3))
        .build()
        .ok()?;
    let resp = http
        .get(RELEASES_API)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let body: serde_json::Value = resp.json().await.ok()?;
    body.get("tag_name")?.as_str().map(str::to_string)
}

pub async fn latest_version() -> Option<Version> {
    if let Some(cache) = read_cache() {
        return cache.latest.as_deref().and_then(parse_tag);
    }
    let tag = fetch_latest_tag().await;
    write_cache(tag.as_deref());
    tag.as_deref().and_then(parse_tag)
}

pub fn print_upgrade_hint(out: &Output, current: &Version, latest: &Version) {
    if out.json {
        return;
    }
    out.warn(format!("times {latest} is out. You have {current}."));
    out.info(format!("  Changelog: {CHANGELOG_URL}"));
    out.info("  Upgrade:   times upgrade");
}

pub async fn maybe_warn(out: &Output) {
    if out.json || skip_check() {
        return;
    }
    let current = current_version();
    let Some(latest) = latest_version().await else {
        return;
    };
    if is_newer(&latest, &current) {
        print_upgrade_hint(out, &current, &latest);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_v_prefix() {
        assert_eq!(parse_tag("v1.2.3").unwrap(), Version::new(1, 2, 3));
        assert_eq!(parse_tag("1.2.3").unwrap(), Version::new(1, 2, 3));
        assert!(parse_tag("nope").is_none());
    }

    #[test]
    fn newer_is_semver() {
        assert!(is_newer(&Version::new(0, 1, 1), &Version::new(0, 1, 0)));
        assert!(!is_newer(&Version::new(0, 1, 0), &Version::new(0, 1, 0)));
        assert!(!is_newer(&Version::new(0, 1, 0), &Version::new(0, 2, 0)));
    }
}
