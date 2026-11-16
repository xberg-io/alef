//! Core crate import path methods for `ResolvedCrateConfig`.

use std::path::{Path, PathBuf};

use super::ResolvedCrateConfig;
use crate::core::config::extras::Language;
use crate::core::config::resolve_helpers::{find_after_crates_prefix, relative_slash_path};

impl ResolvedCrateConfig {
    /// Get the core crate Rust import path (e.g., `"sample_llm"`).
    ///
    /// Returns `[crate] core_import` if set, otherwise derives it from the
    /// crate name by replacing hyphens with underscores.
    pub fn core_import_name(&self) -> String {
        self.core_import.clone().unwrap_or_else(|| self.name.replace('-', "_"))
    }

    /// Get the crate error type name (e.g., `"SampleCrateError"`).
    ///
    /// Returns `[crate] error_type` if set, otherwise `"Error"`.
    pub fn error_type_name(&self) -> String {
        self.error_type.clone().unwrap_or_else(|| "Error".to_string())
    }

    /// Get the error constructor pattern for backends without host exception retention.
    ///
    /// Returns `[crate] error_constructor` if set, otherwise generates
    /// `"{core_import}::{error_type}::from({msg})"`. The optional `{source}` becomes `None`.
    pub fn error_constructor_expr(&self) -> String {
        self.trait_bridge_error_constructor_expr().replace("{source}", "None")
    }

    pub(crate) fn trait_bridge_preserves_error_source(&self) -> bool {
        self.error_constructor
            .as_deref()
            .is_some_and(|expr| expr.contains("{source}"))
    }

    pub(crate) fn trait_bridge_error_constructor_expr(&self) -> String {
        self.error_constructor
            .clone()
            .unwrap_or_else(|| format!("{}::{}::from({{msg}})", self.core_import_name(), self.error_type_name()))
    }

    /// Get the directory name of the core crate (derived from sources or falling back to name).
    ///
    /// For example, if `sources` contains `"crates/sample-markdown/src/lib.rs"`, this returns
    /// `"sample-markdown"`. Used by the scaffold to generate correct `path = "../../crates/…"`
    /// references in binding-crate `Cargo.toml` files.
    pub fn core_crate_dir(&self) -> String {
        if let Some(first_source) = self.sources.first() {
            let path = std::path::Path::new(first_source);
            let mut current = path.parent();
            while let Some(dir) = current {
                if dir.file_name().is_some_and(|n| n == "src") {
                    if let Some(crate_dir) = dir.parent()
                        && let Some(dir_name) = crate_dir.file_name()
                    {
                        return dir_name.to_string_lossy().into_owned();
                    }
                    break;
                }
                current = dir.parent();
            }
        }
        self.name.clone()
    }

    /// The directory (relative to the project root) that holds the core crate's own
    /// `Cargo.toml`.
    ///
    /// Mirrors [`Self::core_crate_dir`]'s walk up from the first `sources` entry looking
    /// for a `src` component, but returns the whole path to that component's parent
    /// instead of only its final segment. A root-flat core crate (`sources =
    /// ["src/lib.rs"]`, the shape alef itself has used since 0.18.0) resolves to an empty
    /// path -- the project root -- rather than a sibling directory that does not exist; a
    /// workspace-shaped one (`sources = ["crates/my-core/src/lib.rs"]`) resolves to
    /// `crates/my-core`. Scaffolders derive a binding crate's core-dependency `path = ...`
    /// from this instead of assuming a fixed nesting depth. ~keep
    pub fn core_crate_root(&self) -> PathBuf {
        if let Some(first_source) = self.sources.first() {
            let path = Path::new(first_source);
            let mut current = path.parent();
            while let Some(dir) = current {
                if dir.file_name().is_some_and(|n| n == "src") {
                    return dir.parent().map(Path::to_path_buf).unwrap_or_default();
                }
                current = dir.parent();
            }
        }
        PathBuf::from("crates").join(&self.name)
    }

    /// The Cargo dependency `path = "..."` value a binding crate rooted at
    /// `binding_root` (itself relative to the project root, e.g. `crates/toolkit-ffi`)
    /// should use to reference this crate's core crate.
    ///
    /// Derives both the "from" and "to" side of the relative path from
    /// [`Self::core_crate_root`], so a root-flat and a workspace-shaped core crate each get
    /// the correct number of `..` segments instead of the single hard-coded `..` that only
    /// the workspace shape happens to satisfy.
    pub fn core_crate_dep_path(&self, binding_root: &Path) -> String {
        relative_slash_path(binding_root, &self.core_crate_root())
    }

