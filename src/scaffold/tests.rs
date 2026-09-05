use super::*;
use crate::core::config::{
    Language, NewAlefConfig, PythonConfig, ResolvedCrateConfig, ScaffoldCargoTargets, ScaffoldConfig,
};
use std::path::{Path, PathBuf};

fn test_config() -> ResolvedCrateConfig {
    test_config_from_toml("")
}

fn test_config_from_toml(extra_crate_config: &str) -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(&format!(
        r#"
[workspace]
languages = ["python", "node"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.scaffold]
description = "Test library"
license = "MIT"
repository = "https://github.com/test/my-lib"
authors = ["Alice"]
keywords = ["test"]
{extra_crate_config}
"#,
    ))
    .expect("valid toml");
    cfg.resolve().expect("resolve ok").remove(0)
}

fn minimal_config_from_toml(extra_crate_config: &str) -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(&format!(
        r#"
[workspace]
languages = ["python", "node"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]
{extra_crate_config}
"#,
    ))
    .expect("valid toml");
    cfg.resolve().expect("resolve ok").remove(0)
}

fn test_api() -> ApiSurface {
    ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::BTreeMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    }
}

/// Filter out project-level scaffold files (like poly.toml)
/// to isolate language-specific scaffold tests.
fn language_files(files: &[GeneratedFile]) -> Vec<&GeneratedFile> {
    files
        .iter()
        .filter(|f| {
            // `.ends_with("/LICENSE")`/`.ends_with(".cargo/config.toml")` below are
            // forward-slash-shaped suffix checks, so `f.path` must be rendered portably rather
            // than with `to_string_lossy()`'s host separator -- see
            // `crate::test_support::portable_path_string`'s doc (alef task #527). ~keep
            let p = crate::test_support::portable_path_string(&f.path);
            p != "poly.toml"
                && p != "rustfmt.toml"
                && !p.ends_with("rust-toolchain.toml")
                && !p.ends_with(".cargo/config.toml")
                && p != ".gitattributes"
                && p != ".clang-format"
                // LICENSE files are synced from the workspace root; the consolidated
                // single-crate layout runs tests from the repo root which has a LICENSE
                // file, causing scaffold_license_files() to emit per-package LICENSE
                && !p.ends_with("/LICENSE")
                && p != "LICENSE"
        })
        .collect()
}

mod cargo_config;
mod cargo_table_order;
mod core_deps;
mod dependency_key_order;
mod extra_deps;
mod ffi_go_java_ruby;
mod general;
mod java_checkstyle;
mod java_pom_compiler;
mod language_elixir;
mod language_php_dart;
mod language_swift_kotlin_gleam_zig;
mod licenses;
mod marker_stamping;
mod poly;
mod poly_migrations;
mod poly_schema_exclude;
mod python_node;
mod repair;
mod unstamped_cache_miss;
mod workspace_inheritance;
