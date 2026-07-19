//! Repository-level `poly.toml` customisations — per-repo additions that
//! survive regeneration.
//!
//! Alef regenerates `poly.toml` on every run, so any repo-specific lint
//! suppressions (e.g. `[discovery] exclude` extras, per-file-ignores, typos
//! allowlists) must live in `alef.toml` under `[workspace.poly]` rather than
//! in the generated file itself.  The poly emitter reads this struct and merges
//! the extra entries into its output.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Typos spell-checker customisations emitted under `[lint.typos.*]`.
///
/// Declared under `[workspace.poly.typos]` in `alef.toml`. When all
/// sub-tables are absent or empty, no `[lint.typos.*]` tables are emitted.
///
/// ```toml
/// [workspace.poly.typos.extend-words]
/// # "typo" = "correct" — set both equal to suppress without correcting.
/// flate = "flate"
/// delocate = "delocate"
///
/// [workspace.poly.typos.extend-identifiers]
/// PyMuPDF = "PyMuPDF"
/// PDFium  = "PDFium"
/// ```
#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct TyposConfig {
    /// Extra word corrections for the typos spell-checker.
    ///
    /// Key = the "incorrect" spelling typos would flag; value = the intended
    /// spelling. Set both equal to suppress a false-positive without
    /// auto-correcting it (e.g. `flate = "flate"` keeps `flate` unchanged).
    ///
    /// Uses [`BTreeMap`] so entries are written to `poly.toml` in
    /// deterministic (alphabetical key) order.
    ///
    /// Emitted as `[lint.typos.extend_words]` in the generated poly.toml.
    #[serde(default)]
    pub extend_words: BTreeMap<String, String>,

    /// Extra identifier corrections for the typos spell-checker.
    ///
    /// Same semantics as `extend_words` but applied to CamelCase / PascalCase
    /// identifiers in source code.
    ///
    /// Uses [`BTreeMap`] so entries are written in deterministic order.
    ///
    /// Emitted as `[lint.typos.extend_identifiers]` in the generated poly.toml.
    #[serde(default)]
    pub extend_identifiers: BTreeMap<String, String>,
}

/// Uncomment linter customisations emitted under `[lint.uncomment]`.
///
/// Declared under `[workspace.poly.uncomment]` in `alef.toml`. When the section
/// is absent, no `[lint.uncomment]` table is emitted (the generated output is
/// byte-identical to the default). When present, a `[lint.uncomment]` table is
/// emitted with each field rendered under its snake_case poly key.
///
/// Field defaults mirror poly's own `uncomment` defaults so declaring an empty
/// `[workspace.poly.uncomment]` table opts the linter in with poly-default
/// behaviour.
///
/// ```toml
/// [workspace.poly.uncomment]
/// enabled = true
/// remove-todos = false
/// preserve-patterns = ["allow(", "SAFETY:"]
/// ```
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct UncommentConfig {
    /// Whether the uncomment linter runs at all.
    ///
    /// Emitted as `enabled` in the generated `[lint.uncomment]` table.
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Whether `TODO` comments are stripped.
    ///
    /// Emitted as `remove_todos` in the generated `[lint.uncomment]` table.
    #[serde(default)]
    pub remove_todos: bool,

    /// Whether `FIXME` comments are stripped.
    ///
    /// Emitted as `remove_fixme` in the generated `[lint.uncomment]` table.
    #[serde(default)]
    pub remove_fixme: bool,

    /// Whether documentation comments are stripped.
    ///
    /// Emitted as `remove_docs` in the generated `[lint.uncomment]` table.
    #[serde(default)]
    pub remove_docs: bool,

    /// Whether poly's built-in ignore patterns apply.
    ///
    /// Emitted as `use_default_ignores` in the generated `[lint.uncomment]`
    /// table.
    #[serde(default = "default_true")]
    pub use_default_ignores: bool,

    /// Extra comment-content substrings that exempt a comment from removal.
    ///
    /// Emitted as `preserve_patterns` in the generated `[lint.uncomment]`
    /// table. Order is preserved as written. An empty list still emits an empty
    /// array so the key round-trips.
    #[serde(default)]
    pub preserve_patterns: Vec<String>,
}

/// Serde default helper: poly defaults `enabled` and `use_default_ignores` to
/// `true`.
fn default_true() -> bool {
    true
}

impl Default for UncommentConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            remove_todos: false,
            remove_fixme: false,
            remove_docs: false,
            use_default_ignores: true,
            preserve_patterns: Vec::new(),
        }
    }
}

