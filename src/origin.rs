use std::time::Duration;

use reqwest::header::{CONTENT_TYPE, HeaderMap};
use reqwest::redirect::Policy;

use crate::error::CliError;
use crate::netutil;

const APP_ORIGIN: &str = "https://app.times.et";
const MAX_ORIGIN_BODY: usize = 1_048_576;

#[derive(Debug)]
pub struct OriginReport {
    pub url: String,
    pub status: u16,
    pub content_type: String,
    pub frame_ancestors_ok: bool,
    pub x_frame_options: Option<String>,
}

pub async fn probe(url: &str) -> anyhow::Result<OriginReport> {
    let target = netutil::validate_probe_url(url)?;
    let http = reqwest::Client::builder()
        .redirect(Policy::none())
        .timeout(Duration::from_secs(10))
        .connect_timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| CliError::network(format!("origin client failed: {e}")))?;
    let resp = http
        .get(target.clone())
        .header("user-agent", "times-cli origin-check")
        .send()
        .await
        .map_err(|e| CliError::network(format!("origin probe failed for {url}: {e}")))?;
    let status = resp.status().as_u16();
    let headers = resp.headers().clone();
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| CliError::network(format!("origin body could not be read: {e}")))?;
    let body = if bytes.len() > MAX_ORIGIN_BODY {
        String::from_utf8_lossy(&bytes[..MAX_ORIGIN_BODY]).into_owned()
    } else {
        String::from_utf8_lossy(&bytes).into_owned()
    };
    evaluate_origin_response(url, status, &headers, &body)
}

pub fn evaluate_origin_response(
    url: &str,
    status: u16,
    headers: &HeaderMap,
    body: &str,
) -> anyhow::Result<OriginReport> {
    let content_type = header_str(headers, CONTENT_TYPE.as_str()).unwrap_or_default();
    let csp = header_str(headers, "content-security-policy");
    let xfo = header_str(headers, "x-frame-options");
    let mut errors = Vec::new();
    if (300..400).contains(&status) {
        let location = header_str(headers, "location").unwrap_or_default();
        errors.push(format!(
            "origin must not redirect (HTTP {status} Location {location})"
        ));
    } else if status != 200 {
        errors.push(format!("expected HTTP 200, got {status}"));
    }
    if !content_type.to_ascii_lowercase().contains("html") && !looks_like_html(body) {
        errors.push(format!(
            "expected HTML, got content-type {content_type:?}"
        ));
    }
    if let Some(xfo) = xfo.as_deref() {
        let v = xfo.trim().to_ascii_uppercase();
        if v == "DENY" || v == "SAMEORIGIN" {
            errors.push(format!(
                "X-Frame-Options: {xfo} blocks embedding in app.times.et"
            ));
        }
    }
    let frame_ok = csp
        .as_deref()
        .map(frame_ancestors_allows_app)
        .unwrap_or(false);
    if !frame_ok {
        errors.push(format!(
            "Content-Security-Policy frame-ancestors must include {APP_ORIGIN} (got {csp:?})"
        ));
    }
    if !errors.is_empty() {
        return Err(CliError::validation(format!(
            "Origin {url} cannot be embedded:\n  - {}",
            errors.join("\n  - ")
        )));
    }
    Ok(OriginReport {
        url: url.to_string(),
        status,
        content_type,
        frame_ancestors_ok: true,
        x_frame_options: xfo,
    })
}

pub fn frame_ancestors_allows_app(csp: &str) -> bool {
    let lower = csp.to_ascii_lowercase();
    let Some(idx) = lower.find("frame-ancestors") else {
        return false;
    };
    let rest = lower[idx + "frame-ancestors".len()..].trim_start();
    let directive = rest.split(';').next().unwrap_or(rest);
    directive.split_whitespace().any(|src| {
        src == "*"
            || src == "https:"
            || src == "https://app.times.et"
            || src == "https://app.times.et/"
            || src == "https://*.times.et"
    })
}

fn header_str(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
}

fn looks_like_html(body: &str) -> bool {
    let t = body.trim_start().to_ascii_lowercase();
    t.starts_with("<!doctype html") || t.starts_with("<html")
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::{HeaderName, HeaderValue};

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (k, v) in pairs {
            map.append(
                HeaderName::from_bytes(k.as_bytes()).unwrap(),
                HeaderValue::from_str(v).unwrap(),
            );
        }
        map
    }

    #[test]
    fn accepts_embeddable_html() {
        let report = evaluate_origin_response(
            "https://app.example.com/",
            200,
            &headers(&[
                ("content-type", "text/html; charset=utf-8"),
                (
                    "content-security-policy",
                    "frame-ancestors https://app.times.et",
                ),
            ]),
            "<!doctype html><html></html>",
        )
        .unwrap();
        assert!(report.frame_ancestors_ok);
    }

    #[test]
    fn rejects_redirect_and_xfo_and_missing_csp() {
        let err = evaluate_origin_response(
            "https://app.example.com/",
            302,
            &headers(&[
                ("location", "https://evil.example/"),
                ("x-frame-options", "DENY"),
            ]),
            "",
        )
        .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("must not redirect"));
        assert!(msg.contains("X-Frame-Options"));
        assert!(msg.contains("frame-ancestors"));
    }
}
