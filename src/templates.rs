use std::io::{Cursor, Read};
use std::time::Duration;

use zip::ZipArchive;

use crate::cli::Framework;
use crate::error::CliError;

const DEFAULT_TEMPLATE_ZIP: &str =
    "https://github.com/Vertex-Technologies-PLC/times-aip-templates/archive/refs/heads/main.zip";

pub async fn fetch_framework_files(
    framework: Framework,
) -> anyhow::Result<Option<Vec<(String, String)>>> {
    let url = std::env::var("TIMES_TEMPLATES_URL").unwrap_or_else(|_| DEFAULT_TEMPLATE_ZIP.into());
    let client = reqwest::Client::builder()
        .user_agent(concat!("times-cli/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(60))
        .build()?;
    let resp = match client.get(&url).send().await {
        Ok(r) => r,
        Err(_) => return Ok(None),
    };
    if !resp.status().is_success() {
        return Ok(None);
    }
    let bytes = resp.bytes().await?;
    let folder = framework.as_str();
    let files = extract_framework_dir(&bytes, folder)?;
    if files.is_empty() {
        return Ok(None);
    }
    Ok(Some(files))
}

fn extract_framework_dir(bytes: &[u8], folder: &str) -> anyhow::Result<Vec<(String, String)>> {
    let mut zip = ZipArchive::new(Cursor::new(bytes)).map_err(|e| {
        CliError::network(format!("Could not read the AIP templates archive: {e}"))
    })?;
    let needle = format!("/{folder}/");
    let mut out = Vec::new();
    for i in 0..zip.len() {
        let mut file = zip.by_index(i).map_err(|e| CliError::network(e.to_string()))?;
        if file.is_dir() {
            continue;
        }
        let name = file.name().replace('\\', "/");
        let Some(idx) = name.find(&needle) else {
            continue;
        };
        let rel = &name[idx + needle.len()..];
        if rel.is_empty() || rel.starts_with('.') {
            continue;
        }
        let mut buf = String::new();
        if file.read_to_string(&mut buf).is_err() {
            continue;
        }
        out.push((rel.to_string(), buf));
    }
    Ok(out)
}

pub fn apply_placeholders(content: &str, slug: &str, name: &str) -> String {
    content
        .replace("__TIMES_SLUG__", slug)
        .replace("__TIMES_NAME__", name)
        .replace("{{slug}}", slug)
        .replace("{{name}}", name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholders_replace() {
        let s = apply_placeholders("hello __TIMES_NAME__ (__TIMES_SLUG__)", "my-app", "My App");
        assert_eq!(s, "hello My App (my-app)");
    }
}
