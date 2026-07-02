//! Workspace-level shared defaults for multi-crate alef workspaces.
//!
//! A `[workspace]` section in `alef.toml` collects defaults that apply to every
//! `[[crates]]` entry unless that crate overrides the field. The fields here
//! are the cross-crate concerns (tooling, DTO style, default pipelines, output
//! templates) — anything that is fundamentally per-crate (sources, language
//! module names, publish settings) lives on [`crate::core::config::raw_crate::RawCrateConfig`]
//! instead.
//!
//! See `crates/alef-core/src/config/resolved.rs` for how workspace defaults
//! merge into a per-crate [`crate::core::config::resolved::ResolvedCrateConfig`].

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::dto::DtoConfig;
use super::extras::Language;
use super::languages::{
    CSharpConfig, DartConfig, ElixirConfig, FfiConfig, GleamConfig, GoConfig, JavaConfig, JniConfig,
    KotlinAndroidConfig, KotlinConfig, NodeConfig, PhpConfig, PythonConfig, RConfig, RubyConfig, SwiftConfig,
    WasmConfig, ZigConfig,
};
use super::output::{
    BuildCommandConfig, CitationConfig, CleanConfig, DocsConfig, GeneratedHeaderConfig, LintConfig, OutputTemplate,
    ScaffoldConfig, SetupConfig, SyncConfig, TestConfig, UpdateConfig,
};
use super::package_metadata::PackageMetadataConfig;
use super::poly::PolyConfig;
use super::tools::ToolsConfig;
use super::{FormatConfig, GenerateConfig};

/// One parameter in a [`ClientConstructorConfig`].
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ConstructorParam {
    /// Parameter name as it appears in the generated function signature.
    pub name: String,
    /// Rust type of the parameter (e.g. `"*const c_char"` for FFI, `"&str"` for Rust-embedded).
    #[serde(rename = "type")]
    pub ty: String,
}

/// Custom constructor configuration for an opaque handle type.
///
/// When present under `[workspace.client_constructors.<TypeName>]`, every
/// backend that wraps the type in an opaque handle emits a constructor whose
/// body is the `body` template string with `{type_name}` and `{source_path}`
/// substituted.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ClientConstructorConfig {
    /// Ordered list of constructor parameters.
    #[serde(default)]
    pub params: Vec<ConstructorParam>,
    /// Body template.  Use `{type_name}` for the bare type name and
    /// `{source_path}` for the fully-qualified core path.
    pub body: String,
    /// Error type returned by the constructor (`Result<Self, ErrType>`).
    /// Defaults to `String` when absent.
    #[serde(default)]
    pub error_type: Option<String>,
}

