use std::io::{self, Write};
use std::path::Path;
use std::process::{Command, Stdio};

use serde_json::Value;

use crate::error::CliError;

pub fn ensure_output(root: &Path, no_build: bool) -> anyhow::Result<()> {
    if no_build {
        return Ok(());
    }
    let pm = detect_package_manager(root);
    let package = read_package_json(root)?;
    if package
        .pointer("/scripts/build")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .is_none()
    {
        return Err(CliError::validation(
            "Build folder is missing and package.json has no scripts.build. Add a build script, run it yourself, or pass --dir.",
        ));
    }
    eprintln!("Running `{pm} run build` because the guest output folder is missing.");
    let mut child = Command::new(pm)
        .args(["run", "build"])
        .current_dir(root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            CliError::network(format!(
                "failed to start `{pm} run build`. Install {pm} or pass --dir / --no-build. {e}"
            ))
        })?;
    let mut stdout = child.stdout.take().expect("piped stdout");
    let mut stderr = child.stderr.take().expect("piped stderr");
    let pump_out = std::thread::spawn(move || {
        let _ = io::copy(&mut stdout, &mut io::stderr());
    });
    let pump_err = std::thread::spawn(move || {
        let _ = io::copy(&mut stderr, &mut io::stderr());
    });
    let status = child
        .wait()
        .map_err(|e| CliError::network(format!("`{pm} run build` could not be waited on: {e}")))?;
    let _ = pump_out.join();
    let _ = pump_err.join();
    let _ = io::stderr().flush();
    if !status.success() {
        let code = status.code().unwrap_or(1);
        return Err(CliError::network(format!(
            "`{pm} run build` failed with exit {code}"
        )));
    }
    Ok(())
}

pub fn detect_package_manager(root: &Path) -> &'static str {
    if root.join("bun.lock").is_file() || root.join("bun.lockb").is_file() {
        "bun"
    } else if root.join("pnpm-lock.yaml").is_file() {
        "pnpm"
    } else {
        "npm"
    }
}

fn read_package_json(root: &Path) -> anyhow::Result<Value> {
    let path = root.join("package.json");
    if !path.is_file() {
        return Err(CliError::validation(
            "Build folder is missing and there is no package.json to run. Pass --dir after building, or add a build script.",
        ));
    }
    let raw = std::fs::read_to_string(&path)?;
    serde_json::from_str(&raw)
        .map_err(|e| CliError::validation(format!("package.json is not valid JSON: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_lockfiles() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(detect_package_manager(dir.path()), "npm");
        std::fs::write(dir.path().join("pnpm-lock.yaml"), "").unwrap();
        assert_eq!(detect_package_manager(dir.path()), "pnpm");
        std::fs::write(dir.path().join("bun.lock"), "").unwrap();
        assert_eq!(detect_package_manager(dir.path()), "bun");
    }
}
