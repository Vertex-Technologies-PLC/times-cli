use serde_json::json;

use crate::api::seg;
use crate::api::types::{ApiToken, CreatedToken};
use crate::cli::TokenCmd;
use crate::context::Ctx;
use crate::error::CliError;
use crate::ui::or_dash;

const DEV_SCOPES: &[&str] = &[
    "aips:read",
    "aips:write",
    "aips:submit",
    "stats:read",
    "earnings:read",
    "payouts:write",
    "webhooks:write",
    "org:read",
    "depot:read",
    "depot:deploy",
    "depot:repos",
];

pub async fn run(ctx: &mut Ctx, cmd: TokenCmd) -> anyhow::Result<()> {
    match cmd {
        TokenCmd::List => list(ctx).await,
        TokenCmd::Create {
            name,
            scopes,
            expires,
        } => create(ctx, name, scopes, expires).await,
        TokenCmd::Revoke { id } => revoke(ctx, id).await,
    }
}

pub fn parse_expiry(s: &str) -> Option<u32> {
    if s.eq_ignore_ascii_case("never") {
        return Some(0);
    }
    let n: u32 = s.strip_suffix('d').unwrap_or(s).parse().ok()?;
    (1..=365).contains(&n).then_some(n)
}

fn fmt_time(t: Option<chrono::DateTime<chrono::Utc>>) -> String {
    or_dash(t.map(|t| t.format("%Y-%m-%d").to_string()))
}

async fn list(ctx: &mut Ctx) -> anyhow::Result<()> {
    let state = ctx.authenticate().await?;
    let tokens: Vec<ApiToken> = ctx
        .client
        .get(
            "/api/v1/auth/developer/tokens",
            Some(&state.access_token),
            &[],
        )
        .await?;
    if ctx.out.json {
        return ctx.out.json(&tokens);
    }
    if tokens.is_empty() {
        ctx.out
            .info("No tokens yet. Create one with `times token create --name ci`.");
        return Ok(());
    }
    let rows = tokens
        .iter()
        .map(|t| {
            vec![
                t.id.clone(),
                t.name.clone(),
                or_dash(t.token_prefix.clone()),
                t.scopes.join(","),
                fmt_time(t.last_used_at),
                fmt_time(t.expires_at),
            ]
        })
        .collect();
    ctx.out.table(
        &["id", "name", "prefix", "scopes", "last used", "expires"],
        rows,
    );
    Ok(())
}

async fn create(
    ctx: &mut Ctx,
    name: String,
    scopes: Vec<String>,
    expires: String,
) -> anyhow::Result<()> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err(CliError::usage("--name cannot be empty."));
    }
    let scopes: Vec<String> = scopes
        .into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if scopes.is_empty() {
        return Err(CliError::usage("Pass at least one scope with --scopes."));
    }
    if let Some(bad) = scopes.iter().find(|s| !DEV_SCOPES.contains(&s.as_str())) {
        return Err(CliError::usage(format!(
            "Unknown scope `{bad}`. Use one of: {}.",
            DEV_SCOPES.join(", ")
        )));
    }
    let days = parse_expiry(&expires)
        .ok_or_else(|| CliError::usage("--expires must be between 1d and 365d, or `never`."))?;
    let state = ctx.authenticate().await?;
    let created: CreatedToken = ctx
        .client
        .post(
            "/api/v1/auth/developer/tokens",
            Some(&state.access_token),
            &json!({ "name": name, "scopes": scopes, "expires_in_days": days }),
        )
        .await?;
    if ctx.out.json {
        return ctx.out.json(&created);
    }
    ctx.out.success(format!("Created token {}.", created.name));
    ctx.out.info("Copy it now. You will not see it again.");
    println!("{}", created.token);
    ctx.out
        .info("Use it in CI with: export TIMES_TOKEN=<token>");
    Ok(())
}

async fn revoke(ctx: &mut Ctx, id: String) -> anyhow::Result<()> {
    if !ctx.out.confirm(&format!("Revoke token {id}?"))? {
        return Err(CliError::usage("Cancelled."));
    }
    let state = ctx.authenticate().await?;
    let _: serde_json::Value = ctx
        .client
        .delete(
            &format!("/api/v1/auth/developer/tokens/{}", seg(&id)),
            Some(&state.access_token),
        )
        .await?;
    if ctx.out.json {
        return ctx.out.json(&json!({ "id": id, "revoked": true }));
    }
    ctx.out.success(format!("Revoked token {id}."));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_expiry() {
        assert_eq!(parse_expiry("90d"), Some(90));
        assert_eq!(parse_expiry("365"), Some(365));
        assert_eq!(parse_expiry("never"), Some(0));
        assert_eq!(parse_expiry("0d"), None);
        assert_eq!(parse_expiry("400d"), None);
        assert_eq!(parse_expiry("soon"), None);
    }
}
