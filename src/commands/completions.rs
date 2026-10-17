use clap::CommandFactory;

use crate::cli::{Cli, CompletionsArgs};

pub fn run(args: CompletionsArgs) -> anyhow::Result<()> {
    let mut cmd = Cli::command();
    clap_complete::generate(args.shell, &mut cmd, "times", &mut std::io::stdout());
    Ok(())
}
