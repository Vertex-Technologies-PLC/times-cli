use clap::CommandFactory;

use crate::cli::{Cli, CompletionsArgs};

pub fn run(args: CompletionsArgs) -> anyhow::Result<()> {
    let mut buf = Vec::new();
    let mut cmd = Cli::command();
    clap_complete::generate(args.shell, &mut cmd, "times", &mut buf);
    let script = String::from_utf8(buf)?;
    print!(
        "{}",
        crate::shell::inject_completion_guard(args.shell, &script)
    );
    Ok(())
}
