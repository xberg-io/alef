use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use super::{FfiTargetDepOverride, StubsConfig};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RubyConfig {
    /// Per-dependency version overrides for the manifests alef regenerates for this
    /// language. Keys are ecosystem-qualified (`cargo:pyo3`, `pypi:pyrefly`, `maven:junit`,
    /// `hex:credo`, `gem:rspec`, `cran:rextendr`) and are listed, with the literal each one
    /// defaults to, by `alef::core::managed_versions`. An unknown key is rejected outright
    /// rather than ignored: silently dropping an override is the exact failure this table
    /// exists to prevent.
    #[serde(default)]
    pub dependency_versions: BTreeMap<String, String>,
    pub gem_name: Option<String>,
    pub stubs: Option<StubsConfig>,
    /// Per-language feature override. When set, these features are used instead of
    /// `[crate] features` for this language's binding crate.
    #[serde(default)]
    pub features: Option<Vec<String>>,
    /// Override the serde rename_all strategy for JSON field names (e.g. "camelCase", "snake_case").
    /// When set, this takes priority over the IR type-level serde_rename_all.
    #[serde(default)]
    pub serde_rename_all: Option<String>,
    /// Functions to exclude from Ruby binding generation.
    #[serde(default)]
    pub exclude_functions: Vec<String>,
    /// Types to exclude from Ruby binding generation.
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
    /// Per-target overrides for the core-crate dependency emitted into the
    /// generated `Cargo.toml`. See [`super::FfiConfig::target_dep_overrides`].
    #[serde(default)]
    pub target_dep_overrides: Vec<FfiTargetDepOverride>,
    /// Feature names that should be declared as opt-in flags in the generated wrapper
    /// crate's `[features]` table but excluded from its `default = [...]` array and
    /// from the core-crate dependency's own explicit `features = [...]` list.
    ///
    /// `scaffold_ruby_cargo` otherwise puts every name `collect_cfg_features` finds
    /// straight into the wrapper's unconditional `default = [...]` list, which
    /// forwards `<name> = ["<core-crate>/<name>"]` on every platform. Cargo unions
    /// feature sets across every dependency edge to the same resolved package, so
    /// that unconditional forwarding re-enables a feature a [`Self::target_dep_overrides`]
    /// entry excluded for a specific `cfg` target -- defeating the override. Listing
    /// the name here keeps it declared (so `cargo build --features <name>` still
    /// works) without auto-enabling it, the same tradeoff
    /// `SwiftConfig::excluded_default_features` makes for the swift-bridge crate. ~keep
    #[serde(default)]
    pub excluded_default_features: Vec<String>,
    /// Override the `required_ruby_version` constraint emitted into the gemspec. A single
    /// RubyGems requirement string (e.g. `">= 3.2.0"`). When unset, defaults to `">= 3.2.0"`
    /// (no upper bound, so the gem installs on Ruby 4.x). ~keep
    #[serde(default)]
    pub required_ruby_version: Option<String>,
}
