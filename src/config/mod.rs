use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::error::CliError;

pub fn validate_profile_name(name: &str) -> anyhow::Result<()> {
    let valid = !name.is_empty()
        && name.len() <= 40
        && name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !valid {
        return Err(CliError::usage(
            "Profile names use letters, digits, hyphens and underscores, up to 40 characters, and must start with a letter or digit.",
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Profile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub org_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub org_slug: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Config {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_profile: Option<String>,
    #[serde(default)]
    pub profiles: BTreeMap<String, Profile>,
}

impl Config {
    pub fn load(dir: &Path) -> anyhow::Result<Self> {
        let path = dir.join("config.toml");
        match fs::read_to_string(&path) {
            Ok(s) => {
                toml::from_str(&s).with_context(|| format!("Could not read {}", path.display()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("Could not read {}", path.display())),
        }
    }

    pub fn save(&self, dir: &Path) -> anyhow::Result<()> {
        fs::create_dir_all(dir).with_context(|| format!("Could not create {}", dir.display()))?;
        let path = dir.join("config.toml");
        write_private(&path, toml::to_string_pretty(self)?.as_bytes())
    }

    pub fn profile(&self, name: &str) -> Profile {
        self.profiles.get(name).cloned().unwrap_or_default()
    }

    pub fn profile_mut(&mut self, name: &str) -> &mut Profile {
        self.profiles.entry(name.to_string()).or_default()
    }

    pub fn current_profile_name(&self) -> String {
        self.current_profile
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("default")
            .to_string()
    }

    pub fn set_current(&mut self, name: &str) {
        self.current_profile = Some(name.to_string());
        self.profiles.entry(name.to_string()).or_default();
    }

    pub fn remove_profile(&mut self, name: &str) {
        self.profiles.remove(name);
        if self.current_profile.as_deref() == Some(name) {
            self.current_profile = self.profiles.keys().next().cloned();
        }
    }
}

pub fn config_dir() -> anyhow::Result<PathBuf> {
    if let Some(dir) = std::env::var_os("TIMES_CONFIG_DIR").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    if cfg!(windows) {
        let dirs = directories::ProjectDirs::from("et", "TIMES", "times")
            .context("Could not find a config directory")?;
        return Ok(dirs.config_dir().to_path_buf());
    }
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(xdg).join("times"));
    }
    let base = directories::BaseDirs::new().context("Could not find your home directory")?;
    Ok(base.home_dir().join(".config").join("times"))
}

pub fn write_private(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes).with_context(|| format!("Could not write {}", tmp.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600))?;
    }
    fs::rename(&tmp, path).with_context(|| format!("Could not write {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_is_default() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(Config::load(dir.path()).unwrap(), Config::default());
    }

    #[test]
    fn round_trip_profiles() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = Config::default();
        let p = cfg.profile_mut("default");
        p.org_slug = Some("acme".into());
        p.org_id = Some("0b8f".into());
        cfg.profile_mut("staging").api_url = Some("https://staging.example".into());
        cfg.save(dir.path()).unwrap();

        let loaded = Config::load(dir.path()).unwrap();
        assert_eq!(loaded, cfg);
        assert_eq!(loaded.profile("default").org_slug.as_deref(), Some("acme"));
        assert_eq!(loaded.profile("missing"), Profile::default());
    }

    #[test]
    fn current_profile_persists() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = Config::default();
        cfg.set_current("work");
        cfg.save(dir.path()).unwrap();
        let loaded = Config::load(dir.path()).unwrap();
        assert_eq!(loaded.current_profile_name(), "work");
        cfg.remove_profile("work");
        assert_eq!(cfg.current_profile, None);
        assert_eq!(cfg.current_profile_name(), "default");
    }

    #[test]
    fn profile_names_are_validated() {
        assert!(validate_profile_name("default").is_ok());
        assert!(validate_profile_name("work-1").is_ok());
        assert!(validate_profile_name("ci_bot").is_ok());
        assert!(validate_profile_name("").is_err());
        assert!(validate_profile_name("-bad").is_err());
        assert!(validate_profile_name("has space").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn config_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        Config::default().save(dir.path()).unwrap();
        let mode = fs::metadata(dir.path().join("config.toml"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
