//! Shared helpers used during config resolution.

use std::collections::HashMap;
use std::path::PathBuf;

use super::extras::Language;
use super::output::{OutputConfig, OutputTemplate};
use super::raw_crate::RawCrateConfig;

/// Compute resolved output paths for a crate: per-crate explicit wins; else use template.
pub(crate) fn resolve_output_paths(
    krate: &RawCrateConfig,
    template: &OutputTemplate,
    languages: &[Language],
    multi_crate: bool,
) -> HashMap<String, PathBuf> {
    let mut paths = HashMap::new();
    for lang in languages {
        let lang_str = lang.to_string();
        // Per-crate explicit output path wins over the workspace template.
        let explicit = per_crate_explicit_output(&krate.output, lang);
        let path = explicit
            .map(PathBuf::from)
            .unwrap_or_else(|| template.resolve(&krate.name, &lang_str, multi_crate));
        paths.insert(lang_str, path);
    }
    paths
}

/// Extract an explicit per-crate output path for a language from [`OutputConfig`].
pub(crate) fn per_crate_explicit_output(output: &OutputConfig, lang: &Language) -> Option<String> {
    let path = match lang {
        Language::Python => output.python.as_ref(),
        Language::Node => output.node.as_ref(),
        Language::Ruby => output.ruby.as_ref(),
        Language::Php => output.php.as_ref(),
        Language::Elixir => output.elixir.as_ref(),
        Language::Wasm => output.wasm.as_ref(),
        Language::Ffi => output.ffi.as_ref(),
        Language::Gleam => output.gleam.as_ref(),
        Language::Go => output.go.as_ref(),
        Language::Java => output.java.as_ref(),
        Language::Kotlin => output.kotlin.as_ref(),
        Language::Dart => output.dart.as_ref(),
        Language::Swift => output.swift.as_ref(),
        Language::Csharp => output.csharp.as_ref(),
        Language::R => output.r.as_ref(),
        Language::Zig => output.zig.as_ref(),
        Language::Rust => None,
    };
    path.map(|p| p.to_string_lossy().into_owned())
}

/// Merge two HashMaps: per-crate values win; workspace values fill in missing keys.
pub(crate) fn merge_map<V: Clone>(
    workspace: &HashMap<String, V>,
    per_crate: &HashMap<String, V>,
) -> HashMap<String, V> {
    let mut merged = workspace.clone();
    for (k, v) in per_crate {
        merged.insert(k.clone(), v.clone());
    }
    merged
}

/// Helper function to resolve output directory path from config.
/// Replaces {name} placeholder with the crate name.
pub fn resolve_output_dir(config_path: Option<&PathBuf>, crate_name: &str, default: &str) -> String {
    config_path
        .map(|p| p.to_string_lossy().replace("{name}", crate_name))
        .unwrap_or_else(|| default.replace("{name}", crate_name))
}

/// Detect whether `serde` and `serde_json` are available in a binding crate's Cargo.toml.
///
/// `output_dir` is the generated source directory (e.g., `crates/spikard-py/src/`).
/// The function walks up to find the crate's Cargo.toml and checks its `[dependencies]`
/// for both `serde` and `serde_json`.
pub fn detect_serde_available(output_dir: &str) -> bool {
    let src_path = std::path::Path::new(output_dir);
    // Walk up from the output dir to find Cargo.toml (usually output_dir is `crates/foo/src/`)
    let mut dir = src_path;
    loop {
        let cargo_toml = dir.join("Cargo.toml");
        if cargo_toml.exists() {
            return cargo_toml_has_serde(&cargo_toml);
        }
        match dir.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => dir = parent,
            _ => break,
        }
    }
    false
}

/// Check if a Cargo.toml has both `serde` (with derive feature) and `serde_json` in its dependencies.
///
/// The `serde::Serialize` derive macro requires `serde` as a direct dependency with the `derive`
/// feature enabled. Having only `serde_json` is not sufficient since it only pulls in `serde`
/// transitively without the derive proc-macro.
fn cargo_toml_has_serde(path: &std::path::Path) -> bool {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return false,
    };

    let has_serde_json = content.contains("serde_json");
    // Check for `serde` as a direct dependency (not just serde_json).
    // Must match "serde" as a TOML key, not as a substring of "serde_json".
    // Valid patterns: `serde = `, `serde.`, `[dependencies.serde]`
    let has_serde_dep = content.lines().any(|line| {
        let trimmed = line.trim();
        // Match `serde = ...` or `serde.workspace = true` etc., but not `serde_json`
        trimmed.starts_with("serde ")
            || trimmed.starts_with("serde=")
            || trimmed.starts_with("serde.")
            || trimmed == "[dependencies.serde]"
    });

    has_serde_json && has_serde_dep
}

/// Find the path segment that comes after a `crates/` component.
///
/// Handles both absolute paths (e.g., `/workspace/repo/crates/foo/src/lib.rs`)
/// and relative paths (e.g., `crates/foo/src/lib.rs`).  Returns the slice
/// starting immediately after the `crates/` prefix, or `None` if the path
/// does not contain such a component.
pub(crate) fn find_after_crates_prefix(path: &str) -> Option<&str> {
    // Normalise to forward slashes for cross-platform matching.
    // We search for `/crates/` (with leading slash) first, then fall back to
    // a leading `crates/` for relative paths that start with that component.
    if let Some(pos) = path.find("/crates/") {
        return Some(&path[pos + "/crates/".len()..]);
    }
    if let Some(stripped) = path.strip_prefix("crates/") {
        return Some(stripped);
    }
    None
}
