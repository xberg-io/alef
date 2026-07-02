use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

use super::FfiTargetDepOverride;

/// Configuration for a single capsule type entry in `NodeConfig::capsule_types`.
///
/// When set, the named Rust type is NOT emitted as a `#[napi]` opaque wrapper.
/// Instead, functions returning this type produce a `JsObject` carrying the raw
/// pointer in a configurable `Napi::External<T>` property — the layout consumed
/// by the `sample_language` npm package's `Parser.setLanguage()`.
///
/// TOML form:
/// ```toml
/// [crates.node.capsule_types.Language]
/// type = "Language"
/// from_module = "sample_language"
/// property_name = "language"
/// type_tag = { lower = "0x8AF2E5212AD58ABF", upper = "0xD5006CAD83ABBA16" }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct NodeCapsuleTypeConfig {
    /// User-facing class name in the ecosystem library (e.g. `"Language"`).
    /// Emitted as the return-type annotation in the generated `index.d.ts`.
    #[serde(rename = "type")]
    pub type_name: String,
    /// npm package to import the type from (e.g. `"sample_language"`).
    /// Emitted as the `from` clause in the generated `import type` line.
    pub from_module: String,
    /// Codegen strategy. Currently only `"external_pointer"` is supported.
    /// Defaults to `"external_pointer"`.
    #[serde(default = "default_node_capsule_construct")]
    pub construct: String,
    /// JS property name to set on the returned object. `node-sample_language`
    /// reads `value["language"]`; other consumers may use different names.
    /// Defaults to `"__parser"` for back-compat with existing configs.
    #[serde(default = "default_node_capsule_property_name")]
    pub property_name: String,
    /// Optional N-API type tag to apply via `napi_type_tag_object`. Required
    /// when the consumer library (e.g. `node-sample_language`) calls
    /// `napi_check_object_type_tag` to validate the External before using it.
    #[serde(default)]
    pub type_tag: Option<NapiTypeTagConfig>,
}

/// An N-API `napi_type_tag` value, expressed as two 64-bit hex strings.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct NapiTypeTagConfig {
    /// Lower 64 bits of the tag, hex (e.g. `"0x8AF2E5212AD58ABF"`).
    pub lower: String,
    /// Upper 64 bits of the tag, hex (e.g. `"0xD5006CAD83ABBA16"`).
    pub upper: String,
}

fn default_node_capsule_construct() -> String {
    "external_pointer".to_string()
}

fn default_node_capsule_property_name() -> String {
    "__parser".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct NodeConfig {
    pub package_name: Option<String>,
    /// Per-language feature override. When set, these features are used instead of
    /// `[crate] features` for this language's binding crate.
    #[serde(default)]
    pub features: Option<Vec<String>>,
    /// Override the serde rename_all strategy for JSON field names (e.g. "camelCase", "snake_case").
    /// When set, this takes priority over the IR type-level serde_rename_all.
    #[serde(default)]
    pub serde_rename_all: Option<String>,
    /// Prefix for generated type names (e.g. "Js" produces `JsConversionOptions`).
    /// Defaults to `"Js"`.
    #[serde(default)]
    pub type_prefix: Option<String>,
    /// Map of Rust type name -> capsule config for raw-pointer passthrough.
    /// Types listed here skip the default `#[napi]` opaque-wrapper emission;
    /// functions returning them produce a `JsObject` with a `__parser`
    /// `Napi::External<T>` property instead. See [`NodeCapsuleTypeConfig`].
    #[serde(default)]
    pub capsule_types: HashMap<String, NodeCapsuleTypeConfig>,
    /// Functions to exclude from Node binding generation.
    #[serde(default)]
    pub exclude_functions: Vec<String>,
    /// Types to exclude from Node binding generation.
    #[serde(default)]
    pub exclude_types: Vec<String>,
    /// napi-rs platforms to drop from generated `package.json.napi.targets`,
    /// `optionalDependencies`, runtime dispatch table, and per-platform stub
    /// directories. Values are napi-rs platform strings (e.g.
    /// `"linux-x64-musl"`, `"linux-arm64-musl"`). Useful when downstream
    /// publish pipelines do not produce binaries for a platform yet the alef
    /// defaults would otherwise emit dangling stub packages.
    #[serde(default)]
    pub exclude_platforms: Vec<String>,
    /// Additional Cargo dependencies for this language's binding crate only.
    #[serde(default)]
    #[schemars(with = "HashMap<String, serde_json::Value>")]
    pub extra_dependencies: HashMap<String, toml::Value>,
    /// Features for the auto-emitted `tokio-util` dependency that backs napi trait-bridge
    /// cancellation tokens. Defaults to `["rt"]` because `tokio_util::sync::CancellationToken`
    /// is gated behind the `rt` feature in tokio-util 0.7+. Override when a consumer needs
    /// additional tokio-util features (e.g. `["rt", "codec"]`).
    #[serde(default)]
    pub tokio_util_features: Option<Vec<String>>,
    /// Override the scaffold output directory for this language's Cargo.toml and package files.
    #[serde(default)]
    pub scaffold_output: Option<PathBuf>,
    /// Overrides the default `crates/{name}-node` formula for the crate directory.
    /// Useful when the Rust crate directory does not follow the alef convention
    /// (e.g., consumer strips a `-rs` suffix so actual crate dir is `crates/sample-markdown-node`
    /// instead of `crates/sample-markdown-rs-node`). Used by setup, test, clean, and format tasks.
    #[serde(default)]
    pub crate_dir: Option<String>,
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
}
