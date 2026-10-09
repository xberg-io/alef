use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

use super::{FfiTargetDepOverride, StubsConfig};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PhpConfig {
    pub extension_name: Option<String>,
    /// Cargo crate name for the PHP binding (e.g. `"parser-core-core-php"`).
    /// Used to derive the shared library filename in the e2e test runner.
    /// When absent, the lib name is derived from `extension_name` by appending `_php`.
    #[serde(default)]
    pub cargo_crate_name: Option<String>,
    /// Override the PHP namespace used for class registration and PSR-4 autoloading.
    ///
    /// When set, this value is used verbatim as the PHP namespace (e.g. `"SampleMarkdown"`).
    /// When absent, the namespace is derived from `extension_name` by splitting on `_` and
    /// converting each segment to PascalCase (e.g. `sample_markup` → `Sample\Markup`).
    #[serde(default)]
    pub namespace: Option<String>,
    /// Feature gate for ext-php-rs (default: "extension-module").
    /// All generated code is wrapped in `#[cfg(feature = "...")]`.
    #[serde(default)]
    pub feature_gate: Option<String>,
    /// Override composer package name (vendor/package-name) for composer.json generation.
    /// When absent, vendor/package is derived from the repository URL.
    /// Format: "vendor/package-name" (e.g., "example/sample-lib").
    #[serde(default)]
    pub composer_package: Option<String>,
    /// Output directory for generated PHP facade / stubs (e.g., `packages/php/src/`).
    #[serde(default)]
    pub stubs: Option<StubsConfig>,
    #[serde(default)]
    pub features: Option<Vec<String>>,
    /// Override the serde rename_all strategy for JSON field names (e.g. "camelCase", "snake_case").
    /// When set, this takes priority over the IR type-level serde_rename_all.
    #[serde(default)]
    pub serde_rename_all: Option<String>,
    /// Functions to exclude from PHP binding generation.
    #[serde(default)]
    pub exclude_functions: Vec<String>,
    /// Types to exclude from PHP binding generation.
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
    /// `scaffold_php_cargo` otherwise puts every name `collect_cfg_features` finds
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
    /// Acknowledge that generated service handler bridges retain request-scoped Zend
    /// callables behind `Send + Sync` Rust trait objects.
    ///
    /// alef refuses to generate PHP service handler registrations by default: the bridge
    /// keeps a request-owned `ZendCallable` reachable from a `Send + Sync` trait object, so a
    /// callback (or the final reference release) can run on a non-PHP thread or after the
    /// originating request ended. Set this to `true` only when the consumer guarantees that
    /// every registered callback is dispatched and dropped on the originating PHP thread for
    /// as long as the request is alive. ~keep
    #[serde(default)]
    pub allow_service_handler_callbacks: bool,
}
