use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;
use walkdir::WalkDir;

use crate::error::CliError;
use crate::hosting::{self, Provider};
use crate::manifest;

const SOURCE_ROOTS: &[&str] = &["src", "app", "public", "pages"];
const ALLOWED_GUEST_EXT: &[&str] = &[
    "html",
    "js",
    "mjs",
    "cjs",
    "css",
    "map",
    "svg",
    "json",
    "woff",
    "woff2",
    "png",
    "jpg",
    "jpeg",
    "webp",
    "ico",
    "txt",
    "webmanifest",
];
const DENIED_NAMES: &[&str] = &[
    ".env",
    ".env.local",
    ".env.production",
    "package-lock.json",
    "bun.lock",
    "bun.lockb",
    "pnpm-lock.yaml",
    "yarn.lock",
];
const DENIED_EXT: &[&str] = &[
    "wasm", "exe", "dll", "so", "dylib", "bin", "pem", "key", "p12", "pfx", "env",
];

#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    pub path: String,
    pub rule: String,
    pub message: String,
}

pub fn run(root: &Path, packed: &[(String, PathBuf)], hosting: &hosting::Hosting) -> Vec<Finding> {
    let mut findings = Vec::new();
    scan_manifest(root, &mut findings);
    scan_source_trees(root, &mut findings);
    if hosting.provider == Provider::Times {
        scan_packed_files(packed, &mut findings);
    }
    findings
}

pub fn print_table(findings: &[Finding]) {
    if findings.is_empty() {
        println!("Scan passed. Human review is still required before publish.");
        return;
    }
    println!("{:<10} {:<32} {}", "RULE", "PATH", "MESSAGE");
    for f in findings {
        println!("{:<10} {:<32} {}", f.rule, truncate(&f.path, 32), f.message);
    }
}

pub fn fail_if_any(findings: Vec<Finding>) -> anyhow::Result<()> {
    if findings.is_empty() {
        return Ok(());
    }
    print_table(&findings);
    Err(CliError::validation(format!(
        "Scan failed with {} finding(s). TIMES will not upload until every finding is fixed.",
        findings.len()
    )))
}

fn scan_manifest(root: &Path, findings: &mut Vec<Finding>) {
    let path = root.join("times.aip.json");
    let Ok(raw) = fs::read_to_string(&path) else {
        findings.push(finding(
            "times.aip.json",
            "MANIFEST",
            "times.aip.json is missing or unreadable",
        ));
        return;
    };
    match serde_json::from_str::<Value>(&raw) {
        Ok(doc) => {
            let report = manifest::validate(&doc, None);
            for issue in report.errors {
                findings.push(finding("times.aip.json", "MANIFEST", issue.message));
            }
        }
        Err(e) => findings.push(finding(
            "times.aip.json",
            "MANIFEST",
            format!("JSON parse failed: {e}"),
        )),
    }
}

fn scan_source_trees(root: &Path, findings: &mut Vec<Finding>) {
    let mut seen = BTreeSet::new();
    for name in SOURCE_ROOTS {
        let dir = root.join(name);
        if !dir.is_dir() {
            continue;
        }
        for entry in WalkDir::new(&dir).into_iter().filter_map(|e| e.ok()) {
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            if skip_source(root, path) {
                continue;
            }
            let rel = rel_str(root, path);
            if !seen.insert(rel.clone()) {
                continue;
            }
            scan_file(path, &rel, findings, true);
        }
    }
}

