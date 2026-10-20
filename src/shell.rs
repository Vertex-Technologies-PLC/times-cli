use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::Serialize;

pub const RC_MARKER: &str = "# times-cli shellenv";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupReport {
    pub shellenv: PathBuf,
    pub rc_files: Vec<PathBuf>,
}

pub fn shellenv_body(bin_dirs: &[PathBuf]) -> String {
    let mut out = String::from("# TIMES CLI. Prefer this binary over the POSIX `times` builtin.\n");
    // Always move each bin dir to the front, even if it already appears later in PATH
    // (e.g. ~/.cargo/bin/times must not win over ~/.local/bin/times).
    for dir in bin_dirs {
        let dir = dir.display();
        out.push_str(&format!(
            "_times_path=\"\"\n\
_times_IFS=\"$IFS\"\n\
IFS=:\n\
for _times_p in $PATH; do\n\
  [ \"$_times_p\" = \"{dir}\" ] && continue\n\
  if [ -z \"$_times_path\" ]; then\n\
    _times_path=\"$_times_p\"\n\
  else\n\
    _times_path=\"$_times_path:$_times_p\"\n\
  fi\n\
done\n\
IFS=\"$_times_IFS\"\n\
PATH=\"{dir}${{_times_path:+:$_times_path}}\"\n\
unset _times_path _times_IFS _times_p\n"
        ));
    }
    if !bin_dirs.is_empty() {
        out.push_str("export PATH\n");
    }
    out.push_str(
        "if [ -n \"${ZSH_VERSION:-}\" ]; then\n  disable times >/dev/null 2>&1 || true\nelif [ -n \"${BASH_VERSION:-}\" ]; then\n  enable -n times >/dev/null 2>&1 || true\nfi\n",
    );
    out
}

pub fn default_bin_dirs() -> Vec<PathBuf> {
    // Each dir is prepended in order; the last entry wins PATH. Put
    // ~/.local/bin last so the installer binary beats ~/.cargo/bin/times.
    let mut dirs = Vec::new();
    if let Ok(exe) = env::current_exe() {
        if let Some(parent) = exe.parent() {
            dirs.push(parent.to_path_buf());
        }
    }
    if let Some(home) = home_dir() {
        let local = home.join(".local").join("bin");
        dirs.retain(|d| d != &local);
        dirs.push(local);
    }
    dirs
}

pub fn apply(config_dir: &Path, bin_dirs: &[PathBuf]) -> anyhow::Result<SetupReport> {
    fs::create_dir_all(config_dir)
        .with_context(|| format!("Could not create {}", config_dir.display()))?;
    let shellenv = config_dir.join("shellenv");
    fs::write(&shellenv, shellenv_body(bin_dirs))
        .with_context(|| format!("Could not write {}", shellenv.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&shellenv, fs::Permissions::from_mode(0o644))?;
    }

    let snippet = rc_snippet(&shellenv);
    let mut rc_files = Vec::new();
    for path in rc_targets()? {
        append_once(&path, &snippet)?;
        rc_files.push(path);
    }
    Ok(SetupReport { shellenv, rc_files })
}

pub fn is_configured(config_dir: &Path) -> bool {
    let shellenv = config_dir.join("shellenv");
    if !shellenv.is_file() {
        return false;
    }
    let Ok(targets) = rc_targets() else {
        return false;
    };
    targets.iter().any(|path| {
        path.is_file()
            && fs::read_to_string(path)
                .map(|text| text.contains(RC_MARKER))
                .unwrap_or(false)
    })
}

pub fn ensure(config_dir: &Path) -> anyhow::Result<Option<SetupReport>> {
    let bin_dirs = default_bin_dirs();
    let expected = shellenv_body(&bin_dirs);
    let shellenv = config_dir.join("shellenv");
    let stale = !shellenv.is_file()
        || fs::read_to_string(&shellenv)
            .map(|text| text != expected)
            .unwrap_or(true);
    if is_configured(config_dir) {
        if stale {
            // RC hooks already exist; refresh shellenv body (PATH order fixes).
            apply(config_dir, &bin_dirs)?;
        }
        return Ok(None);
    }
    Ok(Some(apply(config_dir, &bin_dirs)?))
}

fn rc_snippet(shellenv: &Path) -> String {
    let path = shellenv.display();
    format!("{RC_MARKER}\n[ -f \"{path}\" ] && . \"{path}\"\n")
}

fn rc_targets() -> anyhow::Result<Vec<PathBuf>> {
    let home = home_dir().context("Could not find your home directory")?;
    let shell = env::var("SHELL").unwrap_or_default();
    let mut files = Vec::new();
    let zsh = shell.contains("zsh") || shell.is_empty();
    let bash = shell.contains("bash");

    for name in [".zshenv", ".zshrc", ".zprofile", ".bashrc", ".bash_profile"] {
        let path = home.join(name);
        if path.exists() {
            files.push(path);
        }
    }

    if zsh {
        push_unique(&mut files, home.join(".zshenv"));
        push_unique(&mut files, home.join(".zshrc"));
    }
    if bash {
        push_unique(&mut files, home.join(".bashrc"));
    }
    if files.is_empty() {
        push_unique(&mut files, home.join(".profile"));
    }
    Ok(files)
}

