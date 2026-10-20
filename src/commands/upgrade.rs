use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;

use anyhow::Context;
use serde_json::json;

use crate::error::CliError;
use crate::ui::Output;
use crate::update::{self, CHANGELOG_URL, INSTALL_PS1, INSTALL_SH};

pub async fn run(out: &Output) -> anyhow::Result<()> {
    let current = update::current_version();
    let latest = update::latest_version().await;

    if let Some(latest) = &latest {
        if !update::is_newer(latest, &current) {
            if out.json {
                return out.json(&json!({
                    "current": current.to_string(),
                    "latest": latest.to_string(),
                    "upgraded": false,
                }));
            }
            out.success(format!("times {current} is the latest version."));
            out.info(format!("Changelog: {CHANGELOG_URL}"));
            return Ok(());
        }
        if !out.json {
            out.info(format!("Installing times {latest}. You have {current}."));
        }
    } else if !out.json {
        out.info("Could not check GitHub for the latest version. Running the installer anyway.");
    }

    run_installer(out)?;
    let shell_ok = ensure_shell_after_upgrade();

    if out.json {
        return out.json(&json!({
            "current": current.to_string(),
            "latest": latest.map(|v| v.to_string()),
            "upgraded": true,
            "changelog": CHANGELOG_URL,
            "shellConfigured": shell_ok,
        }));
    }
    out.success(format!(
        "Upgraded to times {}.",
        latest
            .as_ref()
            .map(|v| v.to_string())
            .unwrap_or_else(|| "latest".into())
    ));
    out.info(format!("Changelog: {CHANGELOG_URL}"));
    if !shell_ok {
        out.warn("Could not update shellenv. Run: times setup-shell");
    }
    out.info("If this shell still shows the old version, run: . ~/.config/times/shellenv");
    Ok(())
}

fn ensure_shell_after_upgrade() -> bool {
    if cfg!(windows) {
        return false;
    }
    let Ok(config_dir) = crate::config::config_dir() else {
        return false;
    };
    crate::shell::apply(&config_dir, &crate::shell::default_bin_dirs()).is_ok()
}

fn run_installer(out: &Output) -> anyhow::Result<()> {
    let status = if cfg!(windows) {
        Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                &format!("irm {INSTALL_PS1} | iex"),
            ])
            .status()
            .context("Could not start PowerShell to run the times installer")?
    } else {
        let script = format!("curl -fsSL {INSTALL_SH} | sh");
        let mut child = Command::new("sh")
            .arg("-c")
            .arg(&script)
            .env("TIMES_INSTALL_QUIET", "1")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("Could not start sh to run the times installer")?;

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let show = !out.json;
        let pb = if show {
            Some(out.percent_bar("Installing"))
        } else {
            None
        };
        let log = Arc::new(Mutex::new(Vec::<String>::new()));
        let log_out = Arc::clone(&log);
        let log_err = Arc::clone(&log);
        let pb_out = pb.clone();
        let pb_err = pb.clone();

        let out_thread = thread::spawn(move || {
            if let Some(pipe) = stdout {
                for line in BufReader::new(pipe).lines().map_while(Result::ok) {
                    if let Some(bar) = &pb_out {
                        if let Some((n, total)) = parse_progress_line(&line) {
                            set_progress_pct(bar, n, total);
                            continue;
                        }
                    }
                    if let Ok(mut g) = log_out.lock() {
                        g.push(line);
                    }
                }
            }
        });
        let err_thread = thread::spawn(move || {
            if let Some(pipe) = stderr {
                for line in BufReader::new(pipe).lines().map_while(Result::ok) {
                    if let Some(bar) = &pb_err {
                        if let Some((n, total)) = parse_progress_line(&line) {
                            set_progress_pct(bar, n, total);
                            continue;
                        }
                    }
                    if let Ok(mut g) = log_err.lock() {
                        g.push(line);
                    }
                }
            }
        });
        let status = child
            .wait()
            .context("Could not wait for the times installer")?;
        let _ = out_thread.join();
        let _ = err_thread.join();
        if let Some(bar) = pb {
            if status.success() {
                bar.set_position(100);
            }
            bar.finish_and_clear();
        }
        if !status.success() {
            let lines = log.lock().map(|g| g.clone()).unwrap_or_default();
            let tail = tail_lines(&lines, 40);
            if !tail.is_empty() {
                eprintln!("{tail}");
            }
        }
        status
    };
    if !status.success() {
        return Err(CliError::network(format!(
            "The installer exited with {}. Changelog: {CHANGELOG_URL}",
            status
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "signal".into())
        )));
    }
    Ok(())
}

fn set_progress_pct(bar: &indicatif::ProgressBar, n: u64, total: u64) {
    let pct = if total == 0 {
        0
    } else {
        ((n.saturating_mul(100)) / total).min(100)
    };
    bar.set_position(pct);
}

/// Parse `PROGRESS n/N` from install.sh (quiet mode).
pub fn parse_progress_line(line: &str) -> Option<(u64, u64)> {
    let rest = line.trim().strip_prefix("PROGRESS ")?;
    let (a, b) = rest.split_once('/')?;
    let n = a.trim().parse().ok()?;
    let total = b.trim().parse().ok()?;
    if total == 0 {
        return None;
    }
    Some((n, total))
}

fn tail_lines(lines: &[String], max: usize) -> String {
    let start = lines.len().saturating_sub(max);
    lines[start..].join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_progress_lines() {
        assert_eq!(parse_progress_line("PROGRESS 1/5"), Some((1, 5)));
        assert_eq!(parse_progress_line("  PROGRESS 3/5  "), Some((3, 5)));
        assert_eq!(parse_progress_line("PROGRESS 5/5"), Some((5, 5)));
        assert_eq!(parse_progress_line("times installer: done."), None);
        assert_eq!(parse_progress_line("PROGRESS 0/0"), None);
    }

    #[test]
    fn tail_keeps_last_n() {
        let lines: Vec<String> = (1..=5).map(|i| i.to_string()).collect();
        assert_eq!(tail_lines(&lines, 2), "4\n5");
    }
}
