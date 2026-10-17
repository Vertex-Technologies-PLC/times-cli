pub mod store;

use chrono::{Duration, Utc};
use serde_json::json;

use crate::api::types::{DevOrg, DevSession, PatExchange};
use crate::api::{seg, Client};
use crate::context::Ctx;
use crate::error::CliError;
use store::Credentials;

#[derive(Debug, Clone)]
pub struct AuthState {
    pub access_token: String,
    pub org_id: Option<String>,
    pub via_token: bool,
}

pub async fn exchange_pat(client: &Client, token: &str) -> anyhow::Result<PatExchange> {
    client
        .post(
            "/api/v1/auth/developer/tokens/exchange",
            None,
            &json!({ "token": token }),
        )
        .await
}

pub async fn refresh(
    client: &Client,
    refresh_token: &str,
    org_id: Option<&str>,
) -> anyhow::Result<DevSession> {
    let mut body = json!({ "refresh_token": refresh_token });
    if let Some(id) = org_id {
        body["org_id"] = json!(id);
    }
    client
        .post("/api/v1/auth/developer/refresh", None, &body)
        .await
}

pub async fn list_orgs(client: &Client, access_token: &str) -> anyhow::Result<Vec<DevOrg>> {
    client
        .get("/api/v1/auth/developer/orgs", Some(access_token), &[])
        .await
}

pub async fn select_org(
    client: &Client,
    access_token: &str,
    refresh_token: &str,
    org_id: &str,
) -> anyhow::Result<DevSession> {
    client
        .post(
            &format!("/api/v1/auth/developer/orgs/{}/select", seg(org_id)),
            Some(access_token),
            &json!({ "refresh_token": refresh_token }),
        )
        .await
}

pub fn session_credentials(s: &DevSession) -> Credentials {
    Credentials::Session {
        access_token: s.access_token.clone(),
        access_token_expires_at: s.access_token_expires_at,
        refresh_token: s.refresh_token.clone(),
        refresh_token_expires_at: s.refresh_token_expires_at,
        org_id: s.org.as_ref().map(|o| o.id.clone()),
    }
}

pub fn remember_session(ctx: &mut Ctx, s: &DevSession, update_profile: bool) -> anyhow::Result<()> {
    ctx.store.save(&ctx.profile, &session_credentials(s))?;
    if update_profile {
        let profile = ctx.config.profile_mut(&ctx.profile.clone());
        if !s.user.id.is_empty() {
            profile.user = Some(s.user.display());
        }
        profile.org_id = s.org.as_ref().map(|o| o.id.clone());
        profile.org_slug = s.org.as_ref().map(|o| o.slug.clone());
        ctx.save_config()?;
    }
    Ok(())
}

fn is_session_dead(err: &anyhow::Error) -> bool {
    matches!(
        err.downcast_ref::<CliError>(),
        Some(CliError::Api { code, .. }) if matches!(
            code.as_str(),
            "INVALID_REFRESH_TOKEN" | "REFRESH_TOKEN_REUSED" | "REFRESH_TOKEN_EXPIRED"
        )
    )
}

impl Ctx {
    pub async fn authenticate(&mut self) -> anyhow::Result<AuthState> {
        if let Some(a) = &self.auth {
            return Ok(a.clone());
        }
        let state = self.resolve_auth().await?;
        self.auth = Some(state.clone());
        Ok(state)
    }

    pub async fn erp_token(&mut self) -> anyhow::Result<String> {
        let state = self.authenticate().await?;
        if state.org_id.is_none() {
            return Err(CliError::Api {
                status: 403,
                code: "ORG_REQUIRED".into(),
                message: "No organization is selected.".into(),
                details: None,
            }
            .into());
        }
        Ok(state.access_token)
    }

