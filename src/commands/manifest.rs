use console::style;
use serde_json::json;

use crate::cli::ManifestCmd;
use crate::error::CliError;
use crate::manifest::{self, bump};
use crate::ui::Output;

pub fn run(out: &Output, cmd: ManifestCmd) -> anyhow::Result<()> {
    match cmd {
        ManifestCmd::Validate { dir } => validate(out, &dir),
        ManifestCmd::Bump { level, dir } => {
            let mut loaded = manifest::load(&dir)?;
            let (from, to) = bump::apply(&mut loaded.value, level, &bump::today())?;
            manifest::save(&loaded.path, &loaded.value)?;
            let touched = bump::add_catalog_keys(&loaded.value, &loaded.root, &to)?;
            if out.json {
                return out.json(&json!({ "from": from, "to": to, "catalogs": touched }));
            }
            out.success(format!("Bumped {from} to {to}"));
            if !touched.is_empty() {
                out.info(format!(
                    "Added changelog keys to {}. Replace the placeholder text before you submit.",
                    touched.join(", ")
                ));
            }
            Ok(())
        }
    }
}

fn validate(out: &Output, dir: &std::path::Path) -> anyhow::Result<()> {
    let loaded = manifest::load(dir)?;
    let report = manifest::validate(&loaded.value, Some(&loaded.root));

    if out.json {
        out.json(&json!({
            "path": loaded.path,
            "valid": report.valid,
            "errors": report.errors,
            "warnings": report.warnings,
        }))?;
    } else {
        for e in &report.errors {
            eprintln!(
                "{} {} {}",
                style("error").red().bold(),
                style(&e.path).dim(),
                e.message
            );
        }
        for w in &report.warnings {
            eprintln!(
                "{} {} {}",
                style("warn").yellow().bold(),
                style(&w.path).dim(),
                w.message
            );
        }
        if report.valid {
            out.success(format!("{} is valid", loaded.path.display()));
        }
    }

    if report.valid {
        Ok(())
    } else {
        Err(CliError::Validation(format!(
            "times.aip.json has {} error{}.",
            report.errors.len(),
            if report.errors.len() == 1 { "" } else { "s" }
        ))
        .into())
    }
}
