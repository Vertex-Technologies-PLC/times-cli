use std::path::Path;

use futures_util::stream;
use serde_json::{json, Value};

use crate::api::types::{Aip, Items, Submission, SubmissionDetail, UploadTicket};
use crate::api::{network_error, seg};
use crate::bundle::{self, human_size, MAX_BUNDLE_BYTES};
use crate::cli::AipCmd;
use crate::context::Ctx;
use crate::error::CliError;
use crate::manifest::{self, str_at, Loaded};
use crate::ui::{or_dash, short_date};

const CHUNK: usize = 64 * 1024;

pub async fn run(ctx: &mut Ctx, cmd: AipCmd) -> anyhow::Result<()> {
    match cmd {
        AipCmd::List => list(ctx).await,
        AipCmd::Create => create(ctx).await,
        AipCmd::Info { slug } => info(ctx, slug).await,
        AipCmd::Push { dir } => push(ctx, dir).await,
        AipCmd::Submit { notes, version } => submit(ctx, notes, version).await,
        AipCmd::Status { slug, version } => status(ctx, slug, version).await,
        AipCmd::Withdraw { version, slug } => withdraw(ctx, slug, version).await,
        AipCmd::Logs { slug, version } => logs(ctx, slug, version).await,
    }
}

fn aip_path(slug: &str) -> String {
    format!("/api/v1/developers/aips/{}", seg(slug))
}

fn version_path(slug: &str, version: &str) -> String {
    format!("{}/versions/{}", aip_path(slug), seg(version))
}

fn load_manifest() -> anyhow::Result<Loaded> {
    manifest::load(Path::new("."))
}

fn manifest_slug() -> anyhow::Result<String> {
    let loaded = load_manifest().map_err(|_| {
        CliError::usage("Pass a slug, or run this inside a project with times.aip.json.")
    })?;
    str_at(&loaded.value, "/slug")
        .map(str::to_string)
        .ok_or_else(|| CliError::validation("times.aip.json has no slug."))
}

fn resolve_slug(slug: Option<String>) -> anyhow::Result<String> {
    match slug {
        Some(s) => Ok(s),
        None => manifest_slug(),
    }
}

async fn resolve_version(
    ctx: &mut Ctx,
    token: &str,
    slug: &str,
    version: Option<String>,
) -> anyhow::Result<String> {
    if let Some(v) = version {
        return Ok(v);
    }
    if let Ok(loaded) = load_manifest() {
        if str_at(&loaded.value, "/slug") == Some(slug) {
            if let Some(v) = str_at(&loaded.value, "/version") {
                return Ok(v.to_string());
            }
        }
    }
    let aip: Aip = ctx.client.get(&aip_path(slug), Some(token), &[]).await?;
    aip.latest_version.ok_or_else(|| {
        CliError::usage(format!("{slug} has no versions yet. Run `times aip push`."))
    })
}

async fn list(ctx: &mut Ctx) -> anyhow::Result<()> {
    let token = ctx.erp_token().await?;
    let res: Items<Aip> = ctx
        .client
        .get("/api/v1/developers/aips", Some(&token), &[])
        .await?;
    if ctx.out.json {
        return ctx.out.json(&res.items);
    }
    if res.items.is_empty() {
        ctx.out
            .info("No AIPs yet. Run `times init`, then `times aip create`.");
        return Ok(());
    }
    let rows = res
        .items
        .iter()
        .map(|a| {
            vec![
                a.slug.clone(),
                a.kind.clone(),
                or_dash(a.status.clone()),
                or_dash(a.latest_version.clone()),
                or_dash(a.published_version.clone()),
                or_dash(a.active_installs),
            ]
        })
        .collect();
    ctx.out.table(
        &["slug", "kind", "status", "latest", "published", "installs"],
        rows,
    );
    Ok(())
}

