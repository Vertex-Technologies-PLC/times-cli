use std::fs;
use std::path::PathBuf;
use std::process::Stdio;
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
const INTERNAL_CHECK_ENV: &str = "TIMES_INTERNAL_UPDATE_CHECK";

pub(crate) const CURRENT_TTL_SECS: i64 = 60 * 60;
pub(crate) const KNOWN_UPDATE_TTL_SECS: i64 = 24 * 60 * 60;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct Cache {
    pub checked_at: DateTime<Utc>,
    pub latest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub etag: Option<String>,
}

enum RemoteLatest {
    Fresh {
        tag: String,
        etag: Option<String>,
    },
    NotModified,
    Failed,
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

pub fn is_internal_check() -> bool {
    std::env::var_os(INTERNAL_CHECK_ENV).is_some()
}

fn skip_check() -> bool {
    std::env::var_os("TIMES_NO_UPDATE_CHECK").is_some() || std::env::var_os("CI").is_some()
}

fn cache_path() -> Option<PathBuf> {
    crate::config::config_dir()
        .ok()
        .map(|d| d.join("update-check.json"))
}

pub(crate) fn cache_age_secs(checked_at: DateTime<Utc>, now: DateTime<Utc>) -> i64 {
    now.signed_duration_since(checked_at).num_seconds()
}

pub(crate) fn should_refresh_cache(
    cache: Option<&Cache>,
    current: &Version,
    now: DateTime<Utc>,
) -> bool {
    let Some(cache) = cache else {
        return true;
    };
    let age = cache_age_secs(cache.checked_at, now);
    if age < 0 {
        return true;
    }
    let ttl = match cache.latest.as_deref().and_then(parse_tag) {
        Some(latest) if is_newer(&latest, current) => KNOWN_UPDATE_TTL_SECS,
        _ => CURRENT_TTL_SECS,
    };
    age >= ttl
}

pub(crate) fn cached_update_to_show(cache: Option<&Cache>, current: &Version) -> Option<Version> {
    let latest = cache?.latest.as_deref().and_then(parse_tag)?;
    is_newer(&latest, current).then_some(latest)
}

fn read_cache_file() -> Option<Cache> {
    let path = cache_path()?;
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

fn write_cache(latest: Option<&str>, etag: Option<&str>) {
    let Some(path) = cache_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let cache = Cache {
        checked_at: Utc::now(),
        latest: latest.map(str::to_string),
        etag: etag.map(str::to_string),
    };
    if let Ok(bytes) = serde_json::to_vec_pretty(&cache) {
        let _ = fs::write(path, bytes);
    }
}

async fn fetch_latest(etag: Option<&str>) -> RemoteLatest {
    let Some(http) = reqwest::Client::builder()
        .user_agent(concat!("times-cli/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(3))
        .build()
        .ok()
    else {
        return RemoteLatest::Failed;
    };
    let mut req = http
        .get(RELEASES_API)
        .header("Accept", "application/vnd.github+json");
    if let Some(etag) = etag.filter(|e| !e.is_empty()) {
        req = req.header("If-None-Match", etag);
    }
    let Ok(resp) = req.send().await else {
        return RemoteLatest::Failed;
    };
    let status = resp.status();
    if status.as_u16() == 304 {
        return RemoteLatest::NotModified;
    }
    if !status.is_success() {
        return RemoteLatest::Failed;
    }
    let etag = resp
        .headers()
        .get("etag")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let Ok(body) = resp.json::<serde_json::Value>().await else {
        return RemoteLatest::Failed;
    };
    match body.get("tag_name").and_then(|v| v.as_str()) {
        Some(tag) => RemoteLatest::Fresh {
            tag: tag.to_string(),
            etag,
        },
        None => RemoteLatest::Failed,
    }
}

fn apply_remote(cache: Option<&Cache>, remote: RemoteLatest) -> Option<Version> {
    match remote {
        RemoteLatest::Fresh { tag, etag } => {
            write_cache(Some(&tag), etag.as_deref());
            parse_tag(&tag)
        }
        RemoteLatest::NotModified => {
            let latest = cache.and_then(|c| c.latest.as_deref());
            let etag = cache.and_then(|c| c.etag.as_deref());
            write_cache(latest, etag);
            latest.and_then(parse_tag)
        }
        RemoteLatest::Failed => None,
    }
}

pub async fn refresh_from_network() {
    let cache = read_cache_file();
    let etag = cache.as_ref().and_then(|c| c.etag.as_deref());
    let remote = fetch_latest(etag).await;
    let _ = apply_remote(cache.as_ref(), remote);
}

pub async fn latest_version() -> Option<Version> {
    let cache = read_cache_file();
    let etag = cache.as_ref().and_then(|c| c.etag.as_deref());
    apply_remote(cache.as_ref(), fetch_latest(etag).await)
}

pub fn print_upgrade_hint(out: &Output, current: &Version, latest: &Version) {
    if out.json {
        return;
    }
    out.warn(format!("times {latest} is out. You have {current}."));
    out.info(format!("  Changelog: {CHANGELOG_URL}"));
    out.info("  Upgrade:   times upgrade");
}

fn spawn_background_refresh() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let mut cmd = std::process::Command::new(exe);
    cmd.env(INTERNAL_CHECK_ENV, "1")
        .env("TIMES_NO_UPDATE_CHECK", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        cmd.creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS);
    }
    let _ = cmd.spawn();
}

pub fn maybe_warn(out: &Output) {
    if out.json || skip_check() {
        return;
    }
    let current = current_version();
    let cache = read_cache_file();
    if let Some(latest) = cached_update_to_show(cache.as_ref(), &current) {
        print_upgrade_hint(out, &current, &latest);
    }
    if should_refresh_cache(cache.as_ref(), &current, Utc::now()) {
        spawn_background_refresh();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(secs, 0).unwrap()
    }

    fn cache(checked_at: DateTime<Utc>, latest: &str) -> Cache {
        Cache {
            checked_at,
            latest: Some(latest.into()),
            etag: None,
        }
    }

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

    #[test]
    fn shows_cached_newer_release_immediately() {
        let current = Version::new(0, 2, 0);
        let c = cache(at(1), "v0.4.0");
        assert_eq!(
            cached_update_to_show(Some(&c), &current),
            Some(Version::new(0, 4, 0))
        );
        assert!(cached_update_to_show(Some(&c), &Version::new(0, 4, 0)).is_none());
        assert!(cached_update_to_show(None, &current).is_none());
    }

    #[test]
    fn refreshes_when_cache_says_current_after_one_hour() {
        let current = Version::new(0, 2, 0);
        let checked = at(1_000_000);
        let c = cache(checked, "v0.2.0");
        assert!(!should_refresh_cache(
            Some(&c),
            &current,
            checked + chrono::Duration::minutes(10)
        ));
        assert!(should_refresh_cache(
            Some(&c),
            &current,
            checked + chrono::Duration::hours(1)
        ));
        assert!(should_refresh_cache(
            Some(&c),
            &current,
            checked + chrono::Duration::hours(8)
        ));
    }

    #[test]
    fn known_update_only_rechecks_daily() {
        let current = Version::new(0, 2, 0);
        let checked = at(1_000_000);
        let c = cache(checked, "v0.4.0");
        assert!(!should_refresh_cache(
            Some(&c),
            &current,
            checked + chrono::Duration::hours(8)
        ));
        assert!(should_refresh_cache(
            Some(&c),
            &current,
            checked + chrono::Duration::hours(24)
        ));
    }

    #[test]
    fn missing_or_skewed_cache_refreshes() {
        let current = Version::new(0, 2, 0);
        assert!(should_refresh_cache(None, &current, at(1)));
        let future = cache(at(2_000), "v0.2.0");
        assert!(should_refresh_cache(Some(&future), &current, at(1_000)));
    }
}
