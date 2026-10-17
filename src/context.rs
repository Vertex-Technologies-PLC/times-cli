use std::path::PathBuf;

use crate::api::{Client, DEFAULT_API_URL};
use crate::auth::store::Store;
use crate::auth::AuthState;
use crate::config::Config;
use crate::ui::Output;

pub struct Ctx {
    pub client: Client,
    pub out: Output,
    pub profile: String,
    pub config: Config,
    pub config_dir: PathBuf,
    pub store: Store,
    pub api_url_from_flag: Option<String>,
    pub org_override: Option<String>,
    pub env_token: Option<String>,
    pub auth: Option<AuthState>,
}

pub struct CtxOptions {
    pub api_url: Option<String>,
    pub profile: Option<String>,
    pub org: Option<String>,
    pub out: Output,
}

impl Ctx {
    pub fn new(opts: CtxOptions) -> anyhow::Result<Self> {
        let config_dir = crate::config::config_dir()?;
        let config = Config::load(&config_dir)?;
        let profile = match opts
            .profile
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(name) => {
                crate::config::validate_profile_name(name)?;
                name.to_string()
            }
            None => config.current_profile_name(),
        };
        crate::config::validate_profile_name(&profile)?;
        let api_url_from_flag = opts.api_url.filter(|s| !s.is_empty());
        let api_url = api_url_from_flag
            .clone()
            .or_else(|| config.profile(&profile).api_url)
            .unwrap_or_else(|| DEFAULT_API_URL.to_string());
        let env_token = std::env::var("TIMES_TOKEN")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        Ok(Self {
            client: Client::new(&api_url)?,
            out: opts.out,
            store: Store::new(&config_dir),
            profile,
            config,
            config_dir,
            api_url_from_flag,
            org_override: opts.org.filter(|s| !s.is_empty()),
            env_token,
            auth: None,
        })
    }

    pub fn save_config(&self) -> anyhow::Result<()> {
        self.config.save(&self.config_dir)
    }
}
