use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CSharpConfig {
    /// Map of Rust type name -> host-native capsule (Language-passthrough) config.
    /// When set, functions returning the type construct the host runtime's native
    /// `Language` (e.g. `TreeSitter.Language`) from the raw C grammar pointer (an
    /// `IntPtr`) instead of an opaque handle. See [`crate::core::config::HostCapsuleTypeConfig`].
    #[serde(default)]
    pub capsule_types: HashMap<String, crate::core::config::HostCapsuleTypeConfig>,
    /// Affirms every configured capsule wrapper uses the exact native runtime instance ~keep
    /// and ownership contract that produced its pointer. ~keep
    #[serde(default)]
    pub shares_native_runtime: bool,
    pub namespace: Option<String>,
    /// NuGet `<PackageId>` to publish under. When unset, falls back to `namespace`.
    /// Use this when the published artifact id must differ from the C# `RootNamespace` —
    /// e.g. when the unprefixed name is owned by a third party on nuget.org and
    /// you publish under a vendor-prefixed id like `SampleCrateDev.<Lib>`.
    #[serde(default)]
    pub package_id: Option<String>,
    pub target_framework: Option<String>,
    #[serde(default)]
    pub features: Option<Vec<String>>,
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
    /// Project file for C# (e.g., "MyProject.csproj", "MySolution.sln"). When set, default
    /// lint/build/test commands target this file instead of the output directory.
    #[serde(default)]
    pub project_file: Option<String>,
    /// Split-byte DTOs whose Bytes and Uri variants describe mutually exclusive sources.
    /// Managed extraction honors the source discriminator when both payload fields are populated.
    #[serde(default)]
    pub split_bytes_disjoint_sources: Vec<String>,
    /// Types to exclude from C# binding generation.
    ///
    /// C# bindings call the generated C FFI through P/Invoke, so types excluded from
    /// `[crates.ffi].exclude_types` are also excluded automatically by the C# backend.
    #[serde(default)]
    pub exclude_types: Vec<String>,
    /// Functions to exclude from C# binding generation (e.g., functions not present in the
    /// C FFI layer). Excluded functions are omitted from both NativeMethods.cs and the
    /// wrapper class.
    #[serde(default)]
    pub exclude_functions: Vec<String>,
}
