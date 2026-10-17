use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use console::style;
use serde_json::{json, Value};

use crate::api::types::{DevMe, DevOrg, DevSession, DeviceCode};
use crate::auth::store::Credentials;
use crate::auth::{self, remember_session};
use crate::cli::{AuthCmd, LoginArgs};
use crate::config::validate_profile_name;
use crate::context::Ctx;
use crate::error::CliError;
use crate::ui::or_dash;

const CLIENT_NAME: &str = "times-cli";

pub async fn run(ctx: &mut Ctx, cmd: AuthCmd) -> anyhow::Result<()> {
    match cmd {
        AuthCmd::Login(args) => login(ctx, args).await,
        AuthCmd::Logout => logout(ctx).await,
        AuthCmd::Whoami => whoami(ctx).await,
        AuthCmd::Refresh => refresh_cmd(ctx).await,
        AuthCmd::List => list(ctx),
        AuthCmd::Use { name } => use_profile(ctx, &name),
        AuthCmd::Delete { name } => delete(ctx, &name).await,
    }
}

async fn login(ctx: &mut Ctx, args: LoginArgs) -> anyhow::Result<()> {
    if let Some(token) = args.token {
        return login_with_token(ctx, token.trim()).await;
    }

    let code: DeviceCode = ctx
        .client
        .post(
            "/api/v1/auth/device/code",
            None,
            &json!({ "client_name": CLIENT_NAME }),
        )
        .await?;

    let url = code
        .verification_uri_complete
        .clone()
        .unwrap_or_else(|| code.verification_uri.clone());

    eprintln!();
    eprintln!(
        "Your one-time code: {}",
        style(&code.user_code).bold().cyan()
    );
    eprintln!("Confirm it at {}", style(&url).underlined());
    eprintln!();

    if !args.no_browser && ctx.out.interactive && open::that(&url).is_ok() {
        ctx.out
            .info("Opened your browser. Waiting for you to approve...");
    } else {
        ctx.out.info("Open the link above to approve. Waiting...");
    }

    let session = poll_device_token(ctx, &code).await?;
    let session = ensure_org(ctx, session).await?;
    remember_session(ctx, &session, true)?;
    activate_profile(ctx)?;

    if ctx.out.json {
        return ctx.out.json(&json!({
            "profile": ctx.profile,
            "user": session.user,
            "org": session.org,
            "orgs": session.orgs,
        }));
    }
    ctx.out
        .success(format!("Signed in as {}", session.user.display()));
    ctx.out.info(format!("Profile: {}", ctx.profile));
    match &session.org {
        Some(org) => ctx
            .out
            .success(format!("Active organization: {} ({})", org.name, org.slug)),
        None if session.orgs.is_empty() => ctx.out.info(
            "You are not in a developer organization yet. Create one in the developer portal, then run `times org use <slug>`.",
        ),
        None => ctx
            .out
            .info("Pick an organization with `times org use <slug>`."),
    }
    Ok(())
}

async fn poll_device_token(ctx: &Ctx, code: &DeviceCode) -> anyhow::Result<DevSession> {
    let spinner = ctx.out.spinner("Waiting for approval");
    let deadline = Instant::now() + Duration::from_secs(code.expires_in.max(1));
    let mut interval = code.interval.max(1);

    loop {
        if Instant::now() >= deadline {
            spinner.finish_and_clear();
            return Err(CliError::auth(
                "The code expired before it was approved. Run `times auth login` again.",
            ));
        }
        tokio::time::sleep(Duration::from_secs(interval)).await;

        let result: anyhow::Result<DevSession> = ctx
            .client
            .post(
                "/api/v1/auth/device/token",
                None,
                &json!({ "device_code": code.device_code }),
            )
            .await;

        match result {
            Ok(session) => {
                spinner.finish_and_clear();
                return Ok(session);
            }
            Err(err) => {
                let Some(CliError::Api { code, details, .. }) = err.downcast_ref::<CliError>()
                else {
                    spinner.finish_and_clear();
                    return Err(err);
                };
                match code.to_ascii_lowercase().as_str() {
                    "authorization_pending" => {}
                    "slow_down" => {
                        interval = details
                            .as_ref()
                            .and_then(|d| d.get("interval"))
                            .and_then(Value::as_u64)
                            .unwrap_or(interval + 5);
                    }
                    "access_denied" => {
                        spinner.finish_and_clear();
                        return Err(CliError::auth("The sign in request was denied."));
                    }
                    "expired_token" | "invalid_grant" => {
                        spinner.finish_and_clear();
                        return Err(CliError::auth(
                            "The code expired before it was approved. Run `times auth login` again.",
                        ));
                    }
                    _ => {
                        spinner.finish_and_clear();
                        return Err(err);
                    }
                }
            }
        }
    }
}

