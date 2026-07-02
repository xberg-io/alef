use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

use super::FfiTargetDepOverride;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ElixirConfig {
    pub app_name: Option<String>,
    #[serde(default)]
    pub features: Option<Vec<String>>,
    /// NIF crate `[features]` to forward to the core crate. If empty or not set,
    /// defaults to ["download", "serde", "config"]. Set to an empty list to
    /// disable default feature forwarding (e.g., when the core crate does not
    /// have these features).
    #[serde(default)]
    pub nif_features: Option<Vec<String>>,
    /// Override the serde rename_all strategy for JSON field names (e.g. "camelCase", "snake_case").
    /// When set, this takes priority over the IR type-level serde_rename_all.
    #[serde(default)]
    pub serde_rename_all: Option<String>,
    /// Functions to exclude from Elixir NIF generation.
    #[serde(default)]
    pub exclude_functions: Vec<String>,
    /// Types to exclude from Elixir NIF generation.
    #[serde(default)]
    pub exclude_types: Vec<String>,
    /// Additional Cargo dependencies for this language's binding crate only.
    #[serde(default)]
    #[schemars(with = "HashMap<String, serde_json::Value>")]
    pub extra_dependencies: HashMap<String, toml::Value>,
    /// Override the scaffold output directory for this language's Cargo.toml and package files.
    #[serde(default)]
    pub scaffold_output: Option<PathBuf>,
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
    /// Functions that should be scheduled on the dirty CPU scheduler.
    /// HTML parsing and other CPU-intensive NIFs should be listed here to avoid
    /// blocking BEAM scheduler threads.
    #[serde(default)]
    pub cpu_bound_functions: Vec<String>,
    /// Rust target triples for which precompiled NIFs are uploaded to the
    /// GitHub release. `RustlerPrecompiled` reads this list to know which
    /// archives to download at install time. Must agree with the consumer's
    /// CI build matrix and the `generate-elixir-checksums` action's targets
    /// input. When empty, falls back to the historical default of
    /// `aarch64-apple-darwin, aarch64-unknown-linux-gnu, x86_64-unknown-linux-gnu, x86_64-pc-windows-gnu`.
    #[serde(default)]
    pub nif_targets: Vec<String>,
    /// Per-target overrides for the core-crate dependency emitted into the
    /// generated `Cargo.toml`. See [`super::FfiConfig::target_dep_overrides`].
    #[serde(default)]
    pub target_dep_overrides: Vec<FfiTargetDepOverride>,
}
