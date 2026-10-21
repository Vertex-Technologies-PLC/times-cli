mod aip;
mod auth;
mod completions;
mod create_handshake;
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
use crate::shell;
use crate::ui::Output;

pub async fn run(cli: Cli) -> anyhow::Result<()> {
    let g = cli.global;
    let out = Output::new(g.json, g.no_input, g.yes);
    ensure_shell_hook(&cli.command, &out);

    match cli.command {
        Command::Completions(args) => completions::run(args),
        Command::SetupShell(args) => setup_shell::run(&out, args),
        Command::Upgrade => upgrade::run(&out).await,
        Command::Init(args) => {
            refresh_scopes(&g).await;
            init::run(&out, args).await
        }
        Command::Manifest(cmd) => {
            refresh_scopes(&g).await;
            manifest::run(&out, cmd)
        }
        Command::Dev(args) => {
            refresh_scopes(&g).await;
            dev::run(&out, args).await
        }
        Command::Auth(cmd) => auth::run(&mut session(g, out)?, cmd).await,
        Command::Org(cmd) => org::run(&mut session(g, out)?, cmd).await,
        Command::Aip(cmd) => {
            refresh_scopes(&g).await;
            aip::run(&mut session(g, out)?, cmd).await
        }
        Command::Stats(args) => stats::run(&mut session(g, out)?, args).await,
        Command::Earnings(args) => money::earnings(&mut session(g, out)?, args).await,
        Command::Payout(cmd) => money::payout(&mut session(g, out)?, cmd).await,
        Command::Token(cmd) => tokens::run(&mut session(g, out)?, cmd).await,
    }
}

async fn refresh_scopes(g: &GlobalArgs) {
    let base =
        crate::manifest::scopes::api_base_from_flag(g.api_url.as_deref(), g.profile.as_deref());
    crate::manifest::scopes::refresh(&base).await;
}

fn session(g: GlobalArgs, out: Output) -> anyhow::Result<Ctx> {
    Ctx::new(CtxOptions {
        api_url: g.api_url,
        profile: g.profile,
        org: g.org,
        out,
    })
}

fn ensure_shell_hook(cmd: &Command, out: &Output) {
    if cfg!(windows) || !out.interactive {
        return;
    }
    if matches!(
        cmd,
        Command::Completions(_) | Command::SetupShell(_) | Command::Upgrade
    ) {
        return;
    }
    let Ok(config_dir) = crate::config::config_dir() else {
        return;
    };
    let Ok(Some(report)) = shell::ensure(&config_dir) else {
        return;
    };
    out.info(
        "Configured your shell so the TIMES CLI wins over the POSIX times builtin.",
    );
    out.info(format!("This terminal: . {}", report.shellenv.display()));
}