/// Repo-specific `poly.toml` overrides merged into the emitter's generated
/// output.
///
/// Declared under `[workspace.poly]` in `alef.toml`.  An empty (or absent)
/// `[workspace.poly]` section leaves the generated output byte-identical to the
/// default.
///
/// ```toml
/// [workspace.poly]
/// exclude = ["vendor/generated/**", "third-party/**"]
/// file-safety-exclude = ["crates/*/src/lib.rs"]
///
/// [workspace.poly.typos.extend-words]
/// flate = "flate"
///
/// [workspace.poly.typos.extend-identifiers]
/// PyMuPDF = "PyMuPDF"
///
/// [workspace.poly.per-file-ignores]
/// "**/legacy_api.py" = ["ANN", "D103"]
/// "**/compat.py"     = ["UP035", "F401"]
///
/// [workspace.poly.uncomment]
/// enabled = true
/// preserve-patterns = ["SAFETY:"]
/// ```
#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct PolyConfig {
    /// Extra gitignore-style globs appended to the emitter's default
    /// `[discovery] exclude` list.  The same merged list is also mirrored into
    /// the `polylint`, `polyfmt`, and `file_safety` builtin hook excludes (the
    /// git-hook path that filters per-builtin rather than via discovery).
    ///
    /// Default globs are always emitted first; extra globs follow in the order
    /// given here.
    #[serde(default)]
    pub exclude: Vec<String>,

    /// Extra gitignore-style globs appended ONLY to the `file_safety` builtin
    /// hook exclude list — NOT to `[discovery]`, `polylint`, or `polyfmt`.
    ///
    /// Use this to exempt files from poly's shebang / file-safety check while
    /// still linting and formatting them. For example, Rust source with inner
    /// attributes (`#![...]`) on the first line is misread by the shebang
    /// heuristic as an executable script.
    ///
    /// Default `file_safety` globs (shared with `exclude`) are emitted first;
    /// these extra globs follow in the order given here.
    #[serde(default)]
    pub file_safety_exclude: Vec<String>,

    /// Typos spell-checker word and identifier allowlists.
    ///
    /// Merged into `[lint.typos.extend_words]` and
    /// `[lint.typos.extend_identifiers]` in the generated poly.toml.
    /// When all sub-tables are empty, no `[lint.typos.*]` sections are emitted.
    #[serde(default)]
    pub typos: TyposConfig,

    /// Repo-specific cross-engine per-file rule suppressions merged into the
    /// emitted `[per-file-ignores]` table.
    ///
    /// Keys are gitignore-style glob patterns; values are lists of rule codes.
    /// Rule codes are engine-agnostic — an unknown code simply no-ops on files
    /// of other languages.
    ///
    /// Uses [`BTreeMap`] so entries are written to `poly.toml` in
    /// deterministic (alphabetical key) order.
    #[serde(default)]
    pub per_file_ignores: BTreeMap<String, Vec<String>>,

    /// Extra pyrefly type-checker error suppressions, emitted as additional
    /// `[[tool.pyrefly.sub-config]]` blocks in the generated `pyproject.toml`
    /// (alongside the always-emitted `api.py` wrapper sub-config).
    ///
    /// Keys are glob patterns (matched against file paths by pyrefly); values
    /// are the pyrefly error codes to disable for the matched files (e.g.
    /// `bad-argument-type`, `missing-import`). Use this for extension-generated
    /// Python modules whose runtime-reconciled pyo3 boundaries a static checker
    /// cannot follow — the same rationale as the built-in `api.py` sub-config.
    ///
    /// Uses [`BTreeMap`] so entries are written in deterministic (alphabetical
    /// key) order. An empty map emits no extra sub-config blocks.
    #[serde(default)]
    pub pyrefly_sub_configs: BTreeMap<String, Vec<String>>,

    /// Optional uncomment-linter configuration emitted as a `[lint.uncomment]`
    /// table.
    ///
    /// Absent (the default) emits NO `[lint.uncomment]` table, leaving the
    /// output byte-identical to the pre-feature default. When present — even as
    /// an empty `[workspace.poly.uncomment]` table — a `[lint.uncomment]` table
    /// is emitted with poly-default values filled in.
    #[serde(default)]
    pub uncomment: Option<UncommentConfig>,

    /// External git-sourced pre-commit hook sources, emitted as `[[hooks.sources]]`
    /// blocks in the generated `poly.toml`. Each entry pins a hook repository (e.g.
    /// an `ai-rulez` validation hook) by git URL + revision.
    ///
    /// Empty (the default) emits no `[[hooks.sources]]` blocks, leaving the output
    /// byte-identical to the pre-feature default.
    #[serde(default)]
    pub hooks_sources: Vec<HookSource>,
}

