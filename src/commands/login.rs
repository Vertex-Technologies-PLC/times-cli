use std::time::{Duration, Instant};

use console::style;
use serde_json::{json, Value};

use crate::api::types::{DevMe, DevOrg, DevSession, DeviceCode};
use crate::auth::store::Credentials;
use crate::auth::{self, remember_session};
use crate::cli::LoginArgs;
use crate::context::Ctx;
use crate::error::CliError;

const CLIENT_NAME: &str = "times-cli";

pub async fn login(ctx: &mut Ctx, args: LoginArgs) -> anyhow::Result<()> {
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

    if ctx.out.json {
        return ctx.out.json(&json!({
            "user": session.user,
            "org": session.org,
            "orgs": session.orgs,
        }));
    }
    ctx.out
        .success(format!("Signed in as {}", session.user.display()));
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
                "The code expired before it was approved. Run `times login` again.",
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
                            "The code expired before it was approved. Run `times login` again.",
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
    ctx.save_config()?;

    if ctx.out.json {
        return ctx.out.json(&json!({
            "org_id": ex.org_id,
            "scopes": ex.scopes,
            "token_id": ex.token_id,
        }));
    }
    ctx.out.success("Signed in with a developer token.");
    if !ex.scopes.is_empty() {
        ctx.out.info(format!("Scopes: {}", ex.scopes.join(", ")));
    }
    Ok(())
}

pub async fn logout(ctx: &mut Ctx) -> anyhow::Result<()> {
    let creds = ctx.store.load(&ctx.profile);
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
    ctx.store.clear(&ctx.profile)?;
    let name = ctx.profile.clone();
    let profile = ctx.config.profile_mut(&name);
    profile.user = None;
    profile.org_id = None;
    profile.org_slug = None;
    ctx.save_config()?;

    if ctx.out.json {
        return ctx.out.json(&json!({ "signed_out": creds.is_some() }));
    }
    if creds.is_some() {
        ctx.out.success("Signed out.");
    } else {
        ctx.out.info("You were not signed in.");
    }
    if ctx.env_token.is_some() {
        ctx.out
            .warn("TIMES_TOKEN is still set in your environment.");
    }
    Ok(())
}

pub async fn whoami(ctx: &mut Ctx) -> anyhow::Result<()> {
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
                    "via_token": true,
                    "org_id": state.org_id,
                }));
            }
            ctx.out.fields(&[
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
