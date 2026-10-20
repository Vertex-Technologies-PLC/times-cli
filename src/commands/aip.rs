use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{json, Value};

use crate::api::types::{
    Aip, DepotJob, DepotJobLog, DevMe, Items, Listing, Submission, SubmissionDetail,
};
use crate::api::seg;
use crate::bundle::{self, human_size, MAX_BUNDLE_BYTES};
use crate::cli::{AipCmd, ListingCmd, OriginCmd};
use crate::context::Ctx;
use crate::error::CliError;
use crate::hosting;
use crate::manifest::{self, str_at, Loaded};
use crate::ui::{or_dash, short_date};

const DEVELOPERS_ORIGIN: &str = "https://developers.times.et";

pub async fn run(ctx: &mut Ctx, cmd: AipCmd) -> anyhow::Result<()> {
    match cmd {
        AipCmd::List => list(ctx).await,
        AipCmd::Create => create(ctx).await,
        AipCmd::Info { slug } => info(ctx, slug).await,
        AipCmd::Deploy {
            dir,
            remote,
            r#ref,
            follow,
            submit,
            bump,
        } => deploy(ctx, dir, remote, r#ref, follow, submit, bump).await,
        AipCmd::Submit { notes, version } => submit(ctx, notes, version).await,
        AipCmd::Status { slug, version } => status(ctx, slug, version).await,
        AipCmd::Withdraw { version, slug } => withdraw(ctx, slug, version).await,
        AipCmd::Logs { slug, version } => logs(ctx, slug, version).await,
        AipCmd::Scan { remote, follow } => scan_only(ctx, remote, follow).await,
        AipCmd::Retry { id, follow } => retry_job(ctx, id, follow).await,
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
        CliError::usage(format!("{slug} has no versions yet. Run `times aip deploy`."))
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
    ctx.out.info("Next: run `times aip deploy`.");
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

const DEPOT_TERMINAL: &[&str] = &["succeeded", "failed", "cancelled", "timed_out"];

async fn deploy(
    ctx: &mut Ctx,
    dir: PathBuf,
    remote: Option<String>,
    git_ref: String,
    follow: bool,
    submit_after: bool,
    bump_level: Option<crate::manifest::bump::Level>,
) -> anyhow::Result<()> {
    let project_dir = if dir.as_os_str() == "." {
        PathBuf::from(".")
    } else if dir.is_absolute() {
        dir.clone()
    } else {
        std::env::current_dir()?.join(&dir)
    };

    if let Some(level) = bump_level {
        let report = manifest::bump::bump_project(&project_dir, level)?;
        if !ctx.out.json {
            ctx.out
                .success(format!("Bumped {} to {}", report.from, report.to));
            if report.package_json {
                ctx.out.info("Updated package.json version to match.");
            }
            if !report.catalogs.is_empty() {
                ctx.out.info(format!(
                    "Added changelog keys to {}. Replace the placeholder text before you submit.",
                    report.catalogs.join(", ")
                ));
            }
        }
    }

    let loaded = manifest::load(&project_dir)?;
    require_valid_manifest(ctx, &loaded)?;
    let m = &loaded.value;
    let slug = str_at(m, "/slug")
        .ok_or_else(|| CliError::validation("times.aip.json has no slug."))?
        .to_string();
    let version = str_at(m, "/version").map(str::to_string);
    let token = ctx.erp_token().await?;

    let job = if let Some(repo_url) = remote {
        let body = json!({
            "slug": slug,
            "kind": "build_and_stage",
            "trigger_kind": "cli_remote",
            "github_repo_url": repo_url,
            "git_ref": git_ref,
            "version": version,
        });
        let job: DepotJob = ctx
            .client
            .post("/api/v1/depot/jobs", Some(&token), &body)
            .await?;
        if !ctx.out.json {
            ctx.out.info(format!(
                "Queued remote deploy for {slug} from {repo_url}@{}.",
                job.git_ref.as_deref().unwrap_or(&git_ref)
            ));
        }
        job
    } else {
        let root = loaded.root.clone();
        if !ctx.out.json {
            ctx.out.step(1, 3, format!("Packing source for {slug}"));
        }
        let pack_spin = ctx.out.spinner("Packing source…");
        let packed = bundle::pack_source(&root)?;
        pack_spin.finish_and_clear();
        if !ctx.out.json && !packed.vendored_packages.is_empty() {
            ctx.out.info(format!(
                "  Vendored {} for the depot build.",
                packed.vendored_packages.join(", ")
            ));
        }
        let size = packed.bytes.len() as u64;
        if size > MAX_BUNDLE_BYTES {
            return Err(CliError::validation(format!(
                "The source zip is {}. The limit is {}.",
                human_size(size),
                human_size(MAX_BUNDLE_BYTES)
            )));
        }
        if !ctx.out.json {
            ctx.out.info(format!(
                "  {} files · {}",
                packed.files.len(),
                human_size(size)
            ));
            if size > 8 * 1024 * 1024 {
                ctx.out.warn(
                    "Large upload. Prefer `times aip deploy --remote <github-url>` for repos over ~8 MB.",
                );
            }
            ctx.out.step(2, 3, "Uploading source to depot");
        }
        let source_sha = packed.sha256.clone();
        upload_source(ctx, &token, source_sha.clone(), packed.bytes).await?;
        if !ctx.out.json {
            ctx.out.step(3, 3, "Queuing depot job");
        }
        let body = json!({
            "slug": slug,
            "kind": "build_and_stage",
            "trigger_kind": "cli_local",
            "source_blob_sha": source_sha,
            "version": version,
        });
        let job: DepotJob = post_with_retry(ctx, "/api/v1/depot/jobs", &token, &body).await?;
        if !ctx.out.json {
            ctx.out.success(format!("Queued deploy job {}", job.id));
        }
        job
    };

    let job = if follow {
        follow_job(ctx, &token, job).await?
    } else {
        job
    };

    if submit_after {
        if job.status != "succeeded" {
            return Err(CliError::usage(
                "Deploy did not succeed. Fix the job, then run `times aip submit`.",
            ));
        }
        return submit(ctx, None, version).await;
    }
    if ctx.out.json {
        return ctx.out.json(&job);
    }
    print_deploy_result(ctx, &job);
    if !follow {
        ctx.out.info(format!(
            "Follow with `times aip deploy --follow` or open {DEVELOPERS_ORIGIN}/dashboard/aips/{slug}."
        ));
    }
    Ok(())
}

fn print_deploy_result(ctx: &Ctx, job: &DepotJob) {
    let verb = if DEPOT_TERMINAL.contains(&job.status.as_str()) {
        "Deploy"
    } else {
        "Deploy queued"
    };
    if job.status == "succeeded" {
        ctx.out.success(format!(
            "{verb} {} ({}). Status: {}.",
            job.slug, job.id, job.status
        ));
    } else if matches!(job.status.as_str(), "failed" | "cancelled" | "timed_out") {
        ctx.out.warn(format!(
            "{verb} {} ({}). Status: {}.",
            job.slug, job.id, job.status
        ));
        if let Some(msg) = job.error_message.as_deref().filter(|m| !m.is_empty()) {
            ctx.out.warn(msg);
        }
    } else {
        ctx.out.info(format!(
            "{verb} {} ({}). Status: {}.",
            job.slug, job.id, job.status
        ));
    }
    if let Some(v) = job.version.as_deref().filter(|v| !v.is_empty()) {
        ctx.out.info(format!("Version: {v}"));
    }
}

fn job_status_label(status: &str) -> &'static str {
    match status {
        "queued" => "Queued",
        "claimed" => "Claimed",
        "cloning" => "Fetching source",
        "scanning_source" => "Scanning source",
        "building" => "Building",
        "packing" => "Packing",
        "scanning_bundle" => "Scanning bundle",
        "staging" => "Staging",
        "finalizing" => "Finalizing",
        "succeeded" => "Succeeded",
        "failed" => "Failed",
        "cancelled" => "Cancelled",
        "timed_out" => "Timed out",
        _ => "Working",
    }
}

async fn follow_job(ctx: &mut Ctx, token: &str, mut job: DepotJob) -> anyhow::Result<DepotJob> {
    let mut after: i64 = 0;
    let mut last_status = String::new();
    let spinner = ctx.out.spinner(format!(
        "{} · {}",
        job.slug,
        job_status_label(&job.status)
    ));
    loop {
        if !ctx.out.json {
            if let Ok(logs) = ctx
                .client
                .get::<Vec<DepotJobLog>>(
                    &format!("/api/v1/depot/jobs/{}/logs", seg(&job.id)),
                    Some(token),
                    &[("after", after.to_string())],
                )
                .await
            {
                if !logs.is_empty() {
                    spinner.suspend(|| {
                        for line in &logs {
                            let text = line
                                .message
                                .as_deref()
                                .or(line.line.as_deref())
                                .unwrap_or("");
                            ctx.out.log_line(line.stream.as_deref(), text);
                            if let Some(id) = line.id.or(line.seq) {
                                after = after.max(id);
                            }
                        }
                    });
                }
            }
        }
        if DEPOT_TERMINAL.contains(&job.status.as_str()) {
            spinner.finish_and_clear();
            if !ctx.out.json {
                ctx.out.status_line(&job.status);
            }
            return Ok(job);
        }
        if job.status != last_status {
            last_status = job.status.clone();
            spinner.set_message(format!(
                "{} · {}",
                job.slug,
                job_status_label(&job.status)
            ));
            if !ctx.out.json {
                spinner.suspend(|| ctx.out.status_line(&job.status));
            }
        }
        tokio::time::sleep(Duration::from_millis(1500)).await;
        match ctx
            .client
            .get::<DepotJob>(
                &format!("/api/v1/depot/jobs/{}", seg(&job.id)),
                Some(token),
                &[],
            )
            .await
        {
            Ok(next) => job = next,
            Err(err) if is_retryable_edge(&err) => {
                spinner.set_message(format!("{} · reconnecting…", job.slug));
                continue;
            }
            Err(err) => {
                spinner.finish_and_clear();
                return Err(err);
            }
        }
    }
}

fn is_retryable_edge(err: &anyhow::Error) -> bool {
    err.downcast_ref::<CliError>()
        .map(|e| match e {
            CliError::Api { status, .. } => (521..=524).contains(status) || *status == 429,
            CliError::Network(_) => true,
            _ => false,
        })
        .unwrap_or(false)
}

async fn post_with_retry<T: serde::de::DeserializeOwned, B: serde::Serialize>(
    ctx: &Ctx,
    path: &str,
    token: &str,
    body: &B,
) -> anyhow::Result<T> {
    let mut attempt = 0u32;
    loop {
        attempt += 1;
        match ctx.client.post(path, Some(token), body).await {
            Ok(v) => return Ok(v),
            Err(err) if is_retryable_edge(&err) && attempt < 4 => {
                if !ctx.out.json {
                    ctx.out.warn(format!(
                        "Transient API error (attempt {attempt}/3). Retrying…"
                    ));
                }
                tokio::time::sleep(Duration::from_secs(2u64.pow(attempt))).await;
            }
            Err(err) => return Err(err),
        }
    }
}

async fn upload_source(
    ctx: &Ctx,
    token: &str,
    sha256: String,
    bytes: Vec<u8>,
) -> anyhow::Result<()> {
    let total = bytes.len() as u64;
    let mut attempt = 0u32;
    loop {
        attempt += 1;
        let bar = ctx.out.bytes_bar(total, "Uploading");
        let progress = bar.clone();
        let result = ctx
            .client
            .post_bytes::<Value>(
                "/api/v1/depot/source-upload",
                token,
                bytes.clone(),
                bundle::CONTENT_TYPE,
                &sha256,
                move |n| progress.inc(n),
            )
            .await;
        bar.finish_and_clear();
        match result {
            Ok(data) => {
                if !ctx.out.json {
                    if data.get("deduped").and_then(|v| v.as_bool()) == Some(true) {
                        ctx.out.success(format!(
                            "Source already on depot ({})",
                            human_size(total)
                        ));
                    } else {
                        ctx.out.success(format!("Uploaded {}", human_size(total)));
                    }
                }
                return Ok(());
            }
            Err(err) if is_retryable_edge(&err) && attempt < 4 => {
                if !ctx.out.json {
                    ctx.out.warn(format!(
                        "Upload retry {attempt}/3 after a transient edge error…"
                    ));
                }
                tokio::time::sleep(Duration::from_secs(2u64.pow(attempt))).await;
            }
            Err(err) => return Err(err),
        }
    }
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

async fn scan_only(ctx: &mut Ctx, remote: bool, follow: bool) -> anyhow::Result<()> {
    let loaded = load_manifest()?;
    require_valid_manifest(ctx, &loaded)?;
    if remote {
        return scan_remote(ctx, &loaded, follow).await;
    }
    if !ctx.out.json {
        ctx.out.step(1, 3, "Resolving hosting and entry");
    }
    let hosting = hosting::resolve(&loaded.value)?;
    if !ctx.out.json {
        ctx.out.step(2, 3, "Collecting files to scan");
    }
    let packed = if hosting.needs_bundle(&loaded.value)
        && !hosting::output_dir_missing(&loaded.root, &loaded.value)
    {
        Some(bundle::collect(&loaded.root, &loaded.value, None)?)
    } else {
        None
    };
    let packed_list: Vec<(String, std::path::PathBuf)> = packed
        .as_ref()
        .map(|files| files.iter().map(|(n, p)| (n.clone(), p.clone())).collect())
        .unwrap_or_default();
    if !ctx.out.json {
        ctx.out.step(
            3,
            3,
            format!(
                "Running fail-closed scan ({} packed files)",
                packed_list.len()
            ),
        );
    }
    let spin = ctx.out.spinner("Scanning…");
    let findings = crate::scan::run(&loaded.root, &packed_list, &hosting);
    spin.finish_and_clear();
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

async fn scan_remote(ctx: &mut Ctx, loaded: &Loaded, follow: bool) -> anyhow::Result<()> {
    let m = &loaded.value;
    let slug = str_at(m, "/slug")
        .ok_or_else(|| CliError::validation("times.aip.json has no slug."))?
        .to_string();
    let version = str_at(m, "/version").map(str::to_string);
    let token = ctx.erp_token().await?;
    if !ctx.out.json {
        ctx.out.step(1, 3, format!("Packing source for {slug}"));
    }
    let packed = bundle::pack_source(&loaded.root)?;
    let size = packed.bytes.len() as u64;
    if size > MAX_BUNDLE_BYTES {
        return Err(CliError::validation(format!(
            "The source zip is {}. The limit is {}.",
            human_size(size),
            human_size(MAX_BUNDLE_BYTES)
        )));
    }
    if !ctx.out.json {
        ctx.out.info(format!(
            "  {} files · {}",
            packed.files.len(),
            human_size(size)
        ));
        if !packed.vendored_packages.is_empty() {
            ctx.out.info(format!(
                "  Vendored {} for the depot build.",
                packed.vendored_packages.join(", ")
            ));
        }
        ctx.out.step(2, 3, "Uploading source to depot");
    }
    let source_sha = packed.sha256.clone();
    upload_source(ctx, &token, source_sha.clone(), packed.bytes).await?;
    if !ctx.out.json {
        ctx.out.step(3, 3, "Queuing depot scan");
    }
    let body = json!({
        "slug": slug,
        "kind": "scan_only",
        "trigger_kind": "cli_local",
        "source_blob_sha": source_sha,
        "version": version,
    });
    let job: DepotJob = post_with_retry(ctx, "/api/v1/depot/jobs", &token, &body).await?;
    if !ctx.out.json {
        ctx.out.success(format!("Queued scan job {}", job.id));
    }
    let job = if follow {
        follow_job(ctx, &token, job).await?
    } else {
        job
    };
    if ctx.out.json {
        return ctx.out.json(&job);
    }
    print_deploy_result(ctx, &job);
    if job.status != "succeeded" {
        return Err(CliError::usage(
            "Depot scan did not succeed. Fix findings, then run `times aip scan --remote` again.",
        ));
    }
    ctx.out
        .success("Depot scan passed. Human review is still required before publish.");
    Ok(())
}

async fn retry_job(ctx: &mut Ctx, id: String, follow: bool) -> anyhow::Result<()> {
    let token = ctx.erp_token().await?;
    let job: DepotJob = ctx
        .client
        .post(
            &format!("/api/v1/depot/jobs/{}/retry", seg(&id)),
            Some(&token),
            &json!({}),
        )
        .await?;
    if !ctx.out.json {
        ctx.out.info(format!("Queued retry job {}.", job.id));
    }
    let job = if follow {
        follow_job(ctx, &token, job).await?
    } else {
        job
    };
    if ctx.out.json {
        return ctx.out.json(&job);
    }
    print_deploy_result(ctx, &job);
    Ok(())
}

async fn doctor(ctx: &mut Ctx) -> anyhow::Result<()> {
    let mut rows = Vec::new();
    let token = match ctx.authenticate().await {
        Ok(state) => {
            rows.push(("auth", "ok".into()));
            Some(state.access_token)
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
            let slug = str_at(&loaded.value, "/slug")
                .unwrap_or_default()
                .to_string();
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
            hosting::resolve(&loaded.value)?.entry_url.ok_or_else(|| {
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
