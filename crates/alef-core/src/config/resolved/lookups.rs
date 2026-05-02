//! Pipeline config lookups: lint, test, setup, update, clean, build, extras.

use std::collections::HashMap;

use super::ResolvedCrateConfig;
use crate::config::extras::Language;
use crate::config::output::{BuildCommandConfig, CleanConfig, LintConfig, SetupConfig, TestConfig, UpdateConfig};
use crate::config::tools::LangContext;
use crate::config::{build_defaults, clean_defaults, lint_defaults, setup_defaults, test_defaults, update_defaults};

impl ResolvedCrateConfig {
    /// Get the package output directory for a language.
    /// Uses `scaffold_output` from per-language config if set, otherwise defaults.
    pub fn package_dir(&self, lang: Language) -> String {
        let override_path = match lang {
            Language::Python => self.python.as_ref().and_then(|c| c.scaffold_output.as_ref()),
            Language::Node => self.node.as_ref().and_then(|c| c.scaffold_output.as_ref()),
            Language::Ruby => self.ruby.as_ref().and_then(|c| c.scaffold_output.as_ref()),
            Language::Php => self.php.as_ref().and_then(|c| c.scaffold_output.as_ref()),
            Language::Elixir => self.elixir.as_ref().and_then(|c| c.scaffold_output.as_ref()),
            _ => None,
        };
        if let Some(p) = override_path {
            p.to_string_lossy().to_string()
        } else {
            match lang {
                Language::Python => "packages/python".to_string(),
                Language::Node => "packages/node".to_string(),
                Language::Ruby => "packages/ruby".to_string(),
                Language::Php => "packages/php".to_string(),
                Language::Elixir => "packages/elixir".to_string(),
                _ => format!("packages/{lang}"),
            }
        }
    }

    /// Get the run_wrapper for a language, if set.
    pub fn run_wrapper_for_language(&self, lang: Language) -> Option<&str> {
        match lang {
            Language::Python => self.python.as_ref().and_then(|c| c.run_wrapper.as_deref()),
            Language::Node => self.node.as_ref().and_then(|c| c.run_wrapper.as_deref()),
            Language::Ruby => self.ruby.as_ref().and_then(|c| c.run_wrapper.as_deref()),
            Language::Php => self.php.as_ref().and_then(|c| c.run_wrapper.as_deref()),
            Language::Elixir => self.elixir.as_ref().and_then(|c| c.run_wrapper.as_deref()),
            Language::Wasm => self.wasm.as_ref().and_then(|c| c.run_wrapper.as_deref()),
            Language::Go => self.go.as_ref().and_then(|c| c.run_wrapper.as_deref()),
            Language::Java => self.java.as_ref().and_then(|c| c.run_wrapper.as_deref()),
            Language::Csharp => self.csharp.as_ref().and_then(|c| c.run_wrapper.as_deref()),
            Language::R => self.r.as_ref().and_then(|c| c.run_wrapper.as_deref()),
            Language::Kotlin => self.kotlin.as_ref().and_then(|c| c.run_wrapper.as_deref()),
            Language::Dart => self.dart.as_ref().and_then(|c| c.run_wrapper.as_deref()),
            Language::Swift => self.swift.as_ref().and_then(|c| c.run_wrapper.as_deref()),
            Language::Gleam => self.gleam.as_ref().and_then(|c| c.run_wrapper.as_deref()),
            Language::Zig => self.zig.as_ref().and_then(|c| c.run_wrapper.as_deref()),
            Language::Ffi | Language::Rust => None,
        }
    }

