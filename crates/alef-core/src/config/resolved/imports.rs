//! Core crate import path methods for `ResolvedCrateConfig`.

use super::ResolvedCrateConfig;
use crate::config::extras::Language;
use crate::config::resolve_helpers::find_after_crates_prefix;

impl ResolvedCrateConfig {
    /// Get the core crate Rust import path (e.g., `"liter_llm"`).
    ///
    /// Returns `[crate] core_import` if set, otherwise derives it from the
    /// crate name by replacing hyphens with underscores.
    pub fn core_import_name(&self) -> String {
        self.core_import.clone().unwrap_or_else(|| self.name.replace('-', "_"))
    }

    /// Get the crate error type name (e.g., `"KreuzbergError"`).
    ///
    /// Returns `[crate] error_type` if set, otherwise `"Error"`.
    pub fn error_type_name(&self) -> String {
        self.error_type.clone().unwrap_or_else(|| "Error".to_string())
    }

    /// Get the error constructor pattern. `{msg}` is replaced with the message expression.
    ///
    /// Returns `[crate] error_constructor` if set, otherwise generates
    /// `"{core_import}::{error_type}::from({msg})"`.
    pub fn error_constructor_expr(&self) -> String {
        self.error_constructor
            .clone()
            .unwrap_or_else(|| format!("{}::{}::from({{msg}})", self.core_import_name(), self.error_type_name()))
    }

    /// Get the directory name of the core crate (derived from sources or falling back to name).
    ///
    /// For example, if `sources` contains `"crates/html-to-markdown/src/lib.rs"`, this returns
    /// `"html-to-markdown"`. Used by the scaffold to generate correct `path = "../../crates/…"`
    /// references in binding-crate `Cargo.toml` files.
    pub fn core_crate_dir(&self) -> String {
        // Try to derive from first source path: "crates/foo/src/types/config.rs" → "foo"
        // Walk up from the file until we find the "src" directory, then take its parent.
        if let Some(first_source) = self.sources.first() {
            let path = std::path::Path::new(first_source);
            let mut current = path.parent();
            while let Some(dir) = current {
                if dir.file_name().is_some_and(|n| n == "src") {
                    if let Some(crate_dir) = dir.parent() {
                        if let Some(dir_name) = crate_dir.file_name() {
                            return dir_name.to_string_lossy().into_owned();
                        }
                    }
                    break;
                }
                current = dir.parent();
            }
        }
        self.name.clone()
    }

    /// Resolve the core Cargo dependency name (and matching directory) for a
    /// language's binding crate.
    ///
    /// Returns `[<lang>].core_crate_override` when set (currently honored for
    /// `wasm`, `dart`, `swift`), otherwise falls back to [`Self::core_crate_dir`].
    pub fn core_crate_for_language(&self, lang: Language) -> String {
        let override_name = match lang {
            Language::Wasm => self.wasm.as_ref().and_then(|c| c.core_crate_override.as_deref()),
            Language::Dart => self.dart.as_ref().and_then(|c| c.core_crate_override.as_deref()),
            Language::Swift => self.swift.as_ref().and_then(|c| c.core_crate_override.as_deref()),
            _ => None,
        };
        match override_name {
            Some(name) => name.to_string(),
            None => self.core_crate_dir(),
        }
    }

    /// Resolve the core crate Rust import path for a language's binding crate.
    ///
    /// When `[<lang>].core_crate_override` is set, the override name (with `-`
    /// translated to `_`) is used so that generated `use` paths and `From`
    /// impls reference the overridden crate. Otherwise falls back to
    /// [`Self::core_import_name`].
    pub fn core_import_for_language(&self, lang: Language) -> String {
        let override_name = match lang {
            Language::Wasm => self.wasm.as_ref().and_then(|c| c.core_crate_override.as_deref()),
            Language::Dart => self.dart.as_ref().and_then(|c| c.core_crate_override.as_deref()),
            Language::Swift => self.swift.as_ref().and_then(|c| c.core_crate_override.as_deref()),
            _ => None,
        };
        match override_name {
            Some(name) => name.replace('-', "_"),
            None => self.core_import_name(),
        }
    }

