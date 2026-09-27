use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

use crate::core::config::dto::JavaDtoConfig;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct JavaConfig {
    /// Per-dependency version overrides for the manifests alef regenerates for this
    /// language. Keys are ecosystem-qualified (`cargo:pyo3`, `pypi:pyrefly`, `maven:junit`,
    /// `hex:credo`, `gem:rspec`, `cran:rextendr`) and are listed, with the literal each one
    /// defaults to, by `alef::core::managed_versions`. An unknown key is rejected outright
    /// rather than ignored: silently dropping an override is the exact failure this table
    /// exists to prevent.
    #[serde(default)]
    pub dependency_versions: BTreeMap<String, String>,
    /// Map of Rust type name -> host-native capsule (Language-passthrough) config.
    /// When set, functions returning the type construct the host runtime's native
    /// `Language` (e.g. jtreesitter's `io.github.treesitter.jtreesitter.Language` from a
    /// `MemorySegment`) instead of an opaque handle. See [`crate::core::config::HostCapsuleTypeConfig`].
    #[serde(default)]
    pub capsule_types: HashMap<String, crate::core::config::HostCapsuleTypeConfig>,
    /// Affirms every configured capsule wrapper uses the exact native runtime instance ~keep
    /// and ownership contract that produced its pointer. ~keep
    #[serde(default)]
    pub shares_native_runtime: bool,
    pub package: Option<String>,
    /// Override the Maven `<groupId>` emitted by alef-scaffold and alef-e2e. When unset,
    /// `java_group_id()` falls back to the Java `package` value. Set this when the
    /// published Maven coords differ from the Java package path (e.g. group
    /// `dev.sample_core`, package `dev.sample_core.samplemarkdown`).
    #[serde(default)]
    pub group_id: Option<String>,
    /// Override the Maven `<artifactId>` emitted by alef-scaffold and alef-e2e. When
    /// unset, defaults to the crate name (the `[[crates]] name = "..."`). Set this when
    /// the published artifactId differs from the source crate name (e.g. crate
    /// `sample-markdown-rs` published as `sample-markdown`).
    #[serde(default)]
    pub artifact_id: Option<String>,
    #[serde(default = "default_java_ffi_style")]
    pub ffi_style: String,
    #[serde(default)]
    pub features: Option<Vec<String>>,
    /// Types to exclude from Java binding generation.
    ///
    /// Java's Panama bindings call the generated C FFI directly, so types excluded from
    /// `[crates.ffi].exclude_types` are also excluded automatically by the Java backend.
    #[serde(default)]
    pub exclude_types: Vec<String>,
    /// Functions to exclude from Java binding generation, unioned with
    /// `[crates.ffi].exclude_functions`.
    ///
    /// Use this for functions that must stay in the C ABI (and therefore out of
    /// `[crates.ffi].exclude_functions`) for other bindings, but that this Java binding
    /// wants to hide from its own public surface. Mirrors `GoConfig::exclude_functions`
    /// and `CSharpConfig::exclude_functions`.
    #[serde(default)]
    pub exclude_functions: Vec<String>,
    /// Override the serde rename_all strategy for JSON field names (e.g. "camelCase", "snake_case").
    /// When set, this takes priority over the IR type-level serde_rename_all.
    #[serde(default)]
    pub serde_rename_all: Option<String>,
    /// Per-field name remapping for this language. Key is `TypeName.field_name`, value is the
    /// desired binding field name. Applied after automatic keyword escaping.
    #[serde(default)]
    pub rename_fields: HashMap<String, String>,
    /// Prefix wrapper for default tool invocations. When set, prepends this string to default
    /// commands across all pipelines (lint, test, build, etc.).
    #[serde(default)]
    pub run_wrapper: Option<String>,
    /// Extra paths to append to default lint commands (format, check, typecheck).
    /// Ignored when project_file is set.
    #[serde(default)]
    pub extra_lint_paths: Vec<String>,
    /// Project file for Maven/Gradle (e.g., "pom.xml", "build.gradle"). When set, default
    /// lint/build/test commands target this file instead of the output directory.
    #[serde(default)]
    pub project_file: Option<String>,
    /// DTO-specific configuration (e.g., builder mode).
    #[serde(default)]
    pub dto: JavaDtoConfig,
}

fn default_java_ffi_style() -> String {
    "panama".to_string()
}
