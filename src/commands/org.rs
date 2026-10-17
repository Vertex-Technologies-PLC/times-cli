use serde_json::json;

use crate::auth::store::Credentials;
use crate::auth::{self, remember_session};
use crate::cli::OrgCmd;
use crate::context::Ctx;
use crate::error::CliError;
use crate::ui::or_dash;

pub async fn run(ctx: &mut Ctx, cmd: OrgCmd) -> anyhow::Result<()> {
    match cmd {
        OrgCmd::List => list(ctx).await,
        OrgCmd::Use { slug } => use_org(ctx, &slug).await,
    }
}

async fn list(ctx: &mut Ctx) -> anyhow::Result<()> {
    let state = ctx.authenticate().await?;
    let orgs = auth::list_orgs(&ctx.client, &state.access_token).await?;
    if ctx.out.json {
        return ctx.out.json(&json!({
            "active_org_id": state.org_id,
            "orgs": orgs,
        }));
    }
    if orgs.is_empty() {
        ctx.out.info(
            "You are not in a developer organization yet. Create one in the developer portal.",
        );
        return Ok(());
    }
    let rows = orgs
        .iter()
        .map(|o| {
            let active = if state.org_id.as_deref() == Some(o.id.as_str()) {
                "*".to_string()
            } else {
                String::new()
            };
            vec![
                active,
                o.slug.clone(),
                o.name.clone(),
                or_dash(o.role.clone()),
            ]
        })
        .collect();
    ctx.out.table(&["", "slug", "name", "role"], rows);
    Ok(())
}

async fn use_org(ctx: &mut Ctx, slug: &str) -> anyhow::Result<()> {
    if ctx.env_token.is_some() {
        return Err(CliError::usage(
            "TIMES_TOKEN is bound to one organization. Unset it to switch organizations.",
        ));
    }
    if matches!(
        ctx.store.load(&ctx.profile),
        Some(Credentials::Token { .. })
    ) {
        return Err(CliError::usage(
            "You signed in with a developer token, which is bound to one organization. Run `times login` to switch.",
        ));
    }
    ctx.org_override = None;
    let state = ctx.authenticate().await?;
    let org_id = ctx.org_id_for_slug(&state.access_token, slug).await?;

    let Some(Credentials::Session { refresh_token, .. }) = ctx.store.load(&ctx.profile) else {
        return Err(CliError::auth("You are not signed in. Run `times login`."));
    };
    let session =
        auth::select_org(&ctx.client, &state.access_token, &refresh_token, &org_id).await?;
    remember_session(ctx, &session, true)?;
    ctx.auth = None;

    let org = session.org.clone();
    if ctx.out.json {
        return ctx.out.json(&json!({ "org": org }));
    }
    match org {
        Some(o) => ctx
            .out
            .success(format!("Active organization: {} ({})", o.name, o.slug)),
        None => ctx.out.success(format!("Active organization: {slug}")),
    }
    Ok(())
}
