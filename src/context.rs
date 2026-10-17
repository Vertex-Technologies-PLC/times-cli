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
    pub org_override: Option<String>,
    pub env_token: Option<String>,
    pub auth: Option<AuthState>,
}

pub struct CtxOptions {
    pub api_url: Option<String>,
    pub profile: String,
    pub org: Option<String>,
    pub out: Output,
}

impl Ctx {
    pub fn new(opts: CtxOptions) -> anyhow::Result<Self> {
        let config_dir = crate::config::config_dir()?;
        let config = Config::load(&config_dir)?;
        let api_url = opts
            .api_url
            .filter(|s| !s.is_empty())
            .or_else(|| config.profile(&opts.profile).api_url)
            .unwrap_or_else(|| DEFAULT_API_URL.to_string());
        let env_token = std::env::var("TIMES_TOKEN")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        Ok(Self {
            client: Client::new(&api_url)?,
            out: opts.out,
            store: Store::new(&config_dir),
            profile: opts.profile,
            config,
            config_dir,
            org_override: opts.org.filter(|s| !s.is_empty()),
            env_token,
            auth: None,
        })
    }

    pub fn save_config(&self) -> anyhow::Result<()> {
        self.config.save(&self.config_dir)
    }
}