async fn create(ctx: &mut Ctx) -> anyhow::Result<()> {
    let loaded = load_manifest()?;
    let m = &loaded.value;
    let slug =
        str_at(m, "/slug").ok_or_else(|| CliError::validation("times.aip.json has no slug."))?;
    let kind =
        str_at(m, "/kind").ok_or_else(|| CliError::validation("times.aip.json has no kind."))?;
    let body = json!({
        "slug": slug,
        "kind": kind,
        "mode": str_at(m, "/mode"),
        "name": str_at(m, "/name"),
    });
    let token = ctx.erp_token().await?;
    let aip: Aip = ctx
        .client
        .post("/api/v1/developers/aips", Some(&token), &body)
        .await?;
    if ctx.out.json {
        return ctx.out.json(&aip);
    }
    ctx.out
        .success(format!("Created {} ({}).", aip.slug, aip.kind));
    ctx.out
        .info("Next: build your app, then run `times aip push`.");
    Ok(())
}

async fn info(ctx: &mut Ctx, slug: Option<String>) -> anyhow::Result<()> {
    let slug = resolve_slug(slug)?;
    let token = ctx.erp_token().await?;
    let aip: Aip = ctx.client.get(&aip_path(&slug), Some(&token), &[]).await?;
    let versions: Items<Submission> = ctx
        .client
        .get(&format!("{}/versions", aip_path(&slug)), Some(&token), &[])
        .await?;
    if ctx.out.json {
        return ctx
            .out
            .json(&json!({ "aip": aip, "versions": versions.items }));
    }
    let rating = match (aip.rating_average, aip.rating_count) {
        (Some(avg), Some(n)) if n > 0 => format!("{avg:.1} ({n})"),
        _ => "-".into(),
    };
    ctx.out.fields(&[
        ("slug", aip.slug.clone()),
        ("name", or_dash(aip.name.clone())),
        ("kind", aip.kind.clone()),
        ("mode", or_dash(aip.mode.clone())),
        ("status", or_dash(aip.status.clone())),
        ("published", or_dash(aip.published_version.clone())),
        ("installs", or_dash(aip.active_installs)),
        ("rating", rating),
    ]);
    if !versions.items.is_empty() {
        println!();
        let rows = versions
            .items
            .iter()
            .map(|v| {
                vec![
                    v.version.clone(),
                    v.status.clone(),
                    or_dash(v.scan_status.clone()),
                    short_date(v.updated_at.as_deref().or(v.created_at.as_deref())),
                ]
            })
            .collect();
        ctx.out
            .table(&["version", "status", "scan", "updated"], rows);
    }
    Ok(())
}

async fn push(ctx: &mut Ctx, dir: Option<String>) -> anyhow::Result<()> {
    let loaded = load_manifest()?;
    let report = manifest::validate(&loaded.value, Some(&loaded.root));
    if !report.valid {
        for issue in &report.errors {
            ctx.out.warn(format!("{}: {}", issue.path, issue.message));
        }
        return Err(CliError::validation(format!(
            "times.aip.json has {} error(s). Fix them, then push again.",
            report.errors.len()
        )));
    }
    let m = &loaded.value;
    let slug = str_at(m, "/slug").unwrap_or_default().to_string();
    let version = str_at(m, "/version").unwrap_or_default().to_string();

    let bundle = bundle::pack(&loaded.root, m, dir.as_deref())?;
    let size = bundle.bytes.len() as u64;
    if size > MAX_BUNDLE_BYTES {
        return Err(CliError::validation(format!(
            "The bundle is {}. The limit is {}.",
            human_size(size),
            human_size(MAX_BUNDLE_BYTES)
        )));
    }
    ctx.out.info(format!(
        "Packed {} files ({}) for {slug}@{version}.",
        bundle.files.len(),
        human_size(size)
    ));

    let token = ctx.erp_token().await?;
    let changelog = changelog_text(m, &loaded.root, &version);
    let create = json!({ "version": version, "manifest": m, "changelog": changelog });
    let created: anyhow::Result<Submission> = ctx
        .client
        .post(
            &format!("{}/versions", aip_path(&slug)),
            Some(&token),
            &create,
        )
        .await;
    match created {
        Ok(_) => {}
        Err(e) if api_code(&e) == Some("VERSION_EXISTS") => {
            let _: Submission = ctx
                .client
                .patch(
                    &version_path(&slug, &version),
                    Some(&token),
                    &json!({ "manifest": m, "changelog": changelog }),
                )
                .await?;
        }
        Err(e) => return Err(e),
    }

    let ticket: UploadTicket = ctx
        .client
        .post(
            &format!("{}/bundle-upload-url", version_path(&slug, &version)),
            Some(&token),
            &json!({ "contentType": bundle::CONTENT_TYPE, "sizeBytes": size }),
        )
        .await?;
    upload(ctx, &ticket, bundle.bytes).await?;

    let sub: Submission = ctx
        .client
        .post(
            &format!("{}/bundle-complete", version_path(&slug, &version)),
            Some(&token),
            &json!({ "bundleId": ticket.bundle_id, "sha256": bundle.sha256, "sizeBytes": size }),
        )
        .await?;

    if ctx.out.json {
        return ctx.out.json(&sub);
    }
    ctx.out.success(format!(
        "Uploaded {slug}@{version}. Status: {}. Scan: {}.",
        sub.status,
        or_dash(sub.scan_status.clone())
    ));
    if let Some(notes) = sub.scan_notes.as_deref().filter(|n| !n.is_empty()) {
        ctx.out.info(format!("Scan notes: {notes}"));
    }
    ctx.out
        .info("Next: run `times aip submit` to send it for review.");
    Ok(())
}

