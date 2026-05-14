//! Workspace-level shared defaults for multi-crate alef workspaces.
//!
//! A `[workspace]` section in `alef.toml` collects defaults that apply to every
//! `[[crates]]` entry unless that crate overrides the field. The fields here
//! are the cross-crate concerns (tooling, DTO style, default pipelines, output
//! templates) — anything that is fundamentally per-crate (sources, language
//! module names, publish settings) lives on [`crate::config::raw_crate::RawCrateConfig`]
//! instead.
//!
//! See `crates/alef-core/src/config/resolved.rs` for how workspace defaults
//! merge into a per-crate [`crate::config::resolved::ResolvedCrateConfig`].

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::dto::DtoConfig;
use super::extras::Language;
use super::output::{
    BuildCommandConfig, CleanConfig, GeneratedHeaderConfig, LintConfig, OutputTemplate, PrecommitConfig,
    ScaffoldConfig, SetupConfig, SyncConfig, TestConfig, UpdateConfig,
};
use super::tools::ToolsConfig;
use super::{FormatConfig, GenerateConfig};

/// Workspace-level configuration shared across all `[[crates]]` entries.
///
/// Every field is optional; an empty `[workspace]` section is valid and means
/// every crate uses Alef's built-in defaults (or its own per-crate values).
///
/// Resolution rule (highest priority first):
/// 1. Per-crate value on `[[crates]]`.
/// 2. Workspace default on `[workspace]`.
/// 3. Built-in default (compiled into Alef).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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

    /// Default generated-file header metadata.
    /// Per-crate `[scaffold.generated_header]` values override this field-by-field.
    #[serde(default)]
    pub generated_header: Option<GeneratedHeaderConfig>,

    /// Default pre-commit scaffold metadata.
    /// Per-crate `[scaffold.precommit]` values override this field-by-field.
    #[serde(default)]
    pub precommit: Option<PrecommitConfig>,

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

    /// Workspace-wide version sync rules. A per-crate publish step still runs
    /// independently per crate; sync rules in this section apply globally.
    #[serde(default)]
    pub sync: Option<SyncConfig>,
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
command = "uv run pytest"

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
}
