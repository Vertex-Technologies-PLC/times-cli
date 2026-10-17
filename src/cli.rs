use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

use crate::manifest::bump::Level;

#[derive(Debug, Parser)]
#[command(
    name = "times",
    version,
    about = "Build, test and publish TIMES AIPs from your terminal.",
    after_help = "Docs: https://developers.times.et/docs/cli-reference"
)]
pub struct Cli {
    #[command(flatten)]
    pub global: GlobalArgs,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Args)]
pub struct GlobalArgs {
    /// Print machine readable JSON
    #[arg(long, global = true)]
    pub json: bool,

    /// Run against this organization (slug)
    #[arg(long, global = true, value_name = "SLUG", env = "TIMES_ORG")]
    pub org: Option<String>,

    /// TIMES API gateway URL
    #[arg(long, global = true, value_name = "URL", env = "TIMES_API_URL")]
    pub api_url: Option<String>,

    /// Use a named credentials profile
    #[arg(
        long,
        global = true,
        value_name = "NAME",
        env = "TIMES_PROFILE",
        default_value = "default"
    )]
    pub profile: String,

    /// Skip confirmation prompts
    #[arg(long, short = 'y', global = true)]
    pub yes: bool,

    /// Never prompt. Fail instead when input is needed
    #[arg(
        long,
        global = true,
        env = "TIMES_NO_INPUT",
        value_parser = clap::builder::FalseyValueParser::new()
    )]
    pub no_input: bool,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Sign in with your browser, or with a developer token
    Login(LoginArgs),
    /// Sign out and remove stored credentials
    Logout,
    /// Show the signed in user and organization
    Whoami,
    /// List and switch organizations
    #[command(subcommand)]
    Org(OrgCmd),
    /// Create a new AIP project
    Init(InitArgs),
    /// Validate or bump times.aip.json
    #[command(subcommand)]
    Manifest(ManifestCmd),
    /// Serve your build locally for preview
    Dev(DevArgs),
    /// Create, push and submit AIPs
    #[command(subcommand)]
    Aip(AipCmd),
    /// Show installs, usage and revenue
    Stats(StatsArgs),
    /// Show your balance and earnings
    Earnings(EarningsArgs),
    /// List payout accounts and request withdrawals
    #[command(subcommand)]
    Payout(PayoutCmd),
    /// Manage developer tokens for CI
    #[command(subcommand, visible_alias = "tokens")]
    Token(TokenCmd),
    /// Print shell completions
    Completions(CompletionsArgs),
}

#[derive(Debug, Args)]
pub struct LoginArgs {
    /// Sign in with a developer token (tdk_...)
    #[arg(long, value_name = "TOKEN")]
    pub token: Option<String>,

    /// Do not open the browser
    #[arg(long)]
    pub no_browser: bool,
}

#[derive(Debug, Subcommand)]
pub enum OrgCmd {
    /// List your organizations
    List,
    /// Switch the active organization
    Use {
        /// Organization slug
        slug: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Kind {
    App,
    Integration,
    Pack,
}

impl Kind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Kind::App => "app",
            Kind::Integration => "integration",
            Kind::Pack => "pack",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Framework {
    Vite,
    Static,
    Next,
}

impl Framework {
    pub fn as_str(&self) -> &'static str {
        match self {
            Framework::Vite => "vite",
            Framework::Static => "static",
            Framework::Next => "next",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Mode {
    Platform,
    Runtime,
}

impl Mode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Mode::Platform => "platform",
            Mode::Runtime => "runtime",
        }
    }
}

#[derive(Debug, Args)]
pub struct InitArgs {
    /// Folder to create. Defaults to the current folder
    pub dir: Option<PathBuf>,

    #[arg(long, value_enum)]
    pub kind: Option<Kind>,

    #[arg(long, value_enum)]
    pub framework: Option<Framework>,

    #[arg(long, value_enum)]
    pub mode: Option<Mode>,