fn scan_packed_files(packed: &[(String, PathBuf)], findings: &mut Vec<Finding>) {
    for (archive_name, path) in packed {
        if archive_name.contains("..") || Path::new(archive_name).is_absolute() {
            findings.push(finding(
                archive_name,
                "PATH",
                "zip entries must be relative and must not contain ..",
            ));
            continue;
        }
        if path.is_symlink() {
            findings.push(finding(
                archive_name,
                "SYMLINK",
                "symlinks are not allowed in a TIMES-hosted guest zip",
            ));
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            findings.push(finding(
                archive_name,
                "PATH",
                "file name is not valid UTF-8",
            ));
            continue;
        };
        if DENIED_NAMES.iter().any(|n| name.eq_ignore_ascii_case(n)) {
            findings.push(finding(
                archive_name,
                "DENY",
                format!("{name} is not allowed in a TIMES-hosted guest zip"),
            ));
            continue;
        }
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if DENIED_EXT.contains(&ext.as_str()) {
            findings.push(finding(
                archive_name,
                "DENY",
                format!(".{ext} files are not allowed in a TIMES-hosted guest zip"),
            ));
            continue;
        }
        if !ext.is_empty() && !ALLOWED_GUEST_EXT.contains(&ext.as_str()) && name != "times.aip.json"
        {
            findings.push(finding(
                archive_name,
                "ALLOW",
                format!(".{ext} is not on the TIMES-hosted guest allowlist"),
            ));
            continue;
        }
        scan_file(path, archive_name, findings, false);
    }
}

fn scan_file(path: &Path, rel: &str, findings: &mut Vec<Finding>, source_heuristics: bool) {
    let Ok(bytes) = fs::read(path) else {
        findings.push(finding(
            rel,
            "READ",
            "file could not be read; scan is fail-closed",
        ));
        return;
    };
    if !source_heuristics {
        if let Some(kind) = denied_magic(&bytes) {
            findings.push(finding(
                rel,
                "MAGIC",
                format!("{kind} binaries are not allowed in a TIMES-hosted guest zip"),
            ));
            return;
        }
    }
    if looks_binary(&bytes) {
        return;
    }
    let text = String::from_utf8_lossy(&bytes);
    scan_secrets(rel, &text, findings);
    let lower_rel = rel.to_ascii_lowercase();
    let html = lower_rel.ends_with(".html") || lower_rel.ends_with(".htm");
    if html {
        scan_packed_html(rel, &text, findings);
    }
    if source_heuristics {
        scan_js_heuristics(rel, &text, findings);
    }
}

fn denied_magic(bytes: &[u8]) -> Option<&'static str> {
    if bytes.len() >= 2 && bytes[0] == b'M' && bytes[1] == b'Z' {
        return Some("Windows PE");
    }
    if bytes.len() >= 4 && bytes.starts_with(&[0x7f, b'E', b'L', b'F']) {
        return Some("ELF");
    }
    if bytes.len() >= 4
        && (bytes.starts_with(&[0xcf, 0xfa, 0xed, 0xfe])
            || bytes.starts_with(&[0xfe, 0xed, 0xfa, 0xcf])
            || bytes.starts_with(&[0xca, 0xfe, 0xba, 0xbe])
            || bytes.starts_with(&[0xce, 0xfa, 0xed, 0xfe])
            || bytes.starts_with(&[0xfe, 0xed, 0xfa, 0xce]))
    {
        return Some("Mach-O");
    }
    if bytes.len() >= 4 && bytes.starts_with(&[0x00, 0x61, 0x73, 0x6d]) {
        return Some("WebAssembly");
    }
    None
}

fn scan_js_heuristics(rel: &str, text: &str, findings: &mut Vec<Finding>) {
    let lower_rel = rel.to_ascii_lowercase();
    let js_like = lower_rel.ends_with(".js")
        || lower_rel.ends_with(".mjs")
        || lower_rel.ends_with(".cjs")
        || lower_rel.ends_with(".ts")
        || lower_rel.ends_with(".tsx")
        || lower_rel.ends_with(".jsx")
        || lower_rel.ends_with(".html")
        || lower_rel.ends_with(".htm");
    if !js_like {
        return;
    }
    let code = js_code_only(text);
    if code.contains("eval(") {
        findings.push(unsafe_js(rel, "EVAL", "eval("));
    }
    if code.contains("new Function") {
        findings.push(unsafe_js(rel, "FUNCTION", "new Function"));
    }
    if code.contains("document.write") {
        findings.push(unsafe_js(rel, "DOCWRITE", "document.write"));
    }
    if code.contains("javascript:") {
        findings.push(unsafe_js(rel, "JAVASCRIPT", "javascript: URL"));
    }
    if code.contains("innerHTML") || code.contains("dangerouslySetInnerHTML") {
        findings.push(unsafe_js(
            rel,
            "INNERHTML",
            "innerHTML / dangerouslySetInnerHTML",
        ));
    }
}

