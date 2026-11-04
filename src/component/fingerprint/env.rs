//! The ambient build configuration folded into a fingerprint: environment variables and the
//! implicit cargo and rustup configuration files.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::component::artifact::sha256_file;

/// Variables that name a C toolchain; cc-rs reads each of them in several spellings.
const TOOL_VARS: [&str; 6] = ["CC", "CXX", "AR", "CFLAGS", "CXXFLAGS", "LDFLAGS"];

/// Variables read by cargo, rustc and the platform SDK lookups, whatever the target.
const FIXED_VARS: [&str; 13] = [
    "RUSTFLAGS",
    "CARGO_ENCODED_RUSTFLAGS",
    "CARGO_BUILD_RUSTFLAGS",
    "CARGO_BUILD_RUSTC",
    "CARGO_BUILD_RUSTC_WRAPPER",
    "CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER",
    "CARGO_BUILD_TARGET",
    "RUSTC",
    "RUSTC_WRAPPER",
    "RUSTC_WORKSPACE_WRAPPER",
    "MACOSX_DEPLOYMENT_TARGET",
    "IPHONEOS_DEPLOYMENT_TARGET",
    "SDKROOT",
];

/// The exact variable names (besides the `CARGO_PROFILE_*` and `CARGO_TARGET_*_RUSTFLAGS`
/// families) that can change a build for `target`, following the cc-rs and cargo spellings.
fn target_var_names(target: &str) -> BTreeSet<String> {
    let underscored = target.replace('-', "_");
    let mut names = FIXED_VARS
        .iter()
        .map(|name| (*name).to_string())
        .collect::<BTreeSet<_>>();
    for base in TOOL_VARS {
        names.insert(base.to_string());
        names.insert(format!("{base}_{target}"));
        names.insert(format!("{base}_{underscored}"));
        names.insert(format!("TARGET_{base}"));
        names.insert(format!("HOST_{base}"));
    }
    names.insert(format!("CARGO_TARGET_{}_LINKER", underscored.to_uppercase()));
    names
}

/// The build-affecting subset of an environment for `target`, sorted by name: `RUSTFLAGS` and
/// friends, `CARGO_PROFILE_*`, `CARGO_TARGET_*_RUSTFLAGS`, the target's linker variable, the C
/// toolchain variables (`CC`, `CXX`, `AR`, `CFLAGS`, `CXXFLAGS`, `LDFLAGS` with their `_<target>`,
/// `_<target_with_underscores>`, `TARGET_` and `HOST_` spellings), the SDK and deployment-target
/// variables, and every name in `opt_in` (from `--env`).
#[must_use]
pub fn build_env(
    vars: impl IntoIterator<Item = (String, String)>,
    target: &str,
    opt_in: &[String],
) -> Vec<(String, String)> {
    let wanted = target_var_names(target);
    let mut selected = vars
        .into_iter()
        .filter(|(name, _)| {
            wanted.contains(name)
                || opt_in.contains(name)
                || name.starts_with("CARGO_PROFILE_")
                || (name.starts_with("CARGO_TARGET_") && name.ends_with("_RUSTFLAGS"))
        })
        .collect::<Vec<_>>();
    selected.sort();
    selected
}

/// The current process environment (non-UTF-8 variables are skipped).
#[must_use]
pub fn process_vars() -> Vec<(String, String)> {
    std::env::vars_os()
        .filter_map(|(name, value)| Some((name.into_string().ok()?, value.into_string().ok()?)))
        .collect()
}

const CONFIG_FILES: [&str; 4] = [
    ".cargo/config.toml",
    ".cargo/config",
    "rust-toolchain.toml",
    "rust-toolchain",
];

/// `$CARGO_HOME`, else `~/.cargo`.
#[must_use]
pub fn cargo_home() -> Option<PathBuf> {
    let explicit = std::env::var_os("CARGO_HOME").filter(|value| !value.is_empty());
    explicit.map(PathBuf::from).or_else(|| {
        let home = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .filter(|value| !value.is_empty())?;
        Some(PathBuf::from(home).join(".cargo"))
    })
}

/// `(label, sha256)` of the cargo and rustup configuration files that apply to a build: the
/// `config.toml` / `config` in `cargo_home`, then each `.cargo/config*` and `rust-toolchain*` found
/// from each `(scope, dir)` upward to the filesystem root. Labels carry the scope and the number of
/// levels above the start (never an absolute path), so the checkout location does not change them.
///
/// # Errors
///
/// Fails when a found file cannot be read.
pub fn implicit_config(starts: &[(&str, &Path)], cargo_home: Option<&Path>) -> Result<Vec<(String, String)>> {
    collect_config(starts, cargo_home, |_| None)
}

pub(super) fn checkout_config(starts: &[(&str, &Path)], workspace_root: &Path) -> Result<Vec<(String, String)>> {
    let workspace_root = workspace_root.canonicalize()?;
    collect_config(starts, None, |start| {
        let root = start
            .ancestors()
            .find(|ancestor| ancestor.join(".git").exists())
            .or_else(|| {
                start
                    .ancestors()
                    .find(|ancestor| ancestor.canonicalize().is_ok_and(|path| path == workspace_root))
            })
            .unwrap_or(start);
        Some(root.to_path_buf())
    })
}

fn collect_config(
    starts: &[(&str, &Path)],
    cargo_home: Option<&Path>,
    checkout_root: impl Fn(&Path) -> Option<PathBuf>,
) -> Result<Vec<(String, String)>> {
    let mut found = Vec::new();
    let mut seen = BTreeSet::new();
    if let Some(home) = cargo_home {
        for name in ["config.toml", "config"] {
            let path = home.join(name);
            if path.is_file() && seen.insert(path.clone()) {
                found.push((format!("cargo-home:{name}"), sha256_file(&path)?));
            }
        }
    }
    for (scope, start) in starts {
        let start = std::path::absolute(start).unwrap_or_else(|_| start.to_path_buf());
        let root = checkout_root(&start);
        let dirs = start
            .ancestors()
            .take_while(|dir| root.as_ref().is_none_or(|root| dir.starts_with(root)));
        for (depth, dir) in dirs.enumerate() {
            for name in CONFIG_FILES {
                let path = dir.join(name);
                if path.is_file() && seen.insert(path.clone()) {
                    found.push((format!("{scope}:{depth}:{name}"), sha256_file(&path)?));
                }
            }
        }
    }
    found.sort();
    Ok(found)
}