/// Workspace-level configuration shared across all `[[crates]]` entries.
///
/// Every field is optional; an empty `[workspace]` section is valid and means
/// every crate uses Alef's built-in defaults (or its own per-crate values).
///
/// Resolution rule (highest priority first):
/// 1. Per-crate value on `[[crates]]`.
/// 2. Workspace default on `[workspace]`.
/// 3. Built-in default (compiled into Alef).
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceConfig {
    /// Pinned alef CLI version (e.g. `"0.13.0"`). Used by the `install-alef`
    /// helper to install the exact version this workspace expects.
    ///
    /// In the legacy single-crate schema this lived at `version` at the top
    /// level. The new schema renames it to `[workspace] alef_version` so it
    /// can never collide with any per-crate version field.
    #[serde(default)]
    pub alef_version: Option<String>,

    /// Default list of target languages for crates that do not specify their
    /// own. A per-crate `languages` array overrides this entirely.
    #[serde(default)]
    pub languages: Vec<Language>,

    /// Default Python backend settings.
    #[serde(default)]
    pub python: Option<PythonConfig>,
    /// Default Node/N-API backend settings.
    #[serde(default)]
    pub node: Option<NodeConfig>,
    /// Default Ruby/Magnus backend settings.
    #[serde(default)]
    pub ruby: Option<RubyConfig>,
    /// Default PHP backend settings.
    #[serde(default)]
    pub php: Option<PhpConfig>,
    /// Default Elixir/Rustler backend settings.
    #[serde(default)]
    pub elixir: Option<ElixirConfig>,
    /// Default WASM backend settings.
    #[serde(default)]
    pub wasm: Option<WasmConfig>,
    /// Default C FFI backend settings.
    #[serde(default)]
    pub ffi: Option<FfiConfig>,
    /// Default Go backend settings.
    #[serde(default)]
    pub go: Option<GoConfig>,
    /// Default Java backend settings.
    #[serde(default)]
    pub java: Option<JavaConfig>,
    /// Default Dart backend settings.
    #[serde(default)]
    pub dart: Option<DartConfig>,
    /// Default Kotlin backend settings.
    #[serde(default)]
    pub kotlin: Option<KotlinConfig>,
    /// Default Kotlin Android backend settings.
    #[serde(default)]
    pub kotlin_android: Option<KotlinAndroidConfig>,
    /// Default JNI backend settings.
    #[serde(default)]
    pub jni: Option<JniConfig>,
    /// Default Swift backend settings.
    #[serde(default)]
    pub swift: Option<SwiftConfig>,
    /// Default Gleam backend settings.
    #[serde(default)]
    pub gleam: Option<GleamConfig>,
    /// Default C# backend settings.
    #[serde(default)]
    pub csharp: Option<CSharpConfig>,
    /// Default R/extendr backend settings.
    #[serde(default)]
    pub r: Option<RConfig>,
    /// Default Zig backend settings.
    #[serde(default)]
    pub zig: Option<ZigConfig>,

    /// Global package-manager and dev-tool preferences. Inherited by every
    /// crate; cannot be overridden per-crate today.
    #[serde(default)]
    pub tools: ToolsConfig,

    /// Default DTO/type generation styles per language. A per-crate `[crates.dto]`
    /// table replaces this wholesale (no field-level merge).
    #[serde(default)]
    pub dto: DtoConfig,

    /// Default post-generation formatting flags. A per-crate value replaces
    /// this wholesale.
    #[serde(default)]
    pub format: FormatConfig,

    /// Default per-language formatting overrides (e.g., disable `mix format`
    /// for elixir). Merged with per-crate `format_overrides` by language key:
    /// per-crate keys win wholesale; missing keys fall through to this map.
    /// Note: there is no field-level merge inside a single `FormatConfig`.
    #[serde(default)]
    pub format_overrides: HashMap<String, FormatConfig>,

    /// Default generation-pass flags (which passes alef runs).
    #[serde(default)]
    pub generate: GenerateConfig,

    /// Default per-language generation flag overrides. Merged with per-crate
    /// `generate_overrides` by language key: per-crate keys win wholesale;
    /// missing keys fall through to this map.
    #[serde(default)]
    pub generate_overrides: HashMap<String, GenerateConfig>,

    /// Per-language output path templates with `{crate}` and `{lang}` placeholders.
    /// A per-crate explicit `[crates.output]` path always wins over the template.
    #[serde(default)]
    pub output_template: OutputTemplate,

    /// Default package metadata for generated manifests and README context.
    /// Per-crate `[scaffold]` values override this field-by-field.
    #[serde(default)]
    pub scaffold: Option<ScaffoldConfig>,

    /// Centralized package metadata for generated language manifests.
    /// Per-crate `[crates.package_metadata]` values override this field-by-field.
    #[serde(default)]
    pub package_metadata: Option<PackageMetadataConfig>,

    /// Default generated-file header metadata.
    /// Per-crate `[scaffold.generated_header]` values override this field-by-field.
    #[serde(default)]
    pub generated_header: Option<GeneratedHeaderConfig>,

    /// Default lint pipeline keyed by language code (`"python"`, `"node"`, …).
    /// Merged field-wise with per-crate `[crates.lint.<lang>]`.
    #[serde(default)]
    pub lint: HashMap<String, LintConfig>,

    /// Default test pipeline keyed by language code.
    #[serde(default)]
    pub test: HashMap<String, TestConfig>,

    /// Default setup pipeline keyed by language code.
    #[serde(default)]
    pub setup: HashMap<String, SetupConfig>,

    /// Default update pipeline keyed by language code.
    #[serde(default)]
    pub update: HashMap<String, UpdateConfig>,

    /// Default clean pipeline keyed by language code.
    #[serde(default)]
    pub clean: HashMap<String, CleanConfig>,

    /// Default build pipeline keyed by language code.
    #[serde(default)]
    pub build_commands: HashMap<String, BuildCommandConfig>,

    /// Workspace-wide opaque types — types from external crates that alef can't
    /// extract. Map of type name → fully-qualified Rust path. These get opaque
    /// wrapper structs across all language backends, in every crate that
    /// references them.
    #[serde(default)]
    pub opaque_types: HashMap<String, String>,

    /// Per-type custom constructors emitted by every backend that supports
    /// opaque handles.  Key: type name (e.g. `"DefaultClient"`).
    /// Value: [`ClientConstructorConfig`] describing params and a body template.
    #[serde(default)]
    pub client_constructors: HashMap<String, ClientConstructorConfig>,

    /// Workspace-wide version sync rules. A per-crate publish step still runs
    /// independently per crate; sync rules in this section apply globally.
    #[serde(default)]
    pub sync: Option<SyncConfig>,

    /// Optional CITATION.cff metadata. When present, `alef sync-versions` writes
    /// a fully rendered `CITATION.cff` at the repo root using these fields plus
    /// the canonical workspace version. When absent, a hand-authored
    /// CITATION.cff (if any) only has its `version:` line updated.
    #[serde(default)]
    pub citation: Option<CitationConfig>,

    /// Default template-driven docs generation config. Per-crate `[crates.docs]`
    /// values override this field-by-field.
    #[serde(default)]
    pub docs: Option<DocsConfig>,

    /// Repository-level `poly.toml` customisations merged into the emitter's
    /// generated output.  Because `poly.toml` is regenerated on every `alef`
    /// run, any per-repo lint suppressions must live here to survive regen.
    ///
    /// See [`PolyConfig`] for the full set of configurable knobs.
    #[serde(default)]
    pub poly: PolyConfig,

    /// Extra clippy lints to allow in every generated Rust binding file, merged
    /// (union, de-duplicated) with each backend's built-in default allow-list.
    ///
    /// Entries may be bare lint names (`"single_match"`) or `clippy::`-prefixed
    /// (`"clippy::single_match"`); both forms are accepted and normalised
    /// internally.  When absent or empty the emitted allow-list is byte-identical
    /// to the backend default — no diff in consumers that do not set this field.
    ///
    /// Example:
    /// ```toml
    /// [workspace]
    /// extra_clippy_allows = ["single_match", "collapsible_match"]
    /// ```
    #[serde(default)]
    pub extra_clippy_allows: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_config_deserializes_empty() {
        let cfg: WorkspaceConfig = toml::from_str("").unwrap();
        assert!(cfg.alef_version.is_none());
        assert!(cfg.languages.is_empty());
        assert!(cfg.lint.is_empty());
        assert!(cfg.opaque_types.is_empty());
        assert!(cfg.sync.is_none());
    }

    #[test]
    fn workspace_config_deserializes_full() {
        let toml_str = r#"
alef_version = "0.13.0"
languages = ["python", "node"]

[output_template]
python = "packages/python/{crate}/"
node   = "packages/node/{crate}/"

[lint.python]
precondition = "command -v ruff >/dev/null 2>&1"
check        = "ruff check ."

[test.python]
command = "uv run --no-sync pytest"

[opaque_types]
Tree = "tree_sitter::Tree"
"#;
        let cfg: WorkspaceConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.alef_version.as_deref(), Some("0.13.0"));
        assert_eq!(cfg.languages.len(), 2);
        assert_eq!(cfg.output_template.python.as_deref(), Some("packages/python/{crate}/"));
        assert!(cfg.lint.contains_key("python"));
        assert!(cfg.test.contains_key("python"));
        assert_eq!(
            cfg.opaque_types.get("Tree").map(String::as_str),
            Some("tree_sitter::Tree")
        );
    }

    #[test]
    fn workspace_config_deserializes_client_constructors() {
        let toml_str = r#"
[client_constructors.DefaultClient]
body = "{source_path}::new().map_err(|e| e.to_string())"

[[client_constructors.DefaultClient.params]]
name = "api_key"
type = "*const std::ffi::c_char"
"#;
        let cfg: WorkspaceConfig = toml::from_str(toml_str).unwrap();
        let ctor = cfg.client_constructors.get("DefaultClient").unwrap();
        assert_eq!(ctor.params.len(), 1);
        assert_eq!(ctor.params[0].name, "api_key");
        assert_eq!(ctor.params[0].ty, "*const std::ffi::c_char");
        assert!(ctor.body.contains("{source_path}"));
    }
}
