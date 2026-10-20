//! Names skipped when packing AIP source for depot upload / local zip.

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
        || name == ".env"
        || name.starts_with(".env.")
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
        assert!(!should_skip_pack_file("package.json"));
    }
}