fn js_code_only(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            i += 2;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if c == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(bytes.len());
            continue;
        }
        if c == b'\'' || c == b'"' {
            i = skip_quoted(bytes, i);
            out.push(' ');
            continue;
        }
        if c == b'`' {
            i = skip_template(bytes, i, &mut out);
            continue;
        }
        out.push(c as char);
        i += 1;
    }
    out
}

fn skip_quoted(bytes: &[u8], start: usize) -> usize {
    let quote = bytes[start];
    let mut i = start + 1;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            i += 2;
            continue;
        }
        if bytes[i] == quote {
            return i + 1;
        }
        i += 1;
    }
    bytes.len()
}

fn skip_template(bytes: &[u8], start: usize, out: &mut String) -> usize {
    let mut i = start + 1;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            i += 2;
            continue;
        }
        if bytes[i] == b'`' {
            return i + 1;
        }
        if bytes[i] == b'$' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
            i += 2;
            let mut depth = 1;
            let expr_start = i;
            while i < bytes.len() && depth > 0 {
                if bytes[i] == b'{' {
                    depth += 1;
                    i += 1;
                    continue;
                }
                if bytes[i] == b'}' {
                    depth -= 1;
                    if depth == 0 {
                        out.push_str(&js_code_only(
                            std::str::from_utf8(&bytes[expr_start..i]).unwrap_or(""),
                        ));
                        i += 1;
                        break;
                    }
                    i += 1;
                    continue;
                }
                if bytes[i] == b'\'' || bytes[i] == b'"' || bytes[i] == b'`' {
                    if bytes[i] == b'`' {
                        i = skip_template(bytes, i, out);
                    } else {
                        i = skip_quoted(bytes, i);
                    }
                    continue;
                }
                i += 1;
            }
            continue;
        }
        i += 1;
    }
    bytes.len()
}

fn scan_secrets(rel: &str, text: &str, findings: &mut Vec<Finding>) {
    if text.contains("AKIA") {
        for cap in find_akia(text) {
            if cap.len() >= 20 {
                findings.push(secret(rel, "AWSKEY", "AWS access key id"));
                break;
            }
        }
    }
    if text.contains("BEGIN ") && text.contains("PRIVATE KEY") {
        findings.push(secret(rel, "PRIVKEY", "private key material"));
    }
    for (prefix, rule, label) in [
        ("tdk_", "TDK", "TIMES developer key prefix tdk_"),
        ("ghp_", "GITHUB", "GitHub token prefix ghp_"),
        ("github_pat_", "GITHUB", "GitHub token prefix github_pat_"),
        ("sk_live_", "STRIPE", "Stripe live secret prefix sk_live_"),
        ("xoxb-", "SLACK", "Slack bot token prefix xoxb-"),
        ("xoxp-", "SLACK", "Slack token prefix xoxp-"),
    ] {
        if text.contains(prefix) {
            findings.push(secret(rel, rule, label));
        }
    }
    if looks_jwt(text) {
        findings.push(secret(rel, "JWT", "JWT-shaped token"));
    }
    if looks_connection_string(text) {
        findings.push(secret(rel, "CONN", "database or broker connection string"));
    }
}

fn scan_packed_html(rel: &str, text: &str, findings: &mut Vec<Finding>) {
    let lower = text.to_ascii_lowercase();
    scan_remote_guest_tags(rel, &lower, findings);
    if lower.contains("javascript:") {
        findings.push(unsafe_js(rel, "JAVASCRIPT", "javascript: URL"));
    }
    if has_html_event_handler(&lower) {
        findings.push(finding(
            rel,
            "ONHANDLER",
            "inline HTML event handlers are not allowed",
        ));
    }
}

