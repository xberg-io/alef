use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

use super::FfiTargetDepOverride;

/// Target triples assumed when a consumer sets no explicit `nif_targets`.
///
/// Windows is **msvc, not gnu**. `RustlerPrecompiled.target/3` defaults the ABI to `"msvc"` on
/// `{:win32, _}` and then rejects any triple absent from the declared list, so a `-gnu` default
/// makes every Windows install fail with "precompiled NIF is not available for this target" --
/// while CI happily builds, checksums and uploads the msvc artifact nobody can reach. alef's own
/// `scaffold::cargo_config` emits `[target.x86_64-pc-windows-msvc]`, so the `-gnu` default also
/// contradicted the toolchain alef scaffolds.
///
/// Upstream `RustlerPrecompiled`'s own default lists both Windows ABIs, but a consumer must
/// declare exactly what its CI builds: checksum generation walks this list and fails on a triple
/// with no artifact. One entry per built target, no aspirational ones. ~keep
pub(crate) const DEFAULT_NIF_TARGETS: [&str; 4] = [
    "aarch64-apple-darwin",
    "aarch64-unknown-linux-gnu",
    "x86_64-unknown-linux-gnu",
    "x86_64-pc-windows-msvc",
];

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ElixirConfig {
    pub app_name: Option<String>,
    #[serde(default)]
    pub features: Option<Vec<String>>,
    /// NIF crate `[features]` to forward to the core crate. If not set, defaults to
    /// the core crate's own declared `[features] default = [...]` list (read from its
    /// `Cargo.toml`), so a consumer whose core crate declares no defaults gets no
    /// implicit forwarding. Set to an empty list to disable default feature forwarding
    /// outright regardless of what the core crate declares.
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
    /// input. When empty, falls back to [`DEFAULT_NIF_TARGETS`]:
    /// `aarch64-apple-darwin, aarch64-unknown-linux-gnu, x86_64-unknown-linux-gnu, x86_64-pc-windows-msvc`.
    /// Windows is msvc, not gnu -- `RustlerPrecompiled` resolves the msvc triple at install time
    /// and rejects any triple this list omits.
    #[serde(default)]
    pub nif_targets: Vec<String>,
    /// Per-target overrides for the core-crate dependency emitted into the
    /// generated `Cargo.toml`. See [`super::FfiConfig::target_dep_overrides`].
    #[serde(default)]
    pub target_dep_overrides: Vec<FfiTargetDepOverride>,
    /// Feature names that should be declared as opt-in flags in the generated NIF
    /// crate's `[features]` table but excluded from its `default = [...]` array and
    /// from the core-crate dependency's own explicit `features = [...]` list.
    ///
    /// `scaffold_elixir_cargo` otherwise puts every name `collect_cfg_features` finds
    /// straight into the wrapper's unconditional `default = [...]` list, which
    /// forwards `<name> = ["<core-crate>/<name>"]` on every platform. Cargo unions
    /// feature sets across every dependency edge to the same resolved package, so
    /// that unconditional forwarding re-enables a feature a [`Self::target_dep_overrides`]
    /// entry excluded for a specific `cfg` target -- defeating the override. Listing
    /// the name here keeps it declared (so `cargo build --features <name>` still
    /// works) without auto-enabling it, the same tradeoff
    /// `RubyConfig::excluded_default_features` makes for the Magnus crate. ~keep
    #[serde(default)]
    pub excluded_default_features: Vec<String>,
}
