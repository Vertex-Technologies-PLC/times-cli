//! Names skipped when packing AIP source for depot upload / local zip.

use std::path::{Path, PathBuf};

/// Always skipped at any depth (never needed for a remote rebuild).
const ALWAYS_IGNORED: &[&str] = &[
    "node_modules",
    ".git",
    "target",
    ".DS_Store",
    ".times",
    ".next",
    ".nuxt",
    ".output",
    ".turbo",
    ".vercel",
    ".cache",
    ".parcel-cache",
    ".svelte-kit",
    ".pnpm-store",
    ".idea",
    ".vscode",
    "coverage",
    ".nyc_output",
    "storybook-static",
    "__pycache__",
    ".pytest_cache",
    ".mypy_cache",
    ".ruff_cache",
    ".venv",
    "venv",
    ".tox",
    ".direnv"
];

/// Framework build outputs at the project root only.
/// Nested `dist/` under `vendor/` must still be packed.
const ROOT_BUILD_ARTIFACTS: &[&str] = &["dist", "out", "build"];

pub fn should_skip_pack_entry(depth: usize, name: &str) -> bool {
    if depth == 0 {
        return false;
    }
    if ALWAYS_IGNORED.contains(&name) || name.ends_with(".zip") {
        return true;
    }
    depth == 1 && ROOT_BUILD_ARTIFACTS.contains(&name)
}

pub fn should_skip_pack_file(name: &str) -> bool {
    name == ".DS_Store"
        || name.ends_with(".zip")
        || name.ends_with(".tsbuildinfo")
        || is_dotenv_file(name)
}

pub fn is_dotenv_file(name: &str) -> bool {
    name == ".env" || name.starts_with(".env.")
}

/// Find `.env` / `.env.*` files under `root`, skipping pack-ignored directories.
pub fn find_dotenv_files(root: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    for entry in walkdir::WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            !should_skip_pack_entry(e.depth(), &e.file_name().to_string_lossy())
        })
    {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy();
        if is_dotenv_file(&name) {
            found.push(entry.into_path());
        }
    }
    found.sort();
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skips_next_and_root_out_but_keeps_nested_dist() {
        assert!(should_skip_pack_entry(1, ".next"));
        assert!(should_skip_pack_entry(1, "out"));
        assert!(should_skip_pack_entry(1, "dist"));
        assert!(should_skip_pack_entry(1, "node_modules"));
        assert!(!should_skip_pack_entry(2, "dist"));
        assert!(!should_skip_pack_entry(3, "dist"));
        assert!(!should_skip_pack_entry(1, "src"));
    }

    #[test]
    fn skips_local_junk_files() {
        assert!(should_skip_pack_file("tsconfig.tsbuildinfo"));
        assert!(should_skip_pack_file(".env.local"));
        assert!(is_dotenv_file(".env"));
        assert!(is_dotenv_file(".env.production"));
        assert!(!should_skip_pack_file("package.json"));
    }

    #[test]
    fn finds_dotenv_under_project() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".env"), "A=1").unwrap();
        std::fs::create_dir_all(dir.path().join("node_modules")).unwrap();
        std::fs::write(dir.path().join("node_modules").join(".env"), "X=1").unwrap();
        std::fs::create_dir_all(dir.path().join("config")).unwrap();
        std::fs::write(dir.path().join("config").join(".env.local"), "B=2").unwrap();
        let found = find_dotenv_files(dir.path()).unwrap();
        let names: Vec<_> = found
            .iter()
            .map(|p| p.strip_prefix(dir.path()).unwrap().to_string_lossy().replace('\\', "/"))
            .collect();
        assert_eq!(names, vec![".env".to_string(), "config/.env.local".to_string()]);
    }
}
