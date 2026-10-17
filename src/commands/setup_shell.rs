use crate::cli::SetupShellArgs;
use crate::shell::{self, SetupReport};
use crate::ui::Output;

pub fn run(out: &Output, args: SetupShellArgs) -> anyhow::Result<()> {
    if cfg!(windows) {
        if out.json {
            out.json(&serde_json::json!({
                "ok": true,
                "changed": false,
                "message": "Windows does not have a times builtin."
            }))?;
        } else {
            out.info("Windows does not have a times builtin. Nothing to change.");
        }
        return Ok(());
    }

    let bin_dirs = shell::default_bin_dirs();
    if args.print {
        print!("{}", shell::shellenv_body(&bin_dirs));
        return Ok(());
    }

    let config_dir = crate::config::config_dir()?;
    let report = shell::apply(&config_dir, &bin_dirs)?;
    if out.json {
        out.json(&report)?;
        return Ok(());
    }
    print_human(out, &report);
    Ok(())
}

fn print_human(out: &Output, report: &SetupReport) {
    out.success(format!("Wrote {}", report.shellenv.display()));
    for rc in &report.rc_files {
        out.info(format!("Updated {}", rc.display()));
    }
    out.info(
        "zsh and bash ship a builtin named times. New terminals will run the TIMES CLI instead.",
    );
    out.info(format!("This terminal: . {}", report.shellenv.display()));
}
