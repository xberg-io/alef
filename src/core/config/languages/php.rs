use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

use super::StubsConfig;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
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
}