fn scan_remote_guest_tags(rel: &str, html: &str, findings: &mut Vec<Finding>) {
    for (tag, attr) in [
        ("<script", "src"),
        ("<iframe", "src"),
        ("<embed", "src"),
        ("<object", "data"),
        ("<object", "src"),
    ] {
        let mut search = html;
        while let Some(idx) = search.find(tag) {
            let rest = &search[idx..];
            let end = rest.find('>').unwrap_or(rest.len());
            let open = &rest[..end];
            if let Some(src) = html_attr(open, attr) {
                if attr_is_remote(&src) {
                    findings.push(finding(
                        rel,
                        "SCRIPT",
                        format!("remote {tag} {attr}=\"{src}\" is not allowed"),
                    ));
                }
            }
            search = if end < rest.len() {
                &rest[end + 1..]
            } else {
                ""
            };
        }
    }
}

fn html_attr(tag: &str, attr: &str) -> Option<String> {
    let needle = format!("{attr}=");
    let idx = tag.find(&needle)?;
    let after = tag[idx + needle.len()..].trim_start();
    let quote = after.chars().next()?;
    if quote != '"' && quote != '\'' {
        let end = after.find([' ', '\t', '>']).unwrap_or(after.len());
        return Some(after[..end].to_string());
    }
    let rest = &after[quote.len_utf8()..];
    let end = rest.find(quote)?;
    Some(rest[..end].to_string())
}

fn attr_is_remote(value: &str) -> bool {
    let v = value.trim();
    v.starts_with("https://")
        || v.starts_with("http://")
        || v.starts_with("//")
        || v.starts_with("data:")
}

fn has_html_event_handler(html: &str) -> bool {
    let mut search = html;
    while let Some(idx) = search.find('<') {
        search = &search[idx + 1..];
        if search.starts_with('!') || search.starts_with('/') {
            continue;
        }
        let end = search.find('>').unwrap_or(search.len());
        if tag_has_event_handler(&search[..end]) {
            return true;
        }
        search = if end < search.len() {
            &search[end + 1..]
        } else {
            ""
        };
    }
    false
}

fn tag_has_event_handler(tag: &str) -> bool {
    let mut s = tag;
    while let Some(idx) = s.find(" on") {
        let rest = &s[idx + 3..];
        let n = rest
            .bytes()
            .take_while(|b| (b'a'..=b'z').contains(b))
            .count();
        if n == 0 {
            s = rest;
            continue;
        }
        let after = rest[n..].trim_start();
        if after.starts_with('=') {
            return true;
        }
        s = after;
    }
    false
}

fn skip_source(root: &Path, path: &Path) -> bool {
    let Ok(rel) = path.strip_prefix(root) else {
        return true;
    };
    rel.components().any(|c| {
        matches!(
            c.as_os_str().to_str(),
            Some("node_modules" | ".git" | ".times" | "dist" | "out" | ".next")
        )
    })
}