async fn upload(ctx: &Ctx, ticket: &UploadTicket, bytes: Vec<u8>) -> anyhow::Result<()> {
    let total = bytes.len() as u64;
    let bar = ctx.out.bytes_bar(total, "Uploading");
    let progress = bar.clone();
    let chunks: Vec<Vec<u8>> = bytes.chunks(CHUNK).map(<[u8]>::to_vec).collect();
    let body = stream::iter(chunks.into_iter().map(move |c| {
        progress.inc(c.len() as u64);
        Ok::<_, std::io::Error>(c)
    }));
    let content_type = ticket
        .content_type
        .clone()
        .unwrap_or_else(|| bundle::CONTENT_TYPE.to_string());
    let resp = ctx
        .client
        .http()
        .put(&ticket.upload_url)
        .header("Content-Type", content_type)
        .header("Content-Length", total)
        .body(reqwest::Body::wrap_stream(body))
        .send()
        .await
        .map_err(network_error)?;
    bar.finish_and_clear();
    let status = resp.status();
    if !status.is_success() {
        return Err(CliError::Api {
            status: status.as_u16(),
            code: "BUNDLE_UPLOAD_FAILED".into(),
            message: format!("The bundle upload failed with HTTP {}.", status.as_u16()),
            details: None,
        }
        .into());
    }
    Ok(())
}

fn changelog_text(m: &Value, root: &Path, version: &str) -> Option<String> {
    let entry = m
        .pointer("/changelog")
        .and_then(Value::as_array)?
        .iter()
        .find(|e| e.get("version").and_then(Value::as_str) == Some(version))?;
    let key = entry.get("summaryKey").and_then(Value::as_str)?;
    let catalog = str_at(m, "/i18n/catalogs/en")?;
    let text = std::fs::read_to_string(manifest::relative(root, catalog)).ok()?;
    let catalog: Value = serde_json::from_str(&text).ok()?;
    let mut node = &catalog;
    if let Some(flat) = catalog.get(key) {
        node = flat;
    } else {
        for part in key.split('.') {
            node = node.get(part)?;
        }
    }
    node.as_str().map(str::to_string)
}

fn api_code(e: &anyhow::Error) -> Option<&str> {
    match e.downcast_ref::<CliError>() {
        Some(CliError::Api { code, .. }) => Some(code.as_str()),
        _ => None,
    }
}

async fn submit(
    ctx: &mut Ctx,
    notes: Option<String>,
    version: Option<String>,
) -> anyhow::Result<()> {
    let slug = manifest_slug()?;
    let token = ctx.erp_token().await?;
    let version = resolve_version(ctx, &token, &slug, version).await?;
    if let Some(n) = notes.as_deref().filter(|n| !n.trim().is_empty()) {
        let _: Submission = ctx
            .client
            .patch(
                &version_path(&slug, &version),
                Some(&token),
                &json!({ "changelog": n }),
            )
            .await?;
    }
    let sub: Submission = ctx
        .client
        .send(
            reqwest::Method::POST,
            &format!("{}/submit", version_path(&slug, &version)),
            Some(&token),
            &[],
            None,
        )
        .await?;
    if ctx.out.json {
        return ctx.out.json(&sub);
    }
    ctx.out
        .success(format!("Submitted {slug}@{version} for review."));
    ctx.out.info("Track it with `times aip status`.");
    Ok(())
}

