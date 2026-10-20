use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::Context;
use serde_json::{Map, Value};

use crate::error::CliError;

const VENDOR_DIR: &str = "vendor";

pub struct WorkspacePack {
    pub staged_root: PathBuf,
    pub vendored_packages: Vec<String>,
    _keep: tempfile::TempDir,
}

/// Progress callbacks while staging a deploy pack (human CLI only).
pub enum PackPhase {
    Copying,
    Vendoring(String),
    RegeneratingLockfile,
    Zipping,
}

pub fn stage_for_deploy(project_root: &Path) -> anyhow::Result<WorkspacePack> {
    stage_for_deploy_with(project_root, |_| {})
}

pub fn stage_for_deploy_with(
    project_root: &Path,
    mut on_phase: impl FnMut(PackPhase),
) -> anyhow::Result<WorkspacePack> {
    let project_root = project_root.canonicalize()?;
    let keep = tempfile::tempdir().context("Could not create a temp folder for the deploy pack")?;
    let staged = keep.path().join("src");
    on_phase(PackPhase::Copying);
    copy_tree(&project_root, &staged)?;

    let pkg_path = staged.join("package.json");
    if !pkg_path.is_file() {
        return Ok(WorkspacePack {
            staged_root: staged,
            vendored_packages: Vec::new(),
            _keep: keep,
        });
    }

    let mut pkg: Value = serde_json::from_str(
        &fs::read_to_string(&pkg_path)
            .with_context(|| format!("Could not read {}", pkg_path.display()))?,
    )
    .context("package.json is not valid JSON")?;

    let mut vendored_packages = Vec::new();
    for section in ["dependencies", "devDependencies", "optionalDependencies"] {
        let Some(deps) = pkg.get_mut(section).and_then(|v| v.as_object_mut()) else {
            continue;
        };
        let names: Vec<String> = deps
            .iter()
            .filter(|(_, ver)| {
                ver.as_str()
                    .is_some_and(|s| s.trim().starts_with("workspace:"))
            })
            .map(|(name, _)| name.clone())
            .collect();
        for name in names {
            on_phase(PackPhase::Vendoring(name.clone()));
            let source = resolve_workspace_package(&project_root, &name)?;
            let dest_rel = format!("{VENDOR_DIR}/{}", sanitize_pkg_dir(&name));
            let dest = staged.join(&dest_rel);
            copy_npm_package(&source, &dest)?;
            deps.insert(name.clone(), Value::String(format!("file:./{dest_rel}")));
            vendored_packages.push(name);
        }
    }

    if !vendored_packages.is_empty() {
        let pretty = serde_json::to_string_pretty(&pkg)? + "\n";
        fs::write(&pkg_path, pretty)
            .with_context(|| format!("Could not rewrite {}", pkg_path.display()))?;
        on_phase(PackPhase::RegeneratingLockfile);
        regenerate_npm_lockfile(&staged)?;
    } else {
        ensure_lockfile_present(&staged)?;
    }

    Ok(WorkspacePack {
        staged_root: staged,
        vendored_packages,
        _keep: keep,
    })
}

fn ensure_lockfile_present(staged: &Path) -> anyhow::Result<()> {
    let pkg: Value = serde_json::from_str(&fs::read_to_string(staged.join("package.json"))?)?;
    let has_deps = ["dependencies", "devDependencies", "optionalDependencies"]
        .iter()
        .any(|k| {
            pkg.get(k)
                .and_then(|v| v.as_object())
                .is_some_and(|m| !m.is_empty())
        });
    if !has_deps {
        return Ok(());
    }
    if has_lockfile(staged) {
        return Ok(());
    }
    ensure_npm_lockfile(staged)
}

fn has_lockfile(root: &Path) -> bool {
    [
        "package-lock.json",
        "npm-shrinkwrap.json",
        "bun.lock",
        "bun.lockb",
        "pnpm-lock.yaml",
    ]
    .iter()
    .any(|n| root.join(n).is_file())
}

fn ensure_npm_lockfile(staged: &Path) -> anyhow::Result<()> {
    if staged.join("package-lock.json").is_file() || staged.join("npm-shrinkwrap.json").is_file() {
        return Ok(());
    }
    regenerate_npm_lockfile(staged)
}