    async fn resolve_auth(&mut self) -> anyhow::Result<AuthState> {
        if let Some(token) = self.env_token.clone() {
            if self.org_override.is_some() {
                self.out
                    .warn("TIMES_TOKEN is bound to one organization. Ignoring --org.");
            }
            return self.state_from_pat(&token).await;
        }

        let creds = self.store.load(&self.profile).ok_or_else(|| {
            CliError::auth("You are not signed in. Run `times auth login`, or set TIMES_TOKEN.")
        })?;

        match creds {
            Credentials::Token { token } => {
                if self.org_override.is_some() {
                    self.out
                        .warn("This token is bound to one organization. Ignoring --org.");
                }
                self.state_from_pat(&token).await
            }
            Credentials::Session {
                access_token,
                access_token_expires_at,
                refresh_token,
                refresh_token_expires_at,
                org_id,
            } => {
                if refresh_token_expires_at <= Utc::now() {
                    self.store.clear(&self.profile)?;
                    return Err(CliError::auth(
                        "Your session expired. Run `times auth login`.",
                    ));
                }
                let mut access = access_token;
                let mut refresh_tok = refresh_token;
                let mut current_org = org_id;

                if access_token_expires_at <= Utc::now() + Duration::seconds(60) {
                    let s = self.refresh_or_clear(&refresh_tok, None).await?;
                    access = s.access_token.clone();
                    refresh_tok = s.refresh_token.clone();
                    current_org = s.org.as_ref().map(|o| o.id.clone());
                    remember_session(self, &s, false)?;
                }

                let desired = match self.org_override.clone() {
                    Some(slug) => Some(self.org_id_for_slug(&access, &slug).await?),
                    None => self.config.profile(&self.profile).org_id,
                };

                if let Some(want) = desired {
                    if current_org.as_deref() != Some(want.as_str()) {
                        let s = self.refresh_or_clear(&refresh_tok, Some(&want)).await?;
                        access = s.access_token.clone();
                        current_org = s.org.as_ref().map(|o| o.id.clone());
                        remember_session(self, &s, false)?;
                    }
                }

                Ok(AuthState {
                    access_token: access,
                    org_id: current_org,
                    via_token: false,
                })
            }
        }
    }

    async fn state_from_pat(&self, token: &str) -> anyhow::Result<AuthState> {
        let ex = exchange_pat(&self.client, token).await?;
        Ok(AuthState {
            access_token: ex.access_token,
            org_id: ex.org_id,
            via_token: true,
        })
    }

    async fn refresh_or_clear(
        &self,
        refresh_token: &str,
        org_id: Option<&str>,
    ) -> anyhow::Result<DevSession> {
        match refresh(&self.client, refresh_token, org_id).await {
            Ok(s) => Ok(s),
            Err(e) if is_session_dead(&e) => {
                self.store.clear(&self.profile)?;
                Err(CliError::auth(
                    "Your session expired. Run `times auth login`.",
                ))
            }
            Err(e) => Err(e),
        }
    }

    pub async fn force_refresh(&mut self) -> anyhow::Result<DevSession> {
        if self.env_token.is_some() {
            return Err(CliError::usage(
                "TIMES_TOKEN does not refresh. Unset it to use a stored session.",
            ));
        }
        let creds = self.store.load(&self.profile).ok_or_else(|| {
            CliError::auth("You are not signed in. Run `times auth login`, or set TIMES_TOKEN.")
        })?;
        match creds {
            Credentials::Token { .. } => Err(CliError::usage(
                "Developer tokens do not refresh. Run `times auth login` for a browser session.",
            )),
            Credentials::Session {
                refresh_token,
                refresh_token_expires_at,
                org_id,
                ..
            } => {
                if refresh_token_expires_at <= Utc::now() {
                    self.store.clear(&self.profile)?;
                    return Err(CliError::auth(
                        "Your session expired. Run `times auth login`.",
                    ));
                }
                let stored_org = org_id;
                let config_org = self.config.profile(&self.profile).org_id;
                let mut s = self.refresh_or_clear(&refresh_token, None).await?;
                let desired = match self.org_override.clone() {
                    Some(slug) => Some(self.org_id_for_slug(&s.access_token, &slug).await?),
                    None => config_org.or(stored_org),
                };
                if let Some(want) = desired {
                    if s.org.as_ref().map(|o| o.id.as_str()) != Some(want.as_str()) {
                        s = self.refresh_or_clear(&s.refresh_token, Some(&want)).await?;
                    }
                }
                remember_session(self, &s, true)?;
                self.auth = None;
                Ok(s)
            }
        }
    }

    pub async fn org_id_for_slug(&self, access: &str, slug: &str) -> anyhow::Result<String> {
        let orgs = list_orgs(&self.client, access).await?;
        orgs.iter()
            .find(|o| o.slug.eq_ignore_ascii_case(slug) || o.id == slug)
            .map(|o| o.id.clone())
            .ok_or_else(|| {
                CliError::usage(format!(
                    "No organization named `{slug}`. Run `times org list` to see yours."
                ))
            })
    }
}