fn rel_str(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn looks_binary(bytes: &[u8]) -> bool {
    bytes.len() > 1_000_000 || bytes.iter().take(1024).any(|b| *b == 0)
}

fn find_akia(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(idx) = rest.find("AKIA") {
        let slice = &rest[idx..];
        let take = slice
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .map(|c| c.len_utf8())
            .sum::<usize>();
        out.push(&slice[..take.min(slice.len())]);
        rest = &slice["AKIA".len()..];
    }
    out
}

fn looks_jwt(text: &str) -> bool {
    text.split_whitespace().any(|tok| {
        let parts: Vec<&str> = tok.split('.').collect();
        parts.len() == 3
            && parts[0].starts_with("eyJ")
            && parts[0].len() > 12
            && parts[1].len() > 12
    })
}

fn looks_connection_string(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("postgres://")
        || lower.contains("postgresql://")
        || lower.contains("mongodb://")
        || lower.contains("mysql://")
        || lower.contains("amqp://")
        || lower.contains("redis://")
}

fn finding(
    path: impl Into<String>,
    rule: impl Into<String>,
    message: impl Into<String>,
) -> Finding {
    Finding {
        path: path.into(),
        rule: rule.into(),
        message: message.into(),
    }
}

fn secret(rel: &str, rule: &str, what: &str) -> Finding {
    finding(rel, rule, format!("possible {what}"))
}

fn unsafe_js(rel: &str, rule: &str, what: &str) -> Finding {
    finding(rel, rule, format!("{what} is blocked in fail-closed scan"))
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!(
            "{}...",
            s.chars().take(n.saturating_sub(3)).collect::<String>()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_eval_in_source() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/bad.ts"), "eval('x')").unwrap();
        fs::write(dir.path().join("times.aip.json"), "{}").unwrap();
        let hosting = hosting::Hosting {
            provider: Provider::Developer,
            entry_url: Some("https://example.com/".into()),
        };
        let findings = run(dir.path(), &[], &hosting);
        assert!(findings.iter().any(|f| f.rule == "EVAL"));
    }

    #[test]
    fn ignores_eval_in_comments_and_strings() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(
            dir.path().join("src/ok.ts"),
            r#"
            // eval("nope")
            const s = "eval(";
            /* new Function */
            export const n = 1;
            "#,
        )
        .unwrap();
        fs::write(dir.path().join("times.aip.json"), "{}").unwrap();
        let hosting = hosting::Hosting {
            provider: Provider::Developer,
            entry_url: Some("https://example.com/".into()),
        };
        let findings = run(dir.path(), &[], &hosting);
        assert!(!findings
            .iter()
            .any(|f| f.rule == "EVAL" || f.rule == "FUNCTION"));
    }

    #[test]
    fn flags_eval_inside_template_expression() {
        let code = js_code_only("const x = `hi ${eval('z')}`;");
        assert!(code.contains("eval("));
    }

    #[test]
    fn flags_packed_html_vectors() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("index.html"),
            r#"<script src="//cdn.example/x.js"></script><img onclick="x()">"#,
        )
        .unwrap();
        let packed = vec![("index.html".into(), dir.path().join("index.html"))];
        let mut findings = Vec::new();
        scan_packed_files(&packed, &mut findings);
        assert!(findings.iter().any(|f| f.rule == "SCRIPT"));
        assert!(findings.iter().any(|f| f.rule == "ONHANDLER"));
    }

    #[test]
    fn flags_pe_magic_in_zip() {
        let dir = tempfile::tempdir().unwrap();
        let mut pe = vec![b'M', b'Z'];
        pe.extend_from_slice(&[0u8; 32]);
        fs::write(dir.path().join("readme.txt"), pe).unwrap();
        let packed = vec![("readme.txt".into(), dir.path().join("readme.txt"))];
        let mut findings = Vec::new();
        scan_packed_files(&packed, &mut findings);
        assert!(findings.iter().any(|f| f.rule == "MAGIC"));
    }

    #[test]
    fn flags_wasm_in_zip() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("app.wasm"), b"\0asm").unwrap();
        let packed = vec![("app.wasm".into(), dir.path().join("app.wasm"))];
        let mut findings = Vec::new();
        scan_packed_files(&packed, &mut findings);
        assert!(findings.iter().any(|f| f.rule == "DENY"));
    }

    #[test]
    fn flags_zip_slip_and_secret() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("ok.html"), "<p>ok</p>").unwrap();
        fs::write(dir.path().join("secret.js"), "const k = 'tdk_abc';").unwrap();
        let packed = vec![
            ("../etc/passwd".into(), dir.path().join("ok.html")),
            ("secret.js".into(), dir.path().join("secret.js")),
        ];
        let mut findings = Vec::new();
        scan_packed_files(&packed, &mut findings);
        assert!(findings.iter().any(|f| f.rule == "PATH"));
        assert!(findings.iter().any(|f| f.rule == "TDK"));
    }
}