fn regenerate_npm_lockfile(staged: &Path) -> anyhow::Result<()> {
    for name in [
        "package-lock.json",
        "npm-shrinkwrap.json",
        "bun.lock",
        "bun.lockb",
        "pnpm-lock.yaml",
        "yarn.lock",
    ] {
        let path = staged.join(name);
        if path.is_file() {
            fs::remove_file(&path)
                .with_context(|| format!("Could not remove stale lockfile {}", path.display()))?;
        }
    }
    let output = Command::new("npm")
        .args([
            "install",
            "--package-lock-only",
            "--ignore-scripts",
            "--no-audit",
            "--no-fund",
            "--loglevel",
            "error",
        ])
        .current_dir(staged)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .context(
            "Could not run npm to generate package-lock.json. Install Node.js/npm, or commit a lockfile.",
        )?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        let detail = err.trim();
        if detail.is_empty() {
            return Err(CliError::usage(
                "npm could not generate package-lock.json for the deploy pack. Fix dependency resolution locally, then retry.",
            )
            .into());
        }
        return Err(CliError::usage(format!(
            "npm could not generate package-lock.json for the deploy pack:\n{detail}"
        ))
        .into());
    }
    if !staged.join("package-lock.json").is_file() {
        return Err(CliError::usage(
            "npm install --package-lock-only finished without writing package-lock.json.",
        )
        .into());
    }
    Ok(())
}

fn resolve_workspace_package(project_root: &Path, name: &str) -> anyhow::Result<PathBuf> {
    if let Ok(explicit) = env::var("TIMES_AIP_SDK_PATH") {
        if name == "@times-aip/aip-sdk" {
            let p = PathBuf::from(explicit);
            if package_name_matches(&p, name)? {
                return Ok(p.canonicalize()?);
            }
        }
    }

    let linked = project_root.join("node_modules").join(name);
    if package_name_matches(&linked, name)? {
        return Ok(linked.canonicalize()?);
    }

    if name == "@times-aip/aip-sdk" {
        for parent in project_root.ancestors() {
            let cand = parent.join("times-aip-sdk");
            if package_name_matches(&cand, name)? {
                return Ok(cand.canonicalize()?);
            }
        }
    }

    Err(CliError::usage(format!(
        "workspace dependency {name} could not be resolved for packing. \
Install it locally (node_modules), set TIMES_AIP_SDK_PATH for @times-aip/aip-sdk, \
or change package.json to a published version such as \"{name}\": \"^1.0.0\"."
    ))
    .into())
}

fn package_name_matches(dir: &Path, expected: &str) -> anyhow::Result<bool> {
    let pkg = dir.join("package.json");
    if !pkg.is_file() {
        return Ok(false);
    }
    let v: Value = serde_json::from_str(&fs::read_to_string(pkg)?)?;
    Ok(v.get("name").and_then(|n| n.as_str()) == Some(expected))
}

fn sanitize_pkg_dir(name: &str) -> String {
    name.trim_start_matches('@').replace('/', "-")
}

fn copy_npm_package(source: &Path, dest: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(dest).with_context(|| format!("Could not create {}", dest.display()))?;
    let pkg_json = source.join("package.json");
    let pkg: Value = serde_json::from_str(
        &fs::read_to_string(&pkg_json)
            .with_context(|| format!("Could not read {}", pkg_json.display()))?,
    )?;
    fs::copy(&pkg_json, dest.join("package.json"))?;

    let mut entries: Vec<String> = pkg
        .get("files")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let defaulted_to_dist = entries.is_empty();
    if defaulted_to_dist {
        entries.push("dist".into());
    }
    for entry in &entries {
        let from = source.join(entry);
        let to = dest.join(entry);
        if from.is_dir() {
            copy_tree(&from, &to)?;
        } else if from.is_file() {
            if let Some(parent) = to.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(&from, &to)
                .with_context(|| format!("Could not copy {}", from.display()))?;
        }
    }
    if defaulted_to_dist && !dest.join("dist").exists() {
        let name = pkg
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("package");
        return Err(CliError::usage(format!(
            "vendored {name} has no dist/ folder. Build it (e.g. npm run build in times-aip-sdk) before deploy."
        ))
        .into());
    }

    // Keep package.json exports that point at root CSS files even if omitted from "files".
    if let Some(exports) = pkg.get("exports").and_then(|v| v.as_object()) {
        copy_export_files(source, dest, exports)?;
    }
    Ok(())
}