    /// Get the extra_lint_paths for a language.
    pub fn extra_lint_paths_for_language(&self, lang: Language) -> &[String] {
        match lang {
            Language::Python => self
                .python
                .as_ref()
                .map(|c| c.extra_lint_paths.as_slice())
                .unwrap_or(&[]),
            Language::Node => self.node.as_ref().map(|c| c.extra_lint_paths.as_slice()).unwrap_or(&[]),
            Language::Ruby => self.ruby.as_ref().map(|c| c.extra_lint_paths.as_slice()).unwrap_or(&[]),
            Language::Php => self.php.as_ref().map(|c| c.extra_lint_paths.as_slice()).unwrap_or(&[]),
            Language::Elixir => self
                .elixir
                .as_ref()
                .map(|c| c.extra_lint_paths.as_slice())
                .unwrap_or(&[]),
            Language::Wasm => self.wasm.as_ref().map(|c| c.extra_lint_paths.as_slice()).unwrap_or(&[]),
            Language::Go => self.go.as_ref().map(|c| c.extra_lint_paths.as_slice()).unwrap_or(&[]),
            Language::Java => self.java.as_ref().map(|c| c.extra_lint_paths.as_slice()).unwrap_or(&[]),
            Language::Csharp => self
                .csharp
                .as_ref()
                .map(|c| c.extra_lint_paths.as_slice())
                .unwrap_or(&[]),
            Language::R => self.r.as_ref().map(|c| c.extra_lint_paths.as_slice()).unwrap_or(&[]),
            Language::Kotlin => self
                .kotlin
                .as_ref()
                .map(|c| c.extra_lint_paths.as_slice())
                .unwrap_or(&[]),
            Language::Dart => self.dart.as_ref().map(|c| c.extra_lint_paths.as_slice()).unwrap_or(&[]),
            Language::Swift => self
                .swift
                .as_ref()
                .map(|c| c.extra_lint_paths.as_slice())
                .unwrap_or(&[]),
            Language::Gleam => self
                .gleam
                .as_ref()
                .map(|c| c.extra_lint_paths.as_slice())
                .unwrap_or(&[]),
            Language::Zig => self.zig.as_ref().map(|c| c.extra_lint_paths.as_slice()).unwrap_or(&[]),
            Language::Ffi | Language::Rust => &[],
        }
    }

    /// Get the project_file for a language (Java or C# only).
    pub fn project_file_for_language(&self, lang: Language) -> Option<&str> {
        match lang {
            Language::Java => self.java.as_ref().and_then(|c| c.project_file.as_deref()),
            Language::Csharp => self.csharp.as_ref().and_then(|c| c.project_file.as_deref()),
            _ => None,
        }
    }

    /// Get the effective lint configuration for a language.
    pub fn lint_config_for_language(&self, lang: Language) -> LintConfig {
        let lang_str = lang.to_string();
        if let Some(explicit) = self.lint.get(&lang_str) {
            return explicit.clone();
        }
        let output_dir = self.package_dir(lang);
        let run_wrapper = self.run_wrapper_for_language(lang);
        let extra_lint_paths = self.extra_lint_paths_for_language(lang);
        let project_file = self.project_file_for_language(lang);
        let ctx = LangContext {
            tools: &self.tools,
            run_wrapper,
            extra_lint_paths,
            project_file,
        };
        lint_defaults::default_lint_config(lang, &output_dir, &ctx)
    }

    /// Get the effective update configuration for a language.
    pub fn update_config_for_language(&self, lang: Language) -> UpdateConfig {
        let lang_str = lang.to_string();
        if let Some(explicit) = self.update.get(&lang_str) {
            return explicit.clone();
        }
        let output_dir = self.package_dir(lang);
        let ctx = LangContext {
            tools: &self.tools,
            run_wrapper: None,
            extra_lint_paths: &[],
            project_file: None,
        };
        update_defaults::default_update_config(lang, &output_dir, &ctx)
    }

    /// Get the effective test configuration for a language.
    pub fn test_config_for_language(&self, lang: Language) -> TestConfig {
        let lang_str = lang.to_string();
        if let Some(explicit) = self.test.get(&lang_str) {
            return explicit.clone();
        }
        let output_dir = self.package_dir(lang);
        let run_wrapper = self.run_wrapper_for_language(lang);
        let project_file = self.project_file_for_language(lang);
        let ctx = LangContext {
            tools: &self.tools,
            run_wrapper,
            extra_lint_paths: &[],
            project_file,
        };
        test_defaults::default_test_config(lang, &output_dir, &ctx)
    }

