use std::process::Command;

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
            out.info(format!("Changelog: {CHANGELOG_URL}"));
        }
    } else if !out.json {
        out.info("Could not check GitHub for the latest version. Running the installer anyway.");
    }

    run_installer()?;
    let _ = ensure_shell_after_upgrade();

    if out.json {
        return out.json(&json!({
            "current": current.to_string(),
            "latest": latest.map(|v| v.to_string()),
            "upgraded": true,
            "changelog": CHANGELOG_URL,
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
    Ok(())
}

fn ensure_shell_after_upgrade() -> bool {
    if cfg!(windows) {
        return false;
    }
    let Ok(config_dir) = crate::config::config_dir() else {
        return false;
    };
    // Always rewrite shellenv so PATH priority fixes ship on upgrade, not only
    // the first install.
    crate::shell::apply(&config_dir, &crate::shell::default_bin_dirs()).is_ok()
}

fn run_installer() -> anyhow::Result<()> {
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
        Command::new("sh")
            .arg("-c")
            .arg(&script)
            .env("TIMES_INSTALL_QUIET", "1")
            .status()
            .context("Could not start sh to run the times installer")?
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
