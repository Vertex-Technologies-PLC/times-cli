mod aip;
mod completions;
mod dev;
mod init;
mod login;
mod manifest;
mod money;
mod org;
mod stats;
mod tokens;

use crate::cli::{Cli, Command};
use crate::context::{Ctx, CtxOptions};
use crate::ui::Output;

pub async fn run(cli: Cli) -> anyhow::Result<()> {
    let g = cli.global;
    let out = Output::new(g.json, g.no_input, g.yes);

    match cli.command {
        Command::Completions(args) => return completions::run(args),
        Command::Init(args) => return init::run(&out, args),
        Command::Manifest(cmd) => return manifest::run(&out, cmd),
        Command::Dev(args) => return dev::run(&out, args).await,
        _ => {}
    }

    let mut ctx = Ctx::new(CtxOptions {
        api_url: g.api_url,
        profile: g.profile,
        org: g.org,
        out,
    })?;

    match cli.command {
        Command::Login(args) => login::login(&mut ctx, args).await,
        Command::Logout => login::logout(&mut ctx).await,
        Command::Whoami => login::whoami(&mut ctx).await,
        Command::Org(cmd) => org::run(&mut ctx, cmd).await,
        Command::Aip(cmd) => aip::run(&mut ctx, cmd).await,
        Command::Stats(args) => stats::run(&mut ctx, args).await,
        Command::Earnings(args) => money::earnings(&mut ctx, args).await,
        Command::Payout(cmd) => money::payout(&mut ctx, cmd).await,
        Command::Token(cmd) => tokens::run(&mut ctx, cmd).await,
        Command::Completions(_) | Command::Init(_) | Command::Manifest(_) | Command::Dev(_) => {
            unreachable!()
        }
    }
}
