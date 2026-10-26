use std::cell::RefCell;
use std::collections::BTreeSet;
use std::time::Duration;

use serde_json::Value;

use crate::api::{envelope, DEFAULT_API_URL};
use crate::config;

const SCOPES_PATH: &str = "/api/v1/apps/catalog/public/scopes";
const CACHE_FILE: &str = "scopes.json";

thread_local! {
    static OVERRIDE: RefCell<Option<BTreeSet<String>>> = const { RefCell::new(None) };
}

static CATALOG: std::sync::Mutex<Option<BTreeSet<String>>> = std::sync::Mutex::new(None);

pub fn allows(scope: &str) -> bool {
    let key = normalize(scope);
    if key.is_empty() {
        return false;
    }
    if let Some(set) = OVERRIDE.with(|slot| slot.borrow().clone()) {
        return set.contains(&key);
    }
    catalog().contains(&key)
}

pub fn seed() -> BTreeSet<String> {
    serde_json::from_str::<Vec<String>>(include_str!("scopes.json"))
        .expect("scopes.json")
        .into_iter()
        .map(|scope| normalize(&scope))
        .filter(|scope| !scope.is_empty())
        .collect()
}

#[cfg(test)]
pub fn install_for_test(scopes: impl IntoIterator<Item = impl AsRef<str>>) {
    let set = scopes
        .into_iter()
        .map(|scope| normalize(scope.as_ref()))
        .filter(|scope| !scope.is_empty())
        .collect();
    OVERRIDE.with(|slot| *slot.borrow_mut() = Some(set));
}

#[cfg(test)]
pub fn clear_test_override() {
    OVERRIDE.with(|slot| *slot.borrow_mut() = None);
}

/// Seed, then any scopes cached from an earlier fetch, then the live catalog.
/// A scope added on the gateway is allowed without a CLI release. A fetch
/// failure keeps the seed so offline validate still rejects unknown ids.
pub async fn refresh(api_base: &str) {
    let mut set = seed();
    if let Some(cached) = read_cache() {
        set.extend(cached);
    }
    if let Some(remote) = fetch(api_base).await {
        set.extend(remote);
        let _ = write_cache(&set);
    }
    if let Ok(mut guard) = CATALOG.lock() {
        *guard = Some(set);
    }
}

pub fn api_base_from_flag(flag: Option<&str>, profile: Option<&str>) -> String {
    if let Some(url) = flag.map(str::trim).filter(|url| !url.is_empty()) {
        return url.trim_end_matches('/').to_string();
    }
    if let Ok(dir) = config::config_dir() {
        if let Ok(cfg) = config::Config::load(&dir) {
            let name = profile
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| cfg.current_profile_name());
            if let Some(url) = cfg
                .profile(&name)
                .api_url
                .as_deref()
                .map(str::trim)
                .filter(|url| !url.is_empty())
            {
                return url.trim_end_matches('/').to_string();
            }
        }
    }
    DEFAULT_API_URL.to_string()
}

fn catalog() -> BTreeSet<String> {
    CATALOG
        .lock()
        .ok()
        .and_then(|guard| guard.clone())
        .unwrap_or_else(seed)
}

fn normalize(scope: &str) -> String {
    scope.trim().to_ascii_lowercase()
}

async fn fetch(api_base: &str) -> Option<BTreeSet<String>> {
    let base = api_base.trim().trim_end_matches('/');
    if base.is_empty() {
        return None;
    }
    let http = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(5))
        .user_agent(concat!("times-cli/", env!("CARGO_PKG_VERSION")))
        .build()
        .ok()?;
    let resp = http
        .get(format!("{base}{SCOPES_PATH}"))
        .header("Accept", "application/json")
        .send()
        .await
        .ok()?;
    let status = resp.status().as_u16();
    let text = resp.text().await.ok()?;
    let data = envelope::parse(status, &text).ok()?;
    parse_scopes(&data)
}

fn parse_scopes(data: &Value) -> Option<BTreeSet<String>> {
    let list = data
        .as_array()
        .or_else(|| data.get("scopes").and_then(Value::as_array))?;
    let set: BTreeSet<String> = list
        .iter()
        .filter_map(Value::as_str)
        .map(normalize)
        .filter(|scope| !scope.is_empty())
        .collect();
    if set.is_empty() {
        None
    } else {
        Some(set)
    }
}

fn cache_path() -> Option<std::path::PathBuf> {
    config::config_dir().ok().map(|dir| dir.join(CACHE_FILE))
}

fn read_cache() -> Option<BTreeSet<String>> {
    let raw = std::fs::read_to_string(cache_path()?).ok()?;
    parse_scopes(&serde_json::from_str(&raw).ok()?)
}

fn write_cache(set: &BTreeSet<String>) -> anyhow::Result<()> {
    let Some(path) = cache_path() else {
        return Ok(());
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let list: Vec<&str> = set.iter().map(String::as_str).collect();
    std::fs::write(path, serde_json::to_vec_pretty(&list)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_includes_device_scopes() {
        let set = seed();
        assert!(set.contains("device.clipboard:write"));
        assert!(set.contains("device.camera:use"));
        assert!(!set.contains("root:all"));
    }

    #[test]
    fn override_replaces_the_seed_on_this_thread() {
        install_for_test(["custom:read"]);
        assert!(allows("custom:read"));
        assert!(!allows("erp.company:read"));
        clear_test_override();
        assert!(allows("erp.company:read"));
        assert!(!allows("custom:read"));
    }
}
