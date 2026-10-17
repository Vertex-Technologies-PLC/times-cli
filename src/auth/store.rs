use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::config::write_private;

#[cfg(any(target_os = "macos", target_os = "windows"))]
const SERVICE: &str = "times-cli";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Credentials {
    Session {
        access_token: String,
        access_token_expires_at: DateTime<Utc>,
        refresh_token: String,
        refresh_token_expires_at: DateTime<Utc>,
        #[serde(default)]
        org_id: Option<String>,
    },
    Token {
        token: String,
    },
}

pub struct Store {
    dir: PathBuf,
    use_keyring: bool,
}

fn use_keyring_pref(val: Option<&str>) -> bool {
    cfg!(any(target_os = "macos", target_os = "windows"))
        && val.is_some_and(|v| v.eq_ignore_ascii_case("keyring"))
}

impl Store {
    pub fn new(dir: &Path) -> Self {
        Self {
            dir: dir.to_path_buf(),
            use_keyring: use_keyring_pref(std::env::var("TIMES_CREDENTIAL_STORE").ok().as_deref()),
        }
    }

    fn file(&self) -> PathBuf {
        self.dir.join("credentials.json")
    }

    fn read_file(&self) -> BTreeMap<String, Credentials> {
        fs::read_to_string(self.file())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    fn write_file(&self, map: &BTreeMap<String, Credentials>) -> anyhow::Result<()> {
        if map.is_empty() {
            let _ = fs::remove_file(self.file());
            return Ok(());
        }
        fs::create_dir_all(&self.dir)?;
        write_private(&self.file(), serde_json::to_string_pretty(map)?.as_bytes())
    }

    fn write_profile_file(&self, profile: &str, creds: &Credentials) -> anyhow::Result<()> {
        let mut map = self.read_file();
        map.insert(profile.to_string(), creds.clone());
        self.write_file(&map)
    }

    fn remove_profile_file(&self, profile: &str) -> anyhow::Result<()> {
        let mut map = self.read_file();
        if map.remove(profile).is_some() {
            self.write_file(&map)?;
        }
        Ok(())
    }

    pub fn load(&self, profile: &str) -> Option<Credentials> {
        if self.use_keyring {
            if let Some(c) = keyring_get(profile) {
                return Some(c);
            }
            return self.read_file().remove(profile);
        }
        if let Some(c) = self.read_file().remove(profile) {
            return Some(c);
        }
        let c = keyring_get(profile)?;
        let _ = self.write_profile_file(profile, &c);
        Some(c)
    }

    pub fn save(&self, profile: &str, creds: &Credentials) -> anyhow::Result<()> {
        if self.use_keyring && keyring_set(profile, creds) {
            return self.remove_profile_file(profile);
        }
        self.write_profile_file(profile, creds)
    }

    pub fn clear(&self, profile: &str) -> anyhow::Result<()> {
        keyring_delete(profile);
        self.remove_profile_file(profile)
    }

    pub fn names(&self) -> Vec<String> {
        self.read_file().into_keys().collect()
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn keyring_get(profile: &str) -> Option<Credentials> {
    let entry = keyring::Entry::new(SERVICE, profile).ok()?;
    let secret = entry.get_password().ok()?;
    serde_json::from_str(&secret).ok()
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn keyring_set(profile: &str, creds: &Credentials) -> bool {
    let Ok(secret) = serde_json::to_string(creds) else {
        return false;
    };
    keyring::Entry::new(SERVICE, profile)
        .and_then(|e| e.set_password(&secret))
        .is_ok()
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn keyring_delete(profile: &str) {
    if let Ok(e) = keyring::Entry::new(SERVICE, profile) {
        let _ = e.delete_credential();
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn keyring_get(_profile: &str) -> Option<Credentials> {
    None
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn keyring_set(_profile: &str, _creds: &Credentials) -> bool {
    false
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn keyring_delete(_profile: &str) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn file_store(dir: &Path) -> Store {
        Store {
            dir: dir.to_path_buf(),
            use_keyring: false,
        }
    }

    #[test]
    fn keyring_only_when_requested() {
        assert!(!use_keyring_pref(None));
        assert!(!use_keyring_pref(Some("file")));
        assert!(!use_keyring_pref(Some("FILE")));
        assert_eq!(
            use_keyring_pref(Some("keyring")),
            cfg!(any(target_os = "macos", target_os = "windows"))
        );
        assert_eq!(
            use_keyring_pref(Some("KEYRING")),
            cfg!(any(target_os = "macos", target_os = "windows"))
        );
    }

    #[test]
    fn default_store_uses_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store {
            dir: dir.path().to_path_buf(),
            use_keyring: use_keyring_pref(None),
        };
        assert!(!store.use_keyring);
        let tok = Credentials::Token {
            token: "tdk_file".into(),
        };
        store.save("default", &tok).unwrap();
        assert_eq!(store.load("default"), Some(tok));
        assert!(dir.path().join("credentials.json").exists());
    }

    #[test]
    fn file_store_round_trip_per_profile() {
        let dir = tempfile::tempdir().unwrap();
        let store = file_store(dir.path());
        assert!(store.load("default").is_none());

        let tok = Credentials::Token {
            token: "tdk_abc".into(),
        };
        store.save("default", &tok).unwrap();
        store
            .save(
                "ci",
                &Credentials::Token {
                    token: "tdk_ci".into(),
                },
            )
            .unwrap();
        assert_eq!(store.load("default"), Some(tok));

        store.clear("default").unwrap();
        assert!(store.load("default").is_none());
        assert!(store.load("ci").is_some());
        let mut names = store.names();
        names.sort();
        assert_eq!(names, vec!["ci".to_string()]);
        store.clear("ci").unwrap();
        assert!(!dir.path().join("credentials.json").exists());
    }

    #[test]
    fn session_serializes_with_kind_tag() {
        let c = Credentials::Session {
            access_token: "a".into(),
            access_token_expires_at: Utc::now(),
            refresh_token: "r".into(),
            refresh_token_expires_at: Utc::now(),
            org_id: None,
        };
        let v = serde_json::to_value(&c).unwrap();
        assert_eq!(v["kind"], "session");
        let back: Credentials = serde_json::from_value(v).unwrap();
        assert_eq!(back, c);
    }
}