async fn ensure_org(ctx: &mut Ctx, session: DevSession) -> anyhow::Result<DevSession> {
    if session.org.is_some() || session.orgs.is_empty() {
        return Ok(session);
    }
    let pick: Option<DevOrg> = if let Some(slug) = ctx.org_override.clone() {
        Some(
            session
                .orgs
                .iter()
                .find(|o| o.slug.eq_ignore_ascii_case(&slug) || o.id == slug)
                .cloned()
                .ok_or_else(|| CliError::usage(format!("You are not a member of `{slug}`.")))?,
        )
    } else if session.orgs.len() == 1 {
        Some(session.orgs[0].clone())
    } else if ctx.out.interactive {
        let labels = session
            .orgs
            .iter()
            .map(|o| format!("{} ({})", o.name, o.slug))
            .collect();
        let idx = ctx.out.select("Choose an organization", labels)?;
        Some(session.orgs[idx].clone())
    } else {
        None
    };

    let Some(org) = pick else {
        return Ok(session);
    };
    let mut selected = auth::select_org(
        &ctx.client,
        &session.access_token,
        &session.refresh_token,
        &org.id,
    )
    .await?;
    if selected.orgs.is_empty() {
        selected.orgs = session.orgs.clone();
    }
    if selected.user.id.is_empty() {
        selected.user = session.user.clone();
    }
    Ok(selected)
}

async fn login_with_token(ctx: &mut Ctx, token: &str) -> anyhow::Result<()> {
    if token.is_empty() {
        return Err(CliError::usage("The token is empty."));
    }
    let ex = auth::exchange_pat(&ctx.client, token).await?;
    ctx.store.save(
        &ctx.profile,
        &Credentials::Token {
            token: token.to_string(),
        },
    )?;
    let name = ctx.profile.clone();
    let profile = ctx.config.profile_mut(&name);
    profile.user = None;
    profile.org_id = ex.org_id.clone();
    profile.org_slug = None;
    activate_profile(ctx)?;

    if ctx.out.json {
        return ctx.out.json(&json!({
            "profile": ctx.profile,
            "org_id": ex.org_id,
            "scopes": ex.scopes,
            "token_id": ex.token_id,
        }));
    }
    ctx.out.success("Signed in with a developer token.");
    ctx.out.info(format!("Profile: {}", ctx.profile));
    if !ex.scopes.is_empty() {
        ctx.out.info(format!("Scopes: {}", ex.scopes.join(", ")));
    }
    Ok(())
}

async fn logout(ctx: &mut Ctx) -> anyhow::Result<()> {
    let name = ctx.profile.clone();
    let signed_out = sign_out_profile(ctx, &name).await?;
    ctx.save_config()?;

    if ctx.out.json {
        return ctx.out.json(&json!({
            "profile": name,
            "signed_out": signed_out,
        }));
    }
    if signed_out {
        ctx.out.success(format!("Signed out of `{name}`."));
    } else {
        ctx.out.info(format!("`{name}` was not signed in."));
    }
    if ctx.env_token.is_some() {
        ctx.out
            .warn("TIMES_TOKEN is still set in your environment.");
    }
    Ok(())
}

async fn whoami(ctx: &mut Ctx) -> anyhow::Result<()> {
    let state = ctx.authenticate().await?;
    let me: anyhow::Result<DevMe> = ctx
        .client
        .get("/api/v1/auth/developer/me", Some(&state.access_token), &[])
        .await;

    let me = match me {
        Ok(me) => me,
        Err(_) if state.via_token => {
            if ctx.out.json {
                return ctx.out.json(&json!({
                    "profile": ctx.profile,
                    "via_token": true,
                    "org_id": state.org_id,
                    "api_url": ctx.client.base(),
                }));
            }
            ctx.out.fields(&[
                ("Profile", ctx.profile.clone()),
                ("Auth", "developer token".into()),
                ("Org", state.org_id.unwrap_or_else(|| "-".into())),
                ("API", ctx.client.base().to_string()),
            ]);
            return Ok(());
        }
        Err(e) => return Err(e),
    };

    if ctx.out.json {
        return ctx.out.json(&json!({
            "profile": ctx.profile,
            "user": me.user,
            "org": me.org,
            "orgs": me.orgs,
            "via_token": state.via_token,
            "api_url": ctx.client.base(),
        }));
    }
    let org = me
        .org
        .as_ref()
        .map(|o| format!("{} ({})", o.name, o.slug))
        .unwrap_or_else(|| "none. Run `times org use <slug>`".into());
    ctx.out.fields(&[
        ("Profile", ctx.profile.clone()),
        ("User", me.user.display()),
        ("Org", org),
        (
            "Auth",
            if state.via_token {
                "developer token".into()
            } else {
                "browser session".into()
            },
        ),
        ("API", ctx.client.base().to_string()),
    ]);
    Ok(())
}

