use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GoConfig {
    pub module: Option<String>,
    /// Override the Go package name (default: derived from module path)
    pub package_name: Option<String>,
    /// Map of Rust type name -> host-native capsule (Language-passthrough) config.
    /// When set, functions returning the type construct the host runtime's native
    /// `Language` (e.g. `*tree_sitter.Language`) from the raw C grammar pointer instead
    /// of an opaque handle. See [`crate::core::config::HostCapsuleTypeConfig`].
    #[serde(default)]
    pub capsule_types: std::collections::HashMap<String, crate::core::config::HostCapsuleTypeConfig>,
    /// Affirms every configured capsule wrapper uses the exact native runtime instance ~keep
    /// and ownership contract that produced its pointer. ~keep
    #[serde(default)]
    pub shares_native_runtime: bool,
    /// Go module major version segment (`/vN`). Required for any v2+ Go module.
    /// Defaults to no segment when `None` and the Go module path has no version suffix;
    /// when set, emits `packages/go/v<N>/`.
    #[serde(default)]
    pub module_major: Option<u32>,
    #[serde(default)]
    pub features: Option<Vec<String>>,
    /// Types to exclude from Go binding generation.
    ///
    /// Go bindings call the generated C FFI directly through cgo, so types excluded from
    /// `[crates.ffi].exclude_types` are also excluded automatically by the Go backend.
    #[serde(default)]
    pub exclude_types: Vec<String>,
    /// Functions to exclude from Go binding generation, unioned with
    /// `[crates.ffi].exclude_functions`.
    ///
    /// Use this for functions that must stay in the C ABI (and therefore in
    /// `[crates.ffi].exclude_functions`) for other bindings but that this Go binding
    /// wants to hide from its own public surface — e.g. an async variant with no
    /// synchronous counterpart worth exposing in Go's idiom. Mirrors
    /// `CSharpConfig::exclude_functions` / `KotlinAndroidConfig::exclude_functions`.
    #[serde(default)]
    pub exclude_functions: Vec<String>,
    /// Extra cgo `LDFLAGS` appended after the FFI library's own `-l` flag, keyed by a cgo build
    /// constraint: a GOOS (`darwin`, `linux`, `windows`, ...) or `GOOS,GOARCH`
    /// (`windows,arm64`). Each value is the list of linker arguments for that constraint and
    /// renders as one `#cgo <constraint> LDFLAGS: ...` line in the generated cgo preamble.
    ///
    /// Needed when the Go package links the static FFI library, which leaves the native
    /// libraries the Rust standard library and its dependencies need (`-framework Security`,
    /// `-lresolv`, `-lws2_32`, ...) for the final link to supply.
    #[serde(default)]
    pub link_flags: BTreeMap<String, Vec<String>>,
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
    #[serde(default)]
    pub extra_lint_paths: Vec<String>,
    /// Struct types that should emit the functional-options pattern (`With<Field>` helpers
    /// plus `New<Struct>(opts ...<Struct>Option)` constructor).
    /// By default, pure data DTOs emit plain struct literals; this allowlist enables the
    /// functional-options pattern only for behaviour-knob types (e.g., `DialOptions`).
    /// Defaults to empty (no functional-options emitted).
    #[serde(default)]
    pub functional_options: Vec<String>,
}