/// A single external git hook source, rendered as a `[[hooks.sources]]` block in the
/// generated `poly.toml` (e.g. an `ai-rulez` validation hook pinned by revision).
#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct HookSource {
    /// Stable identifier for the hook source (e.g. `"ai-rulez"`).
    pub id: String,
    /// Git repository URL providing the hook.
    pub git: String,
    /// Pinned git revision (tag, branch, or commit SHA).
    pub revision: String,
    /// Hook names from the source to enable.
    #[serde(default)]
    pub hooks: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poly_config_deserializes_empty() {
        let cfg: PolyConfig = toml::from_str("").unwrap();
        assert!(cfg.exclude.is_empty());
        assert!(cfg.file_safety_exclude.is_empty());
        assert!(cfg.typos.extend_words.is_empty());
        assert!(cfg.typos.extend_identifiers.is_empty());
        assert!(cfg.per_file_ignores.is_empty());
        assert!(
            cfg.uncomment.is_none(),
            "absent [uncomment] must deserialize to None (no table emitted)"
        );
    }

    #[test]
    fn poly_config_deserializes_full() {
        let toml_str = r#"
exclude = ["vendor/**", "third-party/**"]
file-safety-exclude = ["crates/*/src/lib.rs", "**/inner_attrs.rs"]

[typos.extend-words]
flate = "flate"
arange = "arange"

[typos.extend-identifiers]
PyMuPDF = "PyMuPDF"

[per-file-ignores]
"**/legacy.py" = ["ANN", "D103"]
"**/compat.py" = ["UP035"]

[pyrefly-sub-configs]
"**/app.py" = ["bad-argument-type"]
"**/schema.py" = ["missing-import", "bad-return"]

[uncomment]
enabled = true
remove-todos = true
remove-fixme = true
remove-docs = false
use-default-ignores = false
preserve-patterns = ["SAFETY:", "allow("]
"#;
        let cfg: PolyConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.exclude, vec!["vendor/**", "third-party/**"]);
        assert_eq!(
            cfg.file_safety_exclude,
            vec!["crates/*/src/lib.rs", "**/inner_attrs.rs"]
        );
        let uncomment = cfg.uncomment.as_ref().expect("uncomment section present");
        assert!(uncomment.enabled);
        assert!(uncomment.remove_todos);
        assert!(uncomment.remove_fixme);
        assert!(!uncomment.remove_docs);
        assert!(!uncomment.use_default_ignores);
        assert_eq!(uncomment.preserve_patterns, vec!["SAFETY:", "allow("]);
        assert_eq!(cfg.typos.extend_words.len(), 2);
        assert_eq!(cfg.typos.extend_words["flate"], "flate");
        assert_eq!(cfg.typos.extend_words["arange"], "arange");
        assert_eq!(cfg.typos.extend_identifiers.len(), 1);
        assert_eq!(cfg.typos.extend_identifiers["PyMuPDF"], "PyMuPDF");
        assert_eq!(cfg.per_file_ignores.len(), 2);
        assert_eq!(cfg.per_file_ignores["**/legacy.py"], vec!["ANN", "D103"]);
        assert_eq!(cfg.per_file_ignores["**/compat.py"], vec!["UP035"]);
        assert_eq!(cfg.pyrefly_sub_configs.len(), 2);
        assert_eq!(cfg.pyrefly_sub_configs["**/app.py"], vec!["bad-argument-type"]);
        assert_eq!(
            cfg.pyrefly_sub_configs["**/schema.py"],
            vec!["missing-import", "bad-return"]
        );
    }

    #[test]
    fn poly_config_rejects_unknown_fields() {
        let err = toml::from_str::<PolyConfig>("unknown_field = true");
        assert!(err.is_err(), "deny_unknown_fields must reject unrecognised keys");
    }

    #[test]
    fn poly_config_uncomment_empty_table_uses_poly_defaults() {
        let cfg: PolyConfig = toml::from_str("[uncomment]\n").unwrap();
        let uncomment = cfg.uncomment.as_ref().expect("empty [uncomment] table -> Some");
        assert!(uncomment.enabled, "enabled defaults to true");
        assert!(!uncomment.remove_todos, "remove_todos defaults to false");
        assert!(!uncomment.remove_fixme, "remove_fixme defaults to false");
        assert!(!uncomment.remove_docs, "remove_docs defaults to false");
        assert!(uncomment.use_default_ignores, "use_default_ignores defaults to true");
        assert!(
            uncomment.preserve_patterns.is_empty(),
            "preserve_patterns defaults empty"
        );
    }

    #[test]
    fn poly_config_file_safety_exclude_defaults_empty() {
        let cfg: PolyConfig = toml::from_str("exclude = [\"a/**\"]").unwrap();
        assert!(
            cfg.file_safety_exclude.is_empty(),
            "file-safety-exclude defaults empty when unset"
        );
    }

    #[test]
    fn uncomment_config_rejects_unknown_fields() {
        let err = toml::from_str::<UncommentConfig>("unknown_field = true");
        assert!(err.is_err(), "deny_unknown_fields must reject unrecognised keys");
    }

    #[test]
    fn uncomment_config_default_matches_poly_defaults() {
        let cfg = UncommentConfig::default();
        assert!(cfg.enabled);
        assert!(!cfg.remove_todos);
        assert!(!cfg.remove_fixme);
        assert!(!cfg.remove_docs);
        assert!(cfg.use_default_ignores);
        assert!(cfg.preserve_patterns.is_empty());
    }

    #[test]
    fn typos_config_deserializes_empty() {
        let cfg: TyposConfig = toml::from_str("").unwrap();
        assert!(cfg.extend_words.is_empty());
        assert!(cfg.extend_identifiers.is_empty());
    }

    #[test]
    fn typos_config_rejects_unknown_fields() {
        let err = toml::from_str::<TyposConfig>("unknown_field = true");
        assert!(err.is_err(), "deny_unknown_fields must reject unrecognised keys");
    }
}
