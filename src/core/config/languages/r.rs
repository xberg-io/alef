use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RConfig {
    pub package_name: Option<String>,
    #[serde(default)]
    pub features: Option<Vec<String>>,
    /// Whether the generated R Rust crate should enable the core crate's default features.
    /// Defaults to true. Set to false with an explicit `features` list for curated targets
    /// such as `wasm-target`.
    #[serde(default)]
    pub default_features: Option<bool>,
    /// Override the serde rename_all strategy for JSON field names (e.g. "camelCase", "snake_case").
    /// When set, this takes priority over the IR type-level serde_rename_all.
    #[serde(default)]
    pub serde_rename_all: Option<String>,
    /// Functions to exclude from R binding generation.
    #[serde(default)]
    pub exclude_functions: Vec<String>,
    /// Types to exclude from R binding generation.
    #[serde(default)]
    pub exclude_types: Vec<String>,
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
    /// Extra Makefile fragments injected before `PKG_LIBS` in `Makevars`/`Makevars.in`.
    /// Each entry becomes its own line (e.g. `HEIF_LIBS = $(shell pkg-config --libs libheif)`).
    /// Useful for system libraries dynamically linked by Rust deps that R's `R CMD INSTALL`
    /// must also link against.
    #[serde(default)]
    pub extra_makevars_prelude: Vec<String>,
    /// Extra tokens appended to the `PKG_LIBS` variable in `Makevars`/`Makevars.in`,
    /// after the staticlib link. Combined with `extra_makevars_prelude` to inject
    /// downstream-managed link flags (e.g. `$(HEIF_LIBS)`).
    #[serde(default)]
    pub extra_pkg_libs: Vec<String>,
}
