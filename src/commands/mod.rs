mod aip;
mod auth;
mod completions;
mod dev;
mod init;
mod manifest;
mod money;
mod org;
mod setup_shell;
mod stats;
mod tokens;
mod upgrade;

use crate::cli::{Cli, Command, GlobalArgs};
use crate::context::{Ctx, CtxOptions};
use crate::ui::Output;

pub async fn run(cli: Cli) -> anyhow::Result<()> {
    let g = cli.global;
    let out = Output::new(g.json, g.no_input, g.yes);

    if !matches!(cli.command, Command::Completions(_) | Command::Upgrade) {
        crate::update::maybe_warn(&out);
    }

    match cli.command {
        Command::Completions(args) => completions::run(args),
        Command::SetupShell(args) => setup_shell::run(&out, args),
        Command::Upgrade => upgrade::run(&out).await,
        Command::Init(args) => init::run(&out, args),
        Command::Manifest(cmd) => manifest::run(&out, cmd),
        Command::Dev(args) => dev::run(&out, args).await,
        Command::Auth(cmd) => auth::run(&mut session(g, out)?, cmd).await,
        Command::Org(cmd) => org::run(&mut session(g, out)?, cmd).await,
        Command::Aip(cmd) => aip::run(&mut session(g, out)?, cmd).await,
        Command::Stats(args) => stats::run(&mut session(g, out)?, args).await,
        Command::Earnings(args) => money::earnings(&mut session(g, out)?, args).await,
        Command::Payout(cmd) => money::payout(&mut session(g, out)?, cmd).await,
        Command::Token(cmd) => tokens::run(&mut session(g, out)?, cmd).await,
    }
}

fn session(g: GlobalArgs, out: Output) -> anyhow::Result<Ctx> {
    Ctx::new(CtxOptions {
        api_url: g.api_url,
        profile: g.profile,
        org: g.org,
        out,
    })
}