async fn detail(
    ctx: &mut Ctx,
    slug: Option<String>,
    version: Option<String>,
) -> anyhow::Result<SubmissionDetail> {
    let slug = resolve_slug(slug)?;
    let token = ctx.erp_token().await?;
    let version = resolve_version(ctx, &token, &slug, version).await?;
    ctx.client
        .get(&version_path(&slug, &version), Some(&token), &[])
        .await
}

async fn status(
    ctx: &mut Ctx,
    slug: Option<String>,
    version: Option<String>,
) -> anyhow::Result<()> {
    let d = detail(ctx, slug, version).await?;
    if ctx.out.json {
        return ctx.out.json(&d.submission);
    }
    let s = &d.submission;
    ctx.out.fields(&[
        ("aip", format!("{}@{}", s.slug, s.version)),
        ("status", s.status.clone()),
        ("scan", or_dash(s.scan_status.clone())),
        ("scan notes", or_dash(s.scan_notes.clone())),
        ("review notes", or_dash(s.review_notes.clone())),
        ("submitted", short_date(s.submitted_at.as_deref())),
        ("reviewed", short_date(s.reviewed_at.as_deref())),
        ("published", short_date(s.published_at.as_deref())),
    ]);
    Ok(())
}

async fn logs(ctx: &mut Ctx, slug: Option<String>, version: Option<String>) -> anyhow::Result<()> {
    let d = detail(ctx, slug, version).await?;
    if ctx.out.json {
        return ctx.out.json(&d.events);
    }
    if d.events.is_empty() {
        ctx.out.info("No review events yet.");
        return Ok(());
    }
    let rows = d
        .events
        .iter()
        .map(|e| {
            let change = match (&e.from_status, &e.to_status) {
                (Some(f), Some(t)) if f != t => format!("{f} -> {t}"),
                (_, Some(t)) => t.clone(),
                _ => "-".into(),
            };
            vec![
                or_dash(e.created_at.as_deref().map(|s| s.replace('T', " "))),
                or_dash(e.actor_kind.clone()),
                change,
                or_dash(e.notes.clone()),
            ]
        })
        .collect();
    ctx.out.table(&["time", "actor", "status", "notes"], rows);
    Ok(())
}

async fn withdraw(ctx: &mut Ctx, slug: Option<String>, version: String) -> anyhow::Result<()> {
    let slug = resolve_slug(slug)?;
    if !ctx
        .out
        .confirm(&format!("Withdraw {slug}@{version} from review?"))?
    {
        return Err(CliError::usage("Cancelled."));
    }
    let token = ctx.erp_token().await?;
    let sub: Submission = ctx
        .client
        .send(
            reqwest::Method::POST,
            &format!("{}/withdraw", version_path(&slug, &version)),
            Some(&token),
            &[],
            None,
        )
        .await?;
    if ctx.out.json {
        return ctx.out.json(&sub);
    }
    ctx.out.success(format!(
        "Withdrew {slug}@{version}. Status: {}.",
        sub.status
    ));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_changelog_summary_from_catalog() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("i18n")).unwrap();
        std::fs::write(
            dir.path().join("i18n/en.json"),
            r#"{"changelog":{"v1_0_0":{"summary":"First release"}}}"#,
        )
        .unwrap();
        let m = json!({
            "i18n": { "catalogs": { "en": "./i18n/en.json" } },
            "changelog": [{ "version": "1.0.0", "summaryKey": "changelog.v1_0_0.summary" }]
        });
        assert_eq!(
            changelog_text(&m, dir.path(), "1.0.0").as_deref(),
            Some("First release")
        );
        assert_eq!(changelog_text(&m, dir.path(), "2.0.0"), None);
    }

    #[test]
    fn builds_escaped_paths() {
        assert_eq!(
            version_path("my-app", "1.0.0"),
            "/api/v1/developers/aips/my-app/versions/1.0.0"
        );
    }
}