    /// [`Self::core_crate_dep_path`], but honoring a language's
    /// `core_crate_override` when one is set.
    ///
    /// The override names an entirely different crate than the one [`Self::core_crate_root`]
    /// derives from `sources` -- e.g. a wasm-safe sub-crate the umbrella crate cannot target.
    /// That crate is not on the path `sources` describes, so deriving its location from
    /// `core_crate_root()` (as the override-blind [`Self::core_crate_dep_path`] does) silently
    /// points at the wrong directory instead of the override. The override crate is assumed to
    /// sit beside the binding crate, i.e. as `<binding_root's parent>/<override>`, matching the
    /// `crates/{name}-<lang>` sibling-of-`crates/{override}` convention every backend's default
    /// output layout uses. ~keep
    pub fn core_crate_dep_path_for_language(&self, binding_root: &Path, lang: Language) -> String {
        let override_name = match lang {
            Language::Wasm => self.wasm.as_ref().and_then(|c| c.core_crate_override.as_deref()),
            Language::Dart => self.dart.as_ref().and_then(|c| c.core_crate_override.as_deref()),
            Language::Swift => self.swift.as_ref().and_then(|c| c.core_crate_override.as_deref()),
            _ => None,
        };
        match override_name {
            Some(name) => {
                let sibling_root = binding_root
                    .parent()
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| binding_root.to_path_buf())
                    .join(name);
                relative_slash_path(binding_root, &sibling_root)
            }
            None => self.core_crate_dep_path(binding_root),
        }
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
                if let Some(after_crates) = find_after_crates_prefix(&source_str)
                    && let Some(slash_pos) = after_crates.find('/')
                {
                    let crate_dir = &after_crates[..slash_pos];
                    let crate_ident = crate_dir.replace('-', "_");
                    if crate_ident != core_import && !mappings.contains_key(&crate_ident) {
                        mappings.insert(crate_ident, core_import.clone());
                    }
                }
            }
        }

        for (from, to) in &self.path_mappings {
            mappings.insert(from.clone(), to.clone());
        }

        mappings
    }
}

#[cfg(test)]
mod tests {
    use crate::core::config::new_config::NewAlefConfig;

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
    fn error_constructor_expr_uses_none_when_the_backend_has_no_host_source() {
        let mut config = minimal();
        config.error_constructor = Some("MyError::new({msg}, {source})".to_owned());
        assert_eq!(config.error_constructor_expr(), "MyError::new({msg}, None)");
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
    fn core_crate_root_is_empty_for_a_root_flat_core_crate() {
        // `minimal()` uses `sources = ["src/lib.rs"]` -- the shape alef itself has used
        // since 0.18.0, with the core crate's own `Cargo.toml` at the project root.
        let r = minimal();
        assert_eq!(r.core_crate_root(), super::PathBuf::new());
    }

    #[test]
    fn core_crate_root_is_the_crates_sibling_for_a_workspace_core_crate() {
        let r = resolved_one(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "test-lib"
sources = ["crates/my-core/src/lib.rs"]
"#,
        );
        assert_eq!(r.core_crate_root(), super::PathBuf::from("crates/my-core"));
    }

    #[test]
    fn core_crate_dep_path_covers_both_layouts_from_the_default_binding_crate_root() {
        let root_flat = minimal();
        assert_eq!(
            root_flat.core_crate_dep_path(std::path::Path::new("crates/test-lib-ffi")),
            "../..",
            "root-flat: binding crate is two levels below the core crate's own Cargo.toml"
        );

        let workspace = resolved_one(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "test-lib"
sources = ["crates/my-core/src/lib.rs"]
"#,
        );
        assert_eq!(
            workspace.core_crate_dep_path(std::path::Path::new("crates/test-lib-ffi")),
            "../my-core",
            "workspace: core crate is a `crates/` sibling of the binding crate"
        );
    }

    #[test]
    fn core_crate_dep_path_for_language_targets_the_override_sibling_not_sources() {
        use crate::core::config::extras::Language;
        // `sources` describes a root-flat core crate (project root), but the wasm override
        // names an unrelated sub-crate that must resolve to a `crates/` sibling of the binding
        // crate instead -- not to wherever `sources` points. ~keep
        let r = resolved_one(
            r#"
[workspace]
languages = ["wasm"]

[[crates]]
name = "mylib"
sources = ["src/lib.rs"]

[crates.wasm]
core_crate_override = "mylib-core"
"#,
        );
        assert_eq!(
            r.core_crate_dep_path_for_language(std::path::Path::new("crates/mylib-wasm"), Language::Wasm),
            "../mylib-core"
        );
    }

    #[test]
    fn core_crate_dep_path_for_language_falls_back_without_an_override() {
        use crate::core::config::extras::Language;
        let r = minimal();
        assert_eq!(
            r.core_crate_dep_path_for_language(std::path::Path::new("crates/test-lib-wasm"), Language::Wasm),
            r.core_crate_dep_path(std::path::Path::new("crates/test-lib-wasm")),
        );
    }

    #[test]
    fn core_crate_for_language_uses_wasm_override() {
        use crate::core::config::extras::Language;
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
        use crate::core::config::extras::Language;
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
        assert_eq!(mappings.get("my_dep").map(|s| s.as_str()), Some("my_lib"));
        assert!(!mappings.contains_key("my_lib"));
    }
}