fn copy_export_files(source: &Path, dest: &Path, exports: &Map<String, Value>) -> anyhow::Result<()> {
    for (_key, val) in exports {
        match val {
            Value::String(path) => copy_if_file(source, dest, path)?,
            Value::Object(map) => {
                for field in ["default", "import", "require", "types"] {
                    if let Some(Value::String(path)) = map.get(field) {
                        copy_if_file(source, dest, path)?;
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn copy_if_file(source: &Path, dest: &Path, rel: &str) -> anyhow::Result<()> {
    let rel = rel.trim_start_matches("./");
    if rel.is_empty() || rel == "." || rel.starts_with("dist/") {
        return Ok(());
    }
    let from = source.join(rel);
    if !from.is_file() {
        return Ok(());
    }
    let to = dest.join(rel);
    if to.is_file() {
        return Ok(());
    }
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(&from, &to)?;
    Ok(())
}

fn is_env_secret_name(name: &str) -> bool {
    name == ".env" || name.starts_with(".env.")
}

fn copy_tree(from: &Path, to: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(to)?;
    for entry in walkdir::WalkDir::new(from)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            e.depth() == 0
                || !matches!(
                    e.file_name().to_string_lossy().as_ref(),
                    "node_modules" | ".git" | "target" | ".DS_Store" | ".times"
                )
        })
    {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy();
        if is_env_secret_name(&name) {
            continue;
        }
        let rel = entry.path().strip_prefix(from)?;
        let dest = to.join(rel);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(entry.path(), &dest)
            .with_context(|| format!("Could not copy {}", entry.path().display()))?;
    }
    Ok(())
}

pub fn collect_staged_files(staged_root: &Path) -> anyhow::Result<BTreeMap<String, PathBuf>> {
    let mut files = BTreeMap::new();
    for entry in walkdir::WalkDir::new(staged_root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            e.depth() == 0
                || !matches!(
                    e.file_name().to_string_lossy().as_ref(),
                    "node_modules" | ".git" | "target" | ".DS_Store"
                )
        })
    {
        let entry = entry?;
        if entry.file_type().is_file() {
            let rel = entry
                .path()
                .strip_prefix(staged_root)?
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            if rel.ends_with(".zip") {
                continue;
            }
            files.insert(rel, entry.path().to_path_buf());
        }
    }
    if files.is_empty() {
        return Err(CliError::usage("Nothing to zip in this folder.").into());
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn vendors_workspace_aip_sdk_when_sibling_exists() {
        let sdk = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../times-aip-sdk");
        if !sdk.join("package.json").is_file() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(
            root.join("package.json"),
            r#"{"name":"demo","dependencies":{"@times-aip/aip-sdk":"workspace:*","left-pad":"1.3.0"}}"#,
        )
        .unwrap();
        // Stale workspace-era locks must not win over the regenerated npm lock.
        fs::write(root.join("bun.lock"), "workspace-lock-stub\n").unwrap();
        fs::write(
            root.join("package-lock.json"),
            r#"{"name":"demo","lockfileVersion":3,"packages":{}}"#,
        )
        .unwrap();
        env::set_var("TIMES_AIP_SDK_PATH", &sdk);
        let staged = stage_for_deploy(root).expect("stage");
        env::remove_var("TIMES_AIP_SDK_PATH");
        assert!(staged.vendored_packages.contains(&"@times-aip/aip-sdk".to_string()));
        let pkg: Value =
            serde_json::from_str(&fs::read_to_string(staged.staged_root.join("package.json")).unwrap())
                .unwrap();
        let dep = pkg["dependencies"]["@times-aip/aip-sdk"].as_str().unwrap();
        assert!(dep.starts_with("file:./vendor/"));
        assert!(staged.staged_root.join("vendor/times-aip-sdk/package.json").is_file());
        assert!(staged.staged_root.join("package-lock.json").is_file());
        assert!(!staged.staged_root.join("bun.lock").is_file());
    }

    #[test]
    fn leaves_published_deps_when_no_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(
            root.join("package.json"),
            r#"{"name":"demo","dependencies":{"left-pad":"1.3.0"}}"#,
        )
        .unwrap();
        fs::write(
            root.join("package-lock.json"),
            r#"{"name":"demo","lockfileVersion":3,"packages":{}}"#,
        )
        .unwrap();
        let staged = stage_for_deploy(root).expect("stage");
        assert!(staged.vendored_packages.is_empty());
        assert!(staged.staged_root.join("package-lock.json").is_file());
    }

    #[test]
    fn pack_skips_dotenv_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("package.json"), r#"{"name":"demo"}"#).unwrap();
        fs::write(root.join(".env"), "SECRET=1\n").unwrap();
        fs::write(root.join(".env.local"), "SECRET=2\n").unwrap();
        fs::write(root.join("readme.md"), "ok\n").unwrap();
        let staged = stage_for_deploy(root).expect("stage");
        assert!(!staged.staged_root.join(".env").exists());
        assert!(!staged.staged_root.join(".env.local").exists());
        assert!(staged.staged_root.join("readme.md").is_file());
    }

    #[test]
    fn vendor_requires_dist_when_files_omitted() {
        let dir = tempfile::tempdir().unwrap();
        let sdk = dir.path().join("sdk");
        fs::create_dir_all(&sdk).unwrap();
        fs::write(
            sdk.join("package.json"),
            r#"{"name":"@times-aip/aip-sdk","version":"0.0.0"}"#,
        )
        .unwrap();
        let err = copy_npm_package(&sdk, &dir.path().join("out")).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("no dist/"), "{msg}");
    }
}
