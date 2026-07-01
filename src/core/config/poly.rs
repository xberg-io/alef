//! Repository-level `poly.toml` customisations — per-repo additions that
//! survive regeneration.
//!
//! Alef regenerates `poly.toml` on every run, so any repo-specific lint
//! suppressions (e.g. `[discovery] exclude` extras, per-file-ignores) must
//! live in `alef.toml` under `[workspace.poly]` rather than in the generated
//! file itself.  The poly emitter reads this struct and merges the extra
//! entries into its output.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poly_config_deserializes_empty() {
        let cfg: PolyConfig = toml::from_str("").unwrap();
        assert!(cfg.exclude.is_empty());
        assert!(cfg.per_file_ignores.is_empty());
    }

    #[test]
    fn poly_config_deserializes_full() {
        let toml_str = r#"
exclude = ["vendor/**", "third-party/**"]

[per-file-ignores]
"**/legacy.py" = ["ANN", "D103"]
"**/compat.py" = ["UP035"]
"#;
        let cfg: PolyConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.exclude, vec!["vendor/**", "third-party/**"]);
        assert_eq!(cfg.per_file_ignores.len(), 2);
        assert_eq!(cfg.per_file_ignores["**/legacy.py"], vec!["ANN", "D103"]);
        assert_eq!(cfg.per_file_ignores["**/compat.py"], vec!["UP035"]);
    }

    #[test]
    fn poly_config_rejects_unknown_fields() {
        let err = toml::from_str::<PolyConfig>("unknown_field = true");
        assert!(err.is_err(), "deny_unknown_fields must reject unrecognised keys");
    }
}