async fn refresh_cmd(ctx: &mut Ctx) -> anyhow::Result<()> {
    let session = ctx.force_refresh().await?;
    if ctx.out.json {
        return ctx.out.json(&json!({
            "profile": ctx.profile,
            "user": session.user,
            "org": session.org,
            "api_url": ctx.client.base(),
        }));
    }
    ctx.out.success("Session refreshed.");
    ctx.out.fields(&[
        ("Profile", ctx.profile.clone()),
        ("User", session.user.display()),
        (
            "Org",
            session
                .org
                .as_ref()
                .map(|o| format!("{} ({})", o.name, o.slug))
                .unwrap_or_else(|| "-".into()),
        ),
        ("API", ctx.client.base().to_string()),
    ]);
    Ok(())
}

fn list(ctx: &Ctx) -> anyhow::Result<()> {
    let current = ctx.config.current_profile_name();
    let mut names: BTreeSet<String> = ctx.config.profiles.keys().cloned().collect();
    names.extend(ctx.store.names());
    if names.is_empty() {
        names.insert(current.clone());
    }

    let rows: Vec<Value> = names
        .iter()
        .map(|name| {
            let meta = ctx.config.profile(name);
            let (auth, user) = match ctx.store.load(name) {
                Some(Credentials::Session { .. }) => {
                    ("session", meta.user.clone().unwrap_or_else(|| "-".into()))
                }
                Some(Credentials::Token { .. }) => ("token", "-".into()),
                None => ("signed_out", meta.user.unwrap_or_else(|| "-".into())),
            };
            json!({
                "name": name,
                "current": *name == current,
                "auth": auth,
                "user": if user == "-" { Value::Null } else { json!(user) },
                "org_slug": meta.org_slug,
                "api_url": meta.api_url,
            })
        })
        .collect();

    if ctx.out.json {
        return ctx.out.json(&json!({
            "current": current,
            "profiles": rows,
        }));
    }

    if names.iter().all(|n| ctx.store.load(n).is_none()) && ctx.config.profiles.is_empty() {
        ctx.out
            .info("No profiles yet. Sign in with `times auth login`.");
        return Ok(());
    }

    let table = names
        .iter()
        .map(|name| {
            let meta = ctx.config.profile(name);
            let mark = if *name == current { "*" } else { "" };
            let auth = match ctx.store.load(name) {
                Some(Credentials::Session { .. }) => "session",
                Some(Credentials::Token { .. }) => "token",
                None => "signed out",
            };
            vec![
                mark.to_string(),
                name.clone(),
                auth.to_string(),
                or_dash(meta.user),
                or_dash(meta.org_slug),
            ]
        })
        .collect();
    ctx.out
        .table(&["", "profile", "auth", "user", "org"], table);
    Ok(())
}

fn use_profile(ctx: &mut Ctx, name: &str) -> anyhow::Result<()> {
    validate_profile_name(name)?;
    ctx.config.set_current(name);
    ctx.save_config()?;
    let signed_in = ctx.store.load(name).is_some();
    let meta = ctx.config.profile(name);

    if ctx.out.json {
        return ctx.out.json(&json!({
            "profile": name,
            "signed_in": signed_in,
            "user": meta.user,
            "org_slug": meta.org_slug,
        }));
    }
    ctx.out.success(format!("Using profile `{name}`."));
    if !signed_in {
        ctx.out
            .info("This profile is not signed in. Run `times auth login`.");
    }
    Ok(())
}

async fn delete(ctx: &mut Ctx, name: &str) -> anyhow::Result<()> {
    validate_profile_name(name)?;
    let known = ctx.config.profiles.contains_key(name) || ctx.store.load(name).is_some();
    if !known {
        return Err(CliError::usage(format!("No profile named `{name}`.")));
    }
    let signed_out = sign_out_profile(ctx, name).await?;
    ctx.config.remove_profile(name);
    ctx.save_config()?;
    let current = ctx.config.current_profile_name();

    if ctx.out.json {
        return ctx.out.json(&json!({
            "deleted": name,
            "signed_out": signed_out,
            "current": current,
        }));
    }
    ctx.out.success(format!("Deleted profile `{name}`."));
    if current != name {
        ctx.out.info(format!("Current profile is `{current}`."));
    }
    Ok(())
}

async fn sign_out_profile(ctx: &mut Ctx, name: &str) -> anyhow::Result<bool> {
    let creds = ctx.store.load(name);
    if let Some(Credentials::Session { refresh_token, .. }) = &creds {
        let result: anyhow::Result<Value> = ctx
            .client
            .post(
                "/api/v1/auth/developer/logout",
                None,
                &json!({ "refresh_token": refresh_token }),
            )
            .await;
        if let Err(e) = result {
            ctx.out
                .warn(format!("Could not end the session on the server: {e}"));
        }
    }
    ctx.store.clear(name)?;
    if let Some(p) = ctx.config.profiles.get_mut(name) {
        p.user = None;
        p.org_id = None;
        p.org_slug = None;
    }
    Ok(creds.is_some())
}

fn activate_profile(ctx: &mut Ctx) -> anyhow::Result<()> {
    let name = ctx.profile.clone();
    ctx.config.set_current(&name);
    if let Some(url) = ctx.api_url_from_flag.clone() {
        ctx.config.profile_mut(&name).api_url = Some(url);
    }
    ctx.save_config()
}