fn append_once(path: &Path, snippet: &str) -> anyhow::Result<bool> {
    if path.exists() {
        let existing = fs::read_to_string(path)
            .with_context(|| format!("Could not read {}", path.display()))?;
        if existing.contains(RC_MARKER) {
            return Ok(false);
        }
    } else if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Could not create {}", parent.display()))?;
    }
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("Could not update {}", path.display()))?;
    if path.metadata()?.len() > 0 {
        f.write_all(b"\n")?;
    }
    f.write_all(snippet.as_bytes())
        .with_context(|| format!("Could not update {}", path.display()))?;
    Ok(true)
}

fn push_unique(files: &mut Vec<PathBuf>, path: PathBuf) {
    if !files.iter().any(|p| p == &path) {
        files.push(path);
    }
}

fn home_dir() -> Option<PathBuf> {
    env::var_os("HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| directories::BaseDirs::new().map(|d| d.home_dir().to_path_buf()))
}

pub fn inject_completion_guard(shell: clap_complete::Shell, script: &str) -> String {
    match shell {
        clap_complete::Shell::Zsh => {
            inject_after_first_line(script, "disable times >/dev/null 2>&1 || true")
        }
        clap_complete::Shell::Bash => {
            format!("enable -n times >/dev/null 2>&1 || true\n{script}")
        }
        _ => script.to_string(),
    }
}

fn inject_after_first_line(script: &str, extra: &str) -> String {
    match script.split_once('\n') {
        Some((first, rest)) => format!("{first}\n{extra}\n{rest}"),
        None => format!("{script}\n{extra}\n"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn body_disables_builtin_and_prefixes_path() {
        let body = shellenv_body(&[PathBuf::from("/tmp/times-bin")]);
        assert!(body.contains("disable times"));
        assert!(body.contains("enable -n times"));
        assert!(body.contains("/tmp/times-bin"));
        assert!(body.contains("PATH=\"/tmp/times-bin${_times_path:+:$_times_path}\""));
    }

    #[test]
    fn local_bin_is_last_so_it_wins_path() {
        let dirs = default_bin_dirs();
        let local = home_dir()
            .expect("home")
            .join(".local")
            .join("bin");
        assert_eq!(dirs.last(), Some(&local));
    }

    #[test]
    fn append_is_idempotent() {
        let dir = TempDir::new().unwrap();
        let rc = dir.path().join(".zshrc");
        fs::write(&rc, "export EDITOR=vim\n").unwrap();
        let snippet = rc_snippet(&dir.path().join("shellenv"));
        assert!(append_once(&rc, &snippet).unwrap());
        assert!(!append_once(&rc, &snippet).unwrap());
        let text = fs::read_to_string(&rc).unwrap();
        assert_eq!(text.matches(RC_MARKER).count(), 1);
        assert!(text.contains("export EDITOR=vim"));
    }

    #[test]
    fn zsh_guard_keeps_compdef_first() {
        let script = "#compdef times\n_times() {}\n";
        let out = inject_completion_guard(clap_complete::Shell::Zsh, script);
        assert!(out.starts_with("#compdef times\n"));
        assert!(out.contains("disable times"));
    }

    #[test]
    fn ensure_is_idempotent() {
        let home = TempDir::new().unwrap();
        let config = home.path().join("config");
        let prev_home = env::var_os("HOME");
        env::set_var("HOME", home.path());
        let zshrc = home.path().join(".zshrc");
        fs::write(&zshrc, "").unwrap();
        assert!(ensure(&config).unwrap().is_some());
        assert!(is_configured(&config));
        assert!(ensure(&config).unwrap().is_none());
        match prev_home {
            Some(v) => env::set_var("HOME", v),
            None => env::remove_var("HOME"),
        }
    }

    #[test]
    fn ensure_rewrites_stale_shellenv() {
        let home = TempDir::new().unwrap();
        let config = home.path().join("config");
        let prev_home = env::var_os("HOME");
        env::set_var("HOME", home.path());
        fs::write(home.path().join(".zshrc"), "").unwrap();
        assert!(ensure(&config).unwrap().is_some());
        let shellenv = config.join("shellenv");
        fs::write(&shellenv, "# stale shellenv\n").unwrap();
        assert!(ensure(&config).unwrap().is_none());
        let body = fs::read_to_string(&shellenv).unwrap();
        assert!(body.contains(".local/bin"));
        assert!(!body.contains("# stale shellenv"));
        match prev_home {
            Some(v) => env::set_var("HOME", v),
            None => env::remove_var("HOME"),
        }
    }
}
