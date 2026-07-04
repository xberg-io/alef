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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poly_config_deserializes_empty() {
        let cfg: PolyConfig = toml::from_str("").unwrap();
        assert!(cfg.exclude.is_empty());
        assert!(cfg.typos.extend_words.is_empty());
        assert!(cfg.typos.extend_identifiers.is_empty());
        assert!(cfg.per_file_ignores.is_empty());
    }

    #[test]
    fn poly_config_deserializes_full() {
        let toml_str = r#"
exclude = ["vendor/**", "third-party/**"]

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
"#;
        let cfg: PolyConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.exclude, vec!["vendor/**", "third-party/**"]);
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