    /// Return the effective path mappings for this crate.
    ///
    /// When `auto_path_mappings` is true, automatically derives a mapping from each source
    /// crate to the configured `core_import` facade. For each source file whose path contains
    /// `crates/{crate-name}/src/`, a mapping `{crate_name}` → `{core_import}` is added
    /// (hyphens in the crate name are converted to underscores). Source crates that already
    /// equal `core_import` are skipped.
    ///
    /// Explicit entries in `path_mappings` always override auto-derived ones.
    pub fn effective_path_mappings(&self) -> std::collections::HashMap<String, String> {
        let mut mappings = std::collections::HashMap::new();

        if self.auto_path_mappings {
            let core_import = self.core_import_name();

            for source in &self.sources {
                let source_str = source.to_string_lossy();
                if let Some(after_crates) = find_after_crates_prefix(&source_str) {
                    if let Some(slash_pos) = after_crates.find('/') {
                        let crate_dir = &after_crates[..slash_pos];
                        let crate_ident = crate_dir.replace('-', "_");
                        if crate_ident != core_import && !mappings.contains_key(&crate_ident) {
                            mappings.insert(crate_ident, core_import.clone());
                        }
                    }
                }
            }
        }

        // Explicit path_mappings always win — insert last so they overwrite auto entries.
        for (from, to) in &self.path_mappings {
            mappings.insert(from.clone(), to.clone());
        }

        mappings
    }
}

#[cfg(test)]
mod tests {
    use crate::config::new_config::NewAlefConfig;

    fn resolved_one(toml: &str) -> super::super::ResolvedCrateConfig {
        let cfg: NewAlefConfig = toml::from_str(toml).unwrap();
        cfg.resolve().unwrap().remove(0)
    }

    fn minimal() -> super::super::ResolvedCrateConfig {
        resolved_one(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
"#,
        )
    }

    #[test]
    fn core_import_name_defaults_to_snake_case_name() {
        let r = minimal();
        assert_eq!(r.core_import_name(), "test_lib");
    }

    #[test]
    fn core_import_name_explicit_wins() {
        let r = resolved_one(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
core_import = "custom_core"
"#,
        );
        assert_eq!(r.core_import_name(), "custom_core");
    }

    #[test]
    fn error_type_name_defaults_to_error() {
        let r = minimal();
        assert_eq!(r.error_type_name(), "Error");
    }

    #[test]
    fn error_type_name_explicit_wins() {
        let r = resolved_one(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
error_type = "MyError"
"#,
        );
        assert_eq!(r.error_type_name(), "MyError");
    }

    #[test]
    fn error_constructor_expr_defaults_to_from_pattern() {
        let r = minimal();
        assert_eq!(r.error_constructor_expr(), "test_lib::Error::from({msg})");
    }

    #[test]
    fn error_constructor_expr_explicit_wins() {
        let r = resolved_one(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
error_constructor = "MyError::new({msg})"
"#,
        );
        assert_eq!(r.error_constructor_expr(), "MyError::new({msg})");
    }

    #[test]
    fn core_crate_dir_from_source_path() {
        let r = resolved_one(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "test-lib"
sources = ["crates/my-core/src/lib.rs"]
"#,
        );
        assert_eq!(r.core_crate_dir(), "my-core");
    }

    #[test]
    fn core_crate_dir_falls_back_to_name() {
        let r = minimal();
        assert_eq!(r.core_crate_dir(), "test-lib");
    }

    #[test]
    fn core_crate_for_language_uses_wasm_override() {
        use crate::config::extras::Language;
        let r = resolved_one(
            r#"
[workspace]
languages = ["wasm"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.wasm]
core_crate_override = "test-lib-wasm-core"
"#,
        );
        assert_eq!(r.core_crate_for_language(Language::Wasm), "test-lib-wasm-core");
    }

    #[test]
    fn core_import_for_language_normalizes_override_hyphens() {
        use crate::config::extras::Language;
        let r = resolved_one(
            r#"
[workspace]
languages = ["wasm"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.wasm]
core_crate_override = "test-lib-wasm-core"
"#,
        );
        assert_eq!(r.core_import_for_language(Language::Wasm), "test_lib_wasm_core");
    }

    #[test]
    fn resolved_path_mappings_per_crate_only() {
        let r = resolved_one(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
path_mappings = { "old_mod" = "new_mod" }
"#,
        );
        let mappings = r.effective_path_mappings();
        assert_eq!(mappings.get("old_mod").map(|s| s.as_str()), Some("new_mod"));
    }

    #[test]
    fn effective_path_mappings_auto_derives_from_sources() {
        let r = resolved_one(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "my-lib"
sources = ["crates/my-dep/src/lib.rs", "crates/my-lib/src/lib.rs"]
core_import = "my_lib"
auto_path_mappings = true
"#,
        );
        let mappings = r.effective_path_mappings();
        // my-dep differs from core import → auto-derived
        assert_eq!(mappings.get("my_dep").map(|s| s.as_str()), Some("my_lib"));
        // my-lib matches core import → skipped
        assert!(!mappings.contains_key("my_lib"));
    }
}
