use serde_json::Value;

use crate::error::CliError;
use crate::manifest::{self, str_at};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Times,
    Developer,
}

impl Provider {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Times => "times",
            Self::Developer => "developer",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Hosting {
    pub provider: Provider,
    pub entry_url: Option<String>,
}

impl Hosting {
    pub fn needs_bundle(&self, doc: &Value) -> bool {
        if self.provider != Provider::Times {
            return false;
        }
        match str_at(doc, "/ui/entry") {
            Some(entry) if !is_https_url(entry) => true,
            _ => false,
        }
    }
}

pub fn resolve(doc: &Value) -> anyhow::Result<Hosting> {
    if let Some(obj) = doc.get("hosting").and_then(|v| v.as_object()) {
        let provider = match obj.get("provider").and_then(|v| v.as_str()) {
            Some("developer") => Provider::Developer,
            Some("times") => Provider::Times,
            Some(other) => {
                return Err(CliError::validation(format!(
                    "hosting.provider must be times or developer, not {other}"
                )));
            }
            None => {
                return Err(CliError::validation(
                    "hosting.provider is required when hosting is set",
                ));
            }
        };
        let entry_url = obj
            .get("entryUrl")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned);
        return validate(Hosting {
            provider,
            entry_url,
        });
    }
    infer(doc)
}

fn infer(doc: &Value) -> anyhow::Result<Hosting> {
    match str_at(doc, "/ui/entry") {
        Some(url) if is_https_url(url) => validate(Hosting {
            provider: Provider::Developer,
            entry_url: Some(url.to_string()),
        }),
        _ => Ok(Hosting {
            provider: Provider::Times,
            entry_url: None,
        }),
    }
}

fn validate(hosting: Hosting) -> anyhow::Result<Hosting> {
    match hosting.provider {
        Provider::Developer => {
            let Some(url) = hosting.entry_url.as_deref() else {
                return Err(CliError::validation(
                    "hosting.entryUrl is required when hosting.provider is developer",
                ));
            };
            if !manifest::is_public_https(url) {
                return Err(CliError::validation(
                    "hosting.entryUrl must be a public https:// URL (no localhost, no private IPs)",
                ));
            }
        }
        Provider::Times => {
            if let Some(url) = hosting.entry_url.as_deref() {
                if !url.is_empty() {
                    return Err(CliError::validation(
                        "hosting.entryUrl must be empty when TIMES hosts the guest. Use a relative ui.entry.",
                    ));
                }
            }
        }
    }
    Ok(hosting)
}

pub fn is_https_url(value: &str) -> bool {
    value.starts_with("https://")
}

pub fn output_dir_missing(root: &std::path::Path, doc: &Value) -> bool {
    let Some(entry) = str_at(doc, "/ui/entry") else {
        return false;
    };
    if is_https_url(entry) {
        return false;
    }
    match crate::bundle::build_dir_for(doc) {
        Some(dir) => !root.join(&dir).is_dir(),
        None => !manifest::relative(root, entry).is_file(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn infers_times_from_relative_entry() {
        let h = resolve(&json!({"ui": {"entry": "./dist/index.html"}})).unwrap();
        assert_eq!(h.provider, Provider::Times);
        assert!(h.needs_bundle(&json!({"ui": {"entry": "./dist/index.html"}})));
    }

    #[test]
    fn infers_developer_from_https_entry() {
        let h = resolve(&json!({"ui": {"entry": "https://app.example.com/"}})).unwrap();
        assert_eq!(h.provider, Provider::Developer);
        assert_eq!(h.entry_url.as_deref(), Some("https://app.example.com/"));
        assert!(!h.needs_bundle(&json!({"ui": {"entry": "https://app.example.com/"}})));
    }

    #[test]
    fn developer_requires_public_https() {
        let err = resolve(&json!({
            "hosting": { "provider": "developer", "entryUrl": "https://localhost/" }
        }))
        .unwrap_err();
        assert!(err.to_string().contains("public https"));
    }
}