    /// Get the effective setup configuration for a language.
    pub fn setup_config_for_language(&self, lang: Language) -> SetupConfig {
        let lang_str = lang.to_string();
        if let Some(explicit) = self.setup.get(&lang_str) {
            return explicit.clone();
        }
        let output_dir = self.package_dir(lang);
        let ctx = LangContext {
            tools: &self.tools,
            run_wrapper: None,
            extra_lint_paths: &[],
            project_file: None,
        };
        setup_defaults::default_setup_config(lang, &output_dir, &ctx)
    }

    /// Get the effective clean configuration for a language.
    pub fn clean_config_for_language(&self, lang: Language) -> CleanConfig {
        let lang_str = lang.to_string();
        if let Some(explicit) = self.clean.get(&lang_str) {
            return explicit.clone();
        }
        let output_dir = self.package_dir(lang);
        let ctx = LangContext {
            tools: &self.tools,
            run_wrapper: None,
            extra_lint_paths: &[],
            project_file: None,
        };
        clean_defaults::default_clean_config(lang, &output_dir, &ctx)
    }

    /// Get the effective build command configuration for a language.
    pub fn build_command_config_for_language(&self, lang: Language) -> BuildCommandConfig {
        let lang_str = lang.to_string();
        if let Some(explicit) = self.build_commands.get(&lang_str) {
            return explicit.clone();
        }
        let output_dir = self.package_dir(lang);
        let run_wrapper = self.run_wrapper_for_language(lang);
        let project_file = self.project_file_for_language(lang);
        let ctx = LangContext {
            tools: &self.tools,
            run_wrapper,
            extra_lint_paths: &[],
            project_file,
        };
        build_defaults::default_build_config(lang, &output_dir, &self.name, &ctx)
    }

    /// Get the features to use for a specific language's binding crate.
    pub fn features_for_language(&self, lang: Language) -> &[String] {
        let override_features = match lang {
            Language::Python => self.python.as_ref().and_then(|c| c.features.as_deref()),
            Language::Node => self.node.as_ref().and_then(|c| c.features.as_deref()),
            Language::Ruby => self.ruby.as_ref().and_then(|c| c.features.as_deref()),
            Language::Php => self.php.as_ref().and_then(|c| c.features.as_deref()),
            Language::Elixir => self.elixir.as_ref().and_then(|c| c.features.as_deref()),
            Language::Wasm => self.wasm.as_ref().and_then(|c| c.features.as_deref()),
            Language::Ffi => self.ffi.as_ref().and_then(|c| c.features.as_deref()),
            Language::Gleam => self.gleam.as_ref().and_then(|c| c.features.as_deref()),
            Language::Go => self.go.as_ref().and_then(|c| c.features.as_deref()),
            Language::Java => self.java.as_ref().and_then(|c| c.features.as_deref()),
            Language::Kotlin => self.kotlin.as_ref().and_then(|c| c.features.as_deref()),
            Language::Csharp => self.csharp.as_ref().and_then(|c| c.features.as_deref()),
            Language::R => self.r.as_ref().and_then(|c| c.features.as_deref()),
            Language::Zig => self.zig.as_ref().and_then(|c| c.features.as_deref()),
            Language::Dart => self.dart.as_ref().and_then(|c| c.features.as_deref()),
            Language::Swift => self.swift.as_ref().and_then(|c| c.features.as_deref()),
            Language::Rust => None,
        };
        override_features.unwrap_or(&self.features)
    }

