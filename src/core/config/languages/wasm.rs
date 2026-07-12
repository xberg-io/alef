use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WasmConfig {
    /// npm package name for the WASM package. Defaults to the Node package
    /// name with a trailing `-node` removed, plus `-wasm`.
    #[serde(default)]
    pub package_name: Option<String>,
    #[serde(default)]
    pub exclude_functions: Vec<String>,
    #[serde(default)]
    pub exclude_types: Vec<String>,
    #[serde(default)]
    pub type_overrides: HashMap<String, String>,
    #[serde(default)]
    pub features: Option<Vec<String>>,
    /// Override the serde rename_all strategy for JSON field names (e.g. "camelCase", "snake_case").
    /// When set, this takes priority over the IR type-level serde_rename_all.
    #[serde(default)]
    pub serde_rename_all: Option<String>,
    /// Prefix for generated type names (e.g. "Wasm" produces `WasmConversionOptions`).
    /// Defaults to `"Wasm"`.
    #[serde(default)]
    pub type_prefix: Option<String>,
    /// Wide-character C functions to shim for WASM external scanner interop.
    #[serde(default)]
    pub env_shims: Vec<String>,
    /// Additional Cargo dependencies for the WASM binding crate only.
    #[serde(default)]
    #[schemars(with = "HashMap<String, serde_json::Value>")]
    pub extra_dependencies: HashMap<String, toml::Value>,
    /// Per-field name remapping for this language. Key is `TypeName.field_name`, value is the
    /// desired binding field name. Applied after automatic keyword escaping.
    #[serde(default)]
    pub rename_fields: HashMap<String, String>,
    /// Prefix wrapper for default tool invocations. When set, prepends this string to default
    /// commands across all pipelines (lint, test, build, etc.).
    #[serde(default)]
    pub run_wrapper: Option<String>,
    /// Extra paths to append to default lint commands (format, check, typecheck).
    #[serde(default)]
    pub extra_lint_paths: Vec<String>,
    /// Overrides the default `crates/{name}-wasm` formula for the crate directory.
    /// Useful when the Rust crate directory does not follow the alef convention
    /// (e.g., consumer strips a `-rs` suffix so actual crate dir is `crates/sample-markdown-wasm`
    /// instead of `crates/sample-markdown-rs-wasm`). Used by setup, test, clean, and format tasks.
    #[serde(default)]
    pub crate_dir: Option<String>,
    /// Override the core Cargo dependency name and path for the WASM binding crate.
    /// When set, the binding `Cargo.toml` depends on this crate (resolved as
    /// `../<override>`) instead of the umbrella `[crate.name]`. Use this to point
    /// the WASM binding at a wasm-safe sub-crate while other languages keep the
    /// facade. Defaults to unset.
    #[serde(default)]
    pub core_crate_override: Option<String>,
    /// Keys to subtract from the merged `extra_dependencies` set for this
    /// language only. Useful when `[crate.extra_dependencies]` lists sibling
    /// crates that the WASM target cannot link.
    #[serde(default)]
    pub exclude_extra_dependencies: Vec<String>,
    /// Hand-written Rust modules to declare in the generated lib.rs with `pub mod <name>;`
    /// and re-export with `pub use <name>::*;`. Separate from `[custom_modules].wasm` which
    /// only adds TypeScript `export *` re-exports. Use this for Rust-side dispatch/glue modules.
    #[serde(default)]
    pub custom_rust_modules: Vec<String>,
    /// Per-type field exclusions for the generated From impls and binding struct.
    /// Key is the type name (e.g. "ServerConfig"), value is a list of field names to skip.
    /// Use when source fields are gated behind `#[cfg(not(target_arch = "wasm32"))]` and
    /// therefore don't exist in the wasm32 compilation environment.
    #[serde(default)]
    pub exclude_fields: HashMap<String, Vec<String>>,
    /// Source crate names whose types are re-exported by the `core_crate_override`
    /// crate. References to `<original_crate>::TypeName` in generated code are
    /// rewritten to `<override_crate>::TypeName`. Only meaningful when
    /// `core_crate_override` is set.
    /// Example: with `core_crate_override = "mylib-http"`, setting
    /// `source_crate_remaps = ["mylib-core", "mylib"]` rewrites
    /// `mylib_core::Method` and `mylib::Method` references to
    /// `mylib_http::Method` (assumes `mylib-http` re-exports them via
    /// `pub use mylib_core::*`).
    #[serde(default)]
    pub source_crate_remaps: Vec<String>,
    /// List of static-compiled languages supported by WASM.
    /// When set, e2e smoke tests will auto-skip for languages not in this list,
    /// emitting `.skip("not in WASM's static language set")` for each unsupported language.
    /// This bridges the gap between the full 305-language pack and the 8-language
    /// WASM build compiled with `SAMPLE_LANGUAGES=python,rust,javascript,typescript,go,html,css,json`.
    /// Defaults to empty (all languages assumed supported).
    #[serde(default)]
    pub languages: Vec<String>,
    /// wasm-pack build targets to generate and publish. Each entry produces a
    /// `pkg/<target>` build plus a `build:wasm:<target>` script; `build:all`
    /// builds exactly this set, and `files`/`main`/`module`/`types` are derived
    /// from it. Every target embeds its own full copy of the wasm binary, so
    /// restricting this to a single target (e.g. `["web"]`) keeps the published
    /// npm package small — the `web` ES module is consumed by browsers, CDNs,
    /// bundlers, Deno (`npm:`), and Node 22+. Valid entries: `web`, `bundler`,
    /// `nodejs`, `deno`. Defaults to all four for backward compatibility.
    #[serde(default = "default_wasm_targets")]
    pub targets: Vec<String>,
}

/// The default wasm-pack target set: every target wasm-pack supports.
pub fn default_wasm_targets() -> Vec<String> {
    vec![
        "web".to_string(),
        "bundler".to_string(),
        "nodejs".to_string(),
        "deno".to_string(),
    ]
}
