use std::path::Path;

use futures_util::stream;
use serde_json::{json, Value};

use crate::api::types::{Aip, DevMe, Items, Listing, Submission, SubmissionDetail, UploadTicket};
use crate::api::{network_error, seg};
use crate::bundle::{self, human_size, MAX_BUNDLE_BYTES};
use crate::cli::{AipCmd, ListingCmd, OriginCmd};
use crate::context::Ctx;
use crate::error::CliError;
use crate::hosting::{self, Provider};
use crate::manifest::{self, str_at, Loaded};
use crate::push_state::{self, ChangeCheck, PushState};
use crate::ui::{or_dash, short_date};

const CHUNK: usize = 64 * 1024;
const DEVELOPERS_ORIGIN: &str = "https://developers.times.et";

pub async fn run(ctx: &mut Ctx, cmd: AipCmd) -> anyhow::Result<()> {
    match cmd {
        AipCmd::List => list(ctx).await,
        AipCmd::Create => create(ctx).await,
        AipCmd::Info { slug } => info(ctx, slug).await,
        AipCmd::Push {
            dir,
            no_build,
            force,
            submit,
            skip_probe,
        } => push(ctx, dir, no_build, force, submit, skip_probe).await,
        AipCmd::Submit { notes, version } => submit(ctx, notes, version).await,
        AipCmd::Status { slug, version } => status(ctx, slug, version).await,
        AipCmd::Withdraw { version, slug } => withdraw(ctx, slug, version).await,
        AipCmd::Logs { slug, version } => logs(ctx, slug, version).await,
        AipCmd::Scan => scan_only(ctx).await,
        AipCmd::Doctor => doctor(ctx).await,
        AipCmd::Delete { slug } => delete_aip(ctx, slug).await,
        AipCmd::Open { slug } => open_aip(ctx, slug).await,
        AipCmd::Origin(OriginCmd::Check { url }) => origin_check(ctx, url).await,
        AipCmd::Listing(cmd) => listing(ctx, cmd).await,
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
    ctx.out.info("Next: run `times aip push`.");
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

async fn push(
    ctx: &mut Ctx,
    dir: Option<String>,
    no_build: bool,
    force: bool,
    submit_after: bool,
    skip_probe: bool,
) -> anyhow::Result<()> {
    let loaded = load_manifest()?;
    require_valid_manifest(ctx, &loaded)?;
    let hosting = hosting::resolve(&loaded.value)?;
    let m = &loaded.value;
    let slug = str_at(m, "/slug").unwrap_or_default().to_string();
    let version = str_at(m, "/version").unwrap_or_default().to_string();

    if hosting.needs_bundle(m) && dir.is_none() && hosting::output_dir_missing(&loaded.root, m) {
        crate::build::ensure_output(&loaded.root, no_build)?;
        if hosting::output_dir_missing(&loaded.root, m) {
            return Err(CliError::usage(
                "Build finished but the guest output folder is still missing. Check ui.entry, or pass --dir.",
            ));
        }
    }

    let packed = if hosting.needs_bundle(m) {
        Some(bundle::collect(&loaded.root, m, dir.as_deref())?)
    } else {
        None
    };
    let packed_list: Vec<(String, std::path::PathBuf)> = packed
        .as_ref()
        .map(|files| files.iter().map(|(n, p)| (n.clone(), p.clone())).collect())
        .unwrap_or_default();
    crate::scan::fail_if_any(crate::scan::run(&loaded.root, &packed_list, &hosting))?;

    let (sha256, file_count, bundle) = match packed {
        Some(files) => {
            let b = bundle::zip_files(files)?;
            let size = b.bytes.len() as u64;
            if size > MAX_BUNDLE_BYTES {
                return Err(CliError::validation(format!(
                    "The bundle is {}. The limit is {}.",
                    human_size(size),
                    human_size(MAX_BUNDLE_BYTES)
                )));
            }
            ctx.out.info(format!(
                "Packed {} files ({}) for {slug}@{version}.",
                b.files.len(),
                human_size(size)
            ));
            (b.sha256.clone(), b.files.len() as u32, Some(b))
        }
        None => {
            let url = hosting.entry_url.clone().unwrap_or_default();
            (
                push_state::hash_developer(&url, m),
                0,
                None,
            )
        }
    };

    match push_state::classify(
        push_state::load(&loaded.root)?.as_ref(),
        &slug,
        &version,
        &sha256,
        force,
    ) {
        ChangeCheck::Unchanged => {
            if ctx.out.json {
                return ctx.out.json(&json!({
                    "slug": slug,
                    "version": version,
                    "unchanged": true,
                    "sha256": sha256
                }));
            }
            ctx.out.success(format!(
                "No changes since the last push of {slug}@{version}."
            ));
            return Ok(());
        }
        ChangeCheck::VersionBumpRequired { last_version } => {
            return Err(CliError::validation(format!(
                "Files changed but the version is still {last_version}. Run `times manifest bump patch`."
            )));
        }
        ChangeCheck::Proceed => {}
    }

    if hosting.provider == Provider::Developer {
        let url = hosting.entry_url.clone().ok_or_else(|| {
            CliError::validation("hosting.entryUrl is required for developer-hosted AIPs")
        })?;
        if skip_probe {
            if ctx.out.interactive {
                return Err(CliError::usage(
                    "--skip-probe is only for --no-input CI after you already ran `times aip origin check`.",
                ));
            }
            ctx.out
                .warn("Skipping origin probe. TIMES will still probe on the server.");
        } else {
            ctx.out.info(format!("Probing origin {url}"));
            crate::origin::probe(&url).await?;
        }
    }

    let token = ctx.erp_token().await?;
    let created = upsert_version(ctx, &token, m, &loaded.root, &slug, &version).await?;

    let sub = if let Some(bundle) = bundle {
        let size = bundle.bytes.len() as u64;
        let ticket: UploadTicket = ctx
            .client
            .post(
                &format!("{}/bundle-upload-url", version_path(&slug, &version)),
                Some(&token),
                &json!({ "contentType": bundle::CONTENT_TYPE, "sizeBytes": size }),
            )
            .await?;
        upload(ctx, &ticket, bundle.bytes).await?;
        ctx.client
            .post(
                &format!("{}/bundle-complete", version_path(&slug, &version)),
                Some(&token),
                &json!({ "bundleId": ticket.bundle_id, "sha256": bundle.sha256, "sizeBytes": size }),
            )
            .await?
    } else {
        created
    };

    push_state::save(
        &loaded.root,
        &PushState {
            slug: slug.clone(),
            version: version.clone(),
            provider: push_state::provider_str(hosting.provider),
            sha256,
            file_count,
            uploaded_at: push_state::now_rfc3339(),
        },
    )?;

    if ctx.out.json && !submit_after {
        return ctx.out.json(&sub);
    }
    if !ctx.out.json {
        print_push_result(ctx, &sub, hosting.entry_url.as_deref());
    }
    if submit_after {
        return submit(ctx, None, Some(version)).await;
    }
    Ok(())
}

fn require_valid_manifest(ctx: &Ctx, loaded: &Loaded) -> anyhow::Result<()> {
    let report = manifest::validate(&loaded.value, Some(&loaded.root));
    if report.valid {
        return Ok(());
    }
    for issue in &report.errors {
        ctx.out.warn(format!("{}: {}", issue.path, issue.message));
    }
    Err(CliError::validation(format!(
        "times.aip.json has {} error(s). Fix them, then try again.",
        report.errors.len()
    )))
}

async fn upsert_version(
    ctx: &mut Ctx,
    token: &str,
    m: &Value,
    root: &Path,
    slug: &str,
    version: &str,
) -> anyhow::Result<Submission> {
    let changelog = changelog_text(m, root, version);
    let create = json!({ "version": version, "manifest": m, "changelog": changelog });
    let created: anyhow::Result<Submission> = ctx
        .client
        .post(&format!("{}/versions", aip_path(slug)), Some(token), &create)
        .await;
    match created {
        Ok(sub) => Ok(sub),
        Err(e) if api_code(&e) == Some("VERSION_EXISTS") => {
            ctx.client
                .patch(
                    &version_path(slug, version),
                    Some(token),
                    &json!({ "manifest": m, "changelog": changelog }),
                )
                .await
        }
        Err(e) => Err(e),
    }
}

fn print_push_result(ctx: &Ctx, sub: &Submission, origin: Option<&str>) {
    ctx.out.success(format!(
        "Pushed {}@{}. Status: {}. Scan: {}.",
        sub.slug,
        sub.version,
        sub.status,
        or_dash(sub.scan_status.clone())
    ));
    if let Some(url) = sub.cdn_url.as_deref().filter(|u| !u.is_empty()) {
        ctx.out.info(format!("CDN: {url}"));
    }
    if let Some(url) = origin.filter(|u| !u.is_empty()) {
        ctx.out.info(format!("Origin: {url}"));
    }
    if let Some(notes) = sub.scan_notes.as_deref().filter(|n| !n.is_empty()) {
        ctx.out.info(format!("Scan notes: {notes}"));
    }
    ctx.out
        .info("Next: run `times aip submit` to send it for review.");
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
        ("cdn", or_dash(s.cdn_url.clone())),
        (
            "origin verified",
            short_date(s.origin_verified_at.as_deref()),
        ),
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

async fn scan_only(ctx: &mut Ctx) -> anyhow::Result<()> {
    let loaded = load_manifest()?;
    require_valid_manifest(ctx, &loaded)?;
    let hosting = hosting::resolve(&loaded.value)?;
    let packed = if hosting.needs_bundle(&loaded.value) && !hosting::output_dir_missing(&loaded.root, &loaded.value)
    {
        Some(bundle::collect(&loaded.root, &loaded.value, None)?)
    } else {
        None
    };
    let packed_list: Vec<(String, std::path::PathBuf)> = packed
        .as_ref()
        .map(|files| files.iter().map(|(n, p)| (n.clone(), p.clone())).collect())
        .unwrap_or_default();
    let findings = crate::scan::run(&loaded.root, &packed_list, &hosting);
    if ctx.out.json {
        return ctx.out.json(&json!({
            "passed": findings.is_empty(),
            "findings": findings
        }));
    }
    crate::scan::fail_if_any(findings)?;
    ctx.out
        .success("Scan passed. Human review is still required before publish.");
    Ok(())
}

async fn doctor(ctx: &mut Ctx) -> anyhow::Result<()> {
    let mut rows = Vec::new();
    let token = match ctx.erp_token().await {
        Ok(t) => {
            rows.push(("auth", "ok".into()));
            Some(t)
        }
        Err(e) => {
            rows.push(("auth", e.to_string()));
            None
        }
    };
    if let Some(token) = token.as_deref() {
        match ctx
            .client
            .get::<DevMe>("/api/v1/auth/developer/me", Some(token), &[])
            .await
        {
            Ok(me) => rows.push((
                "org",
                me.org
                    .map(|o| o.slug)
                    .unwrap_or_else(|| "not selected".into()),
            )),
            Err(e) => rows.push(("org", e.to_string())),
        }
        match ctx
            .client
            .get::<Items<Aip>>("/api/v1/developers/aips", Some(token), &[])
            .await
        {
            Ok(_) => rows.push(("api", ctx.client.base().to_string())),
            Err(e) => rows.push(("api", e.to_string())),
        }
    }
    match load_manifest() {
        Ok(loaded) => {
            let report = manifest::validate(&loaded.value, Some(&loaded.root));
            rows.push((
                "manifest",
                if report.valid {
                    str_at(&loaded.value, "/slug").unwrap_or("-").to_string()
                } else {
                    format!("{} error(s)", report.errors.len())
                },
            ));
            match hosting::resolve(&loaded.value) {
                Ok(h) => rows.push((
                    "hosting",
                    match h.entry_url {
                        Some(url) => format!("{} ({url})", h.provider.as_str()),
                        None => h.provider.as_str().to_string(),
                    },
                )),
                Err(e) => rows.push(("hosting", e.to_string())),
            }
            let slug = str_at(&loaded.value, "/slug").unwrap_or_default().to_string();
            if let Some(token) = token.as_deref() {
                match ctx
                    .client
                    .get::<Listing>(&format!("{}/listing", aip_path(&slug)), Some(token), &[])
                    .await
                {
                    Ok(listing) => {
                        let issues = listing_issues(&listing);
                        rows.push((
                            "listing",
                            if issues.is_empty() {
                                "complete".into()
                            } else {
                                issues.join("; ")
                            },
                        ));
                    }
                    Err(e) => rows.push(("listing", e.to_string())),
                }
                if let Ok(d) = detail(ctx, Some(slug), None).await {
                    rows.push((
                        "scan",
                        format!(
                            "{} ({})",
                            d.submission.status,
                            or_dash(d.submission.scan_status.clone())
                        ),
                    ));
                }
            }
        }
        Err(e) => rows.push(("manifest", e.to_string())),
    }
    if ctx.out.json {
        let map: serde_json::Map<String, Value> = rows
            .into_iter()
            .map(|(k, v)| (k.to_string(), Value::String(v)))
            .collect();
        return ctx.out.json(&Value::Object(map));
    }
    ctx.out.fields(&rows);
    Ok(())
}

async fn delete_aip(ctx: &mut Ctx, slug: Option<String>) -> anyhow::Result<()> {
    let slug = resolve_slug(slug)?;
    if !ctx
        .out
        .confirm(&format!("Delete draft AIP {slug}? This cannot be undone."))?
    {
        return Err(CliError::usage("Cancelled."));
    }
    let token = ctx.erp_token().await?;
    let body: Value = ctx.client.delete(&aip_path(&slug), Some(&token)).await?;
    if ctx.out.json {
        return ctx.out.json(&body);
    }
    ctx.out.success(format!("Deleted {slug}."));
    Ok(())
}

async fn open_aip(ctx: &mut Ctx, slug: Option<String>) -> anyhow::Result<()> {
    let slug = resolve_slug(slug)?;
    let url = format!("{DEVELOPERS_ORIGIN}/apps/{slug}");
    if ctx.out.json {
        return ctx.out.json(&json!({ "url": url }));
    }
    match open::that(&url) {
        Ok(()) => ctx.out.success(format!("Opened {url}")),
        Err(_) => ctx.out.info(format!("Open {url} in your browser.")),
    }
    Ok(())
}

async fn origin_check(ctx: &mut Ctx, url: Option<String>) -> anyhow::Result<()> {
    let url = match url {
        Some(u) => u,
        None => {
            let loaded = load_manifest()?;
            hosting::resolve(&loaded.value)?
                .entry_url
                .ok_or_else(|| {
                    CliError::usage("Pass a URL, or set hosting.entryUrl in times.aip.json.")
                })?
        }
    };
    let report = crate::origin::probe(&url).await?;
    if ctx.out.json {
        return ctx.out.json(&json!({
            "url": report.url,
            "status": report.status,
            "contentType": report.content_type,
            "frameAncestorsOk": report.frame_ancestors_ok,
            "xFrameOptions": report.x_frame_options
        }));
    }
    ctx.out.success(format!(
        "{url} can be embedded in app.times.et (HTTP {}).",
        report.status
    ));
    Ok(())
}

async fn listing(ctx: &mut Ctx, cmd: ListingCmd) -> anyhow::Result<()> {
    match cmd {
        ListingCmd::Get { slug } => listing_get(ctx, slug).await,
        ListingCmd::Set {
            slug,
            name,
            summary,
            description,
            category,
            pricing,
            logo_url,
            support_url,
            privacy_url,
            website_url,
        } => {
            listing_set(
                ctx,
                slug,
                name,
                summary,
                description,
                category,
                pricing,
                logo_url,
                support_url,
                privacy_url,
                website_url,
            )
            .await
        }
    }
}

async fn listing_get(ctx: &mut Ctx, slug: Option<String>) -> anyhow::Result<()> {
    let slug = resolve_slug(slug)?;
    let token = ctx.erp_token().await?;
    let listing: Listing = ctx
        .client
        .get(&format!("{}/listing", aip_path(&slug)), Some(&token), &[])
        .await?;
    if ctx.out.json {
        return ctx.out.json(&listing);
    }
    let issues = listing_issues(&listing);
    ctx.out.fields(&[
        ("slug", listing.slug),
        ("name", listing.name),
        ("summary", listing.summary),
        ("category", listing.category),
        ("pricing", listing.pricing),
        (
            "issues",
            if issues.is_empty() {
                "none".into()
            } else {
                issues.join("; ")
            },
        ),
    ]);
    Ok(())
}

async fn listing_set(
    ctx: &mut Ctx,
    slug: Option<String>,
    name: Option<String>,
    summary: Option<String>,
    description: Option<String>,
    category: Option<String>,
    pricing: Option<String>,
    logo_url: Option<String>,
    support_url: Option<String>,
    privacy_url: Option<String>,
    website_url: Option<String>,
) -> anyhow::Result<()> {
    let slug = resolve_slug(slug)?;
    let token = ctx.erp_token().await?;
    let current: Listing = ctx
        .client
        .get(&format!("{}/listing", aip_path(&slug)), Some(&token), &[])
        .await
        .unwrap_or_else(|_| Listing {
            slug: slug.clone(),
            ..Listing::default()
        });
    let body = json!({
        "name": name.unwrap_or(current.name),
        "summary": summary.unwrap_or(current.summary),
        "description": description.unwrap_or(current.description),
        "category": category.unwrap_or(current.category),
        "pricing": pricing.unwrap_or(current.pricing),
        "logoUrl": logo_url.or(current.logo_url),
        "screenshots": current.screenshots,
        "supportUrl": support_url.or(current.support_url),
        "privacyUrl": privacy_url.or(current.privacy_url),
        "websiteUrl": website_url.or(current.website_url),
        "priceEtb": current.price_etb,
        "billingPeriod": current.billing_period,
        "trialDays": current.trial_days,
        "memberSlugs": current.member_slugs
    });
    let listing: Listing = ctx
        .client
        .put(&format!("{}/listing", aip_path(&slug)), Some(&token), &body)
        .await?;
    if ctx.out.json {
        return ctx.out.json(&listing);
    }
    let issues = listing_issues(&listing);
    if issues.is_empty() {
        ctx.out.success(format!("Listing for {slug} is complete."));
    } else {
        ctx.out.warn(format!(
            "Listing saved. Still missing: {}.",
            issues.join("; ")
        ));
    }
    Ok(())
}

fn listing_issues(listing: &Listing) -> Vec<String> {
    let mut issues = Vec::new();
    if listing.name.trim().len() < 2 {
        issues.push("name".into());
    }
    if listing.summary.trim().len() < 10 {
        issues.push("summary".into());
    }
    if listing.description.trim().len() < 40 {
        issues.push("description".into());
    }
    if listing.logo_url.as_deref().unwrap_or("").is_empty() {
        issues.push("logo".into());
    }
    if listing.support_url.as_deref().unwrap_or("").is_empty() {
        issues.push("support URL".into());
    }
    if listing.privacy_url.as_deref().unwrap_or("").is_empty() {
        issues.push("privacy URL".into());
    }
    issues
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
