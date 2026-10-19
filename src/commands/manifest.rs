use std::time::Duration;

use console::style;
use indicatif::ProgressBar;
use serde_json::json;

use crate::cli::ManifestCmd;
use crate::error::CliError;
use crate::manifest::{self, bump, CheckStatus, Report};
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
    let jobs = manifest::validation_plan(&loaded.value, Some(&loaded.root));
    let pb = out.progress(jobs.len() as u64);
    let mut report = Report::default();
    let mut checks = Vec::new();

    if !out.json {
        let slug = manifest::str_at(&loaded.value, "/slug").unwrap_or("AIP");
        let version = manifest::str_at(&loaded.value, "/version").unwrap_or("");
        eprintln!(
            "{}  {}  {}",
            style("TIMES").cyan().bold(),
            style(slug).bold(),
            style(version).dim()
        );
    }

    for job in jobs {
        pb.set_message(job.name);
        let mut partial = Report::default();
        (job.run)(&loaded.value, Some(&loaded.root), &mut partial);
        let status = partial.check_status();
        if !out.json {
            emit(out, &pb, status_line(status, job.name));
            for issue in &partial.errors {
                emit(
                    out,
                    &pb,
                    format!("        {}  {}", style(&issue.path).red(), issue.message),
                );
            }
            for issue in &partial.warnings {
                emit(
                    out,
                    &pb,
                    format!("        {}  {}", style(&issue.path).yellow(), issue.message),
                );
            }
        }
        checks.push(json!({
            "name": job.name,
            "status": status_name(status),
            "errors": &partial.errors,
            "warnings": &partial.warnings,
        }));
        report.errors.extend(partial.errors);
        report.warnings.extend(partial.warnings);
        pb.inc(1);
        if out.interactive {
            std::thread::sleep(Duration::from_millis(40));
        }
    }
    pb.finish_and_clear();

    report.errors.dedup();
    report.valid = report.errors.is_empty();

    if out.json {
        out.json(&json!({
            "path": loaded.path,
            "valid": report.valid,
            "errors": report.errors,
            "warnings": report.warnings,
            "checks": checks,
        }))?;
    } else {
        let (pass, warn, fail) = tally(&checks);
        eprintln!("{}", summary_meter(pass, warn, fail, report.valid));
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

fn status_name(status: CheckStatus) -> &'static str {
    match status {
        CheckStatus::Pass => "pass",
        CheckStatus::Warn => "warn",
        CheckStatus::Fail => "fail",
    }
}

fn status_line(status: CheckStatus, name: &str) -> String {
    let badge = match status {
        CheckStatus::Pass => style("PASS").green().bold().to_string(),
        CheckStatus::Warn => style("WARN").yellow().bold().to_string(),
        CheckStatus::Fail => style("FAIL").red().bold().to_string(),
    };
    format!("  {badge}  {name}")
}

fn emit(out: &Output, pb: &ProgressBar, line: String) {
    if out.interactive {
        pb.println(line);
    } else {
        eprintln!("{line}");
    }
}

fn tally(checks: &[serde_json::Value]) -> (usize, usize, usize) {
    let mut pass = 0;
    let mut warn = 0;
    let mut fail = 0;
    for check in checks {
        match check.get("status").and_then(|v| v.as_str()) {
            Some("warn") => warn += 1,
            Some("fail") => fail += 1,
            _ => pass += 1,
        }
    }
    (pass, warn, fail)
}

fn bar_widths(pass: usize, warn: usize, fail: usize, width: usize) -> (usize, usize, usize) {
    let total = pass + warn + fail;
    if total == 0 {
        return (width, 0, 0);
    }
    let mut pass_w = width * pass / total;
    let mut warn_w = width * warn / total;
    let mut fail_w = width * fail / total;
    let leftover = width - pass_w - warn_w - fail_w;
    if fail > 0 {
        fail_w += leftover;
    } else if warn > 0 {
        warn_w += leftover;
    } else {
        pass_w += leftover;
    }
    (pass_w, warn_w, fail_w)
}

fn summary_meter(pass: usize, warn: usize, fail: usize, valid: bool) -> String {
    let (pass_w, warn_w, fail_w) = bar_widths(pass, warn, fail, 24);
    let bar = format!(
        "{}{}{}",
        style("█".repeat(pass_w)).green(),
        style("█".repeat(warn_w)).yellow(),
        style("█".repeat(fail_w)).red()
    );
    let verdict = if !valid {
        style("Invalid").red().bold().to_string()
    } else if warn > 0 {
        style("Valid, with warnings").yellow().bold().to_string()
    } else {
        style("Valid").green().bold().to_string()
    };
    format!(
        "{bar}  {verdict}   {} passed   {} {}   {} failed",
        style(pass).green(),
        style(warn).yellow(),
        if warn == 1 { "warning" } else { "warnings" },
        style(fail).red()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn badges_name_each_status() {
        assert!(status_line(CheckStatus::Pass, "Schema").contains("PASS"));
        assert!(status_line(CheckStatus::Pass, "Schema").contains("Schema"));
        assert!(status_line(CheckStatus::Warn, "Catalogs").contains("WARN"));
        assert!(status_line(CheckStatus::Fail, "Icons").contains("FAIL"));
    }

    #[test]
    fn bar_keeps_a_slice_for_warnings_and_failures() {
        assert_eq!(bar_widths(10, 0, 0, 24), (24, 0, 0));
        let (pass, warn, fail) = bar_widths(8, 1, 1, 24);
        assert!(warn > 0);
        assert!(fail > 0);
        assert_eq!(pass + warn + fail, 24);
    }
}