    /// AIP slug. Defaults to the folder name
    #[arg(long)]
    pub slug: Option<String>,

    /// Display name
    #[arg(long)]
    pub name: Option<String>,

    /// Overwrite files that already exist
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, Subcommand)]
pub enum ManifestCmd {
    /// Check times.aip.json against the schema and your catalogs
    Validate {
        /// Project folder
        #[arg(long, default_value = ".")]
        dir: PathBuf,
    },
    /// Bump the version and add a changelog entry
    Bump {
        #[arg(value_enum)]
        level: Level,
        #[arg(long, default_value = ".")]
        dir: PathBuf,
    },
}

#[derive(Debug, Args)]
pub struct DevArgs {
    #[arg(long, default_value_t = 4321)]
    pub port: u16,

    /// Folder to serve. Defaults to the folder of ui.entry
    #[arg(long)]
    pub dir: Option<String>,

    #[arg(long, default_value = "127.0.0.1")]
    pub host: String,
}

#[derive(Debug, Subcommand)]
pub enum AipCmd {
    /// List your AIPs
    List,
    /// Register the AIP from times.aip.json
    Create,
    /// Show an AIP and its versions
    Info {
        /// Defaults to the slug in times.aip.json
        slug: Option<String>,
    },
    /// Bundle and upload the current version
    Push {
        /// Build folder. Defaults to the folder of ui.entry
        #[arg(long)]
        dir: Option<String>,
    },
    /// Submit the current version for review
    Submit {
        /// Release notes for reviewers
        #[arg(long)]
        notes: Option<String>,
        /// Defaults to the version in times.aip.json
        #[arg(long)]
        version: Option<String>,
    },
    /// Show the review status of the latest version
    Status {
        slug: Option<String>,
        #[arg(long)]
        version: Option<String>,
    },
    /// Withdraw a version from review
    Withdraw {
        version: String,
        #[arg(long)]
        slug: Option<String>,
    },
    /// Show the review history of a version
    Logs {
        slug: Option<String>,
        #[arg(long)]
        version: Option<String>,
    },
}

#[derive(Debug, Args)]
pub struct StatsArgs {
    #[arg(long)]
    pub slug: Option<String>,

    /// Time range like 7d, 30d or 90d
    #[arg(long, default_value = "30d")]
    pub range: String,

    /// Start date (YYYY-MM-DD). Overrides --range
    #[arg(long)]
    pub from: Option<String>,

    /// End date (YYYY-MM-DD)
    #[arg(long)]
    pub to: Option<String>,
}

#[derive(Debug, Args)]
pub struct EarningsArgs {
    #[arg(long)]
    pub slug: Option<String>,

    #[arg(long, default_value_t = 1)]
    pub page: u32,

    #[arg(long, default_value_t = 20)]
    pub page_size: u32,
}

#[derive(Debug, Subcommand)]
pub enum PayoutCmd {
    /// List withdrawal requests
    List,
    /// List payout accounts
    Accounts,
    /// Request a withdrawal
    Request {
        /// Amount in ETB
        #[arg(long)]
        amount: f64,
        /// Payout account id. Defaults to your default account
        #[arg(long)]
        account: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum TokenCmd {
    /// List developer tokens
    List,
    /// Create a developer token
    Create {
        #[arg(long)]
        name: String,
        /// Scopes, comma separated
        #[arg(
            long,
            value_delimiter = ',',
            default_value = "aips:read,aips:write,aips:submit,stats:read"
        )]
        scopes: Vec<String>,
        /// Lifetime like 30d, 90d or 365d. Use `never` for no expiry
        #[arg(long, default_value = "90d")]
        expires: String,
    },
    /// Revoke a developer token
    Revoke { id: String },
}

#[derive(Debug, Args)]
pub struct CompletionsArgs {
    #[arg(value_enum)]
    pub shell: clap_complete::Shell,
}