    /// Get the merged extra dependencies for a specific language's binding crate.
    pub fn extra_deps_for_language(&self, lang: Language) -> HashMap<String, toml::Value> {
        let mut deps = self.extra_dependencies.clone();
        let lang_deps = match lang {
            Language::Python => self.python.as_ref().map(|c| &c.extra_dependencies),
            Language::Node => self.node.as_ref().map(|c| &c.extra_dependencies),
            Language::Ruby => self.ruby.as_ref().map(|c| &c.extra_dependencies),
            Language::Php => self.php.as_ref().map(|c| &c.extra_dependencies),
            Language::Elixir => self.elixir.as_ref().map(|c| &c.extra_dependencies),
            Language::Wasm => self.wasm.as_ref().map(|c| &c.extra_dependencies),
            _ => None,
        };
        if let Some(lang_deps) = lang_deps {
            deps.extend(lang_deps.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
        let exclude: &[String] = match lang {
            Language::Wasm => self
                .wasm
                .as_ref()
                .map(|c| c.exclude_extra_dependencies.as_slice())
                .unwrap_or(&[]),
            Language::Dart => self
                .dart
                .as_ref()
                .map(|c| c.exclude_extra_dependencies.as_slice())
                .unwrap_or(&[]),
            Language::Swift => self
                .swift
                .as_ref()
                .map(|c| c.exclude_extra_dependencies.as_slice())
                .unwrap_or(&[]),
            _ => &[],
        };
        for key in exclude {
            deps.remove(key);
        }
        deps
    }
}

#[cfg(test)]
mod tests {
    use crate::config::extras::Language;
    use crate::config::new_config::NewAlefConfig;

    fn resolved_one(toml: &str) -> super::super::ResolvedCrateConfig {
        let cfg: NewAlefConfig = toml::from_str(toml).unwrap();
        cfg.resolve().unwrap().remove(0)
    }

    fn minimal() -> super::super::ResolvedCrateConfig {
        resolved_one(
            r#"
[workspace]
languages = ["python", "node"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
"#,
        )
    }

    #[test]
    fn resolved_lint_config_inherits_workspace_when_crate_unset() {
        let r = resolved_one(
            r#"
[workspace]
languages = ["python"]

[workspace.lint.python]
check = "ruff check ."

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
"#,
        );
        let lint = r.lint_config_for_language(Language::Python);
        assert_eq!(lint.check.unwrap().commands(), vec!["ruff check ."]);
    }

    #[test]
    fn resolved_lint_config_crate_overrides_workspace_field_wholesale() {
        let r = resolved_one(
            r#"
[workspace]
languages = ["python"]

[workspace.lint.python]
check = "ruff check ."

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.lint.python]
check = "ruff check crates/test-lib-py/"
"#,
        );
        let lint = r.lint_config_for_language(Language::Python);
        assert_eq!(lint.check.unwrap().commands(), vec!["ruff check crates/test-lib-py/"]);
    }

    #[test]
    fn resolved_features_per_language_overrides_crate_default() {
        let r = resolved_one(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
features = ["base"]

[crates.python]
features = ["python-extra"]
"#,
        );
        assert_eq!(r.features_for_language(Language::Python), &["python-extra"]);
        assert_eq!(r.features_for_language(Language::Node), &["base"]);
    }

    #[test]
    fn resolved_extra_deps_crate_value_wins_on_key_collision() {
        let r = resolved_one(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.extra_dependencies]
tokio = "1"

[crates.python]
extra_dependencies = { tokio = "2" }
"#,
        );
        let deps = r.extra_deps_for_language(Language::Python);
        let tokio = deps.get("tokio").unwrap().as_str().unwrap();
        assert_eq!(tokio, "2", "per-language extra_dep should win on collision");
    }

    #[test]
    fn resolved_extra_deps_excludes_apply_after_merge() {
        let r = resolved_one(
            r#"
[workspace]
languages = ["wasm"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.extra_dependencies]
tokio = "1"
serde = "1"

[crates.wasm]
exclude_extra_dependencies = ["tokio"]
"#,
        );
        let deps = r.extra_deps_for_language(Language::Wasm);
        assert!(!deps.contains_key("tokio"), "excluded dep should be absent");
        assert!(deps.contains_key("serde"), "non-excluded dep should be present");
    }

    #[test]
    fn package_dir_defaults_are_correct() {
        let r = minimal();
        assert_eq!(r.package_dir(Language::Python), "packages/python");
        assert_eq!(r.package_dir(Language::Node), "packages/node");
        assert_eq!(r.package_dir(Language::Ruby), "packages/ruby");
        assert_eq!(r.package_dir(Language::Go), "packages/go");
        assert_eq!(r.package_dir(Language::Java), "packages/java");
    }
}
