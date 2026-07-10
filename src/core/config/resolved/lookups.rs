//! Pipeline config lookups: lint, test, setup, update, clean, build, extras.

use std::collections::HashMap;

use super::ResolvedCrateConfig;
use crate::core::config::extras::{AdapterConfig, Language};
use crate::core::config::output::{
    BuildCommandConfig, CleanConfig, LintConfig, SetupConfig, TestAppRunConfig, TestConfig, UpdateConfig,
};
use crate::core::config::tools::LangContext;
use crate::core::config::{
    build_defaults, clean_defaults, lint_defaults, setup_defaults, test_apps_run_defaults, test_defaults,
    update_defaults,
};

impl ResolvedCrateConfig {
    /// Find the [`AdapterConfig`] whose `name` matches `fn_name`, if any.
    ///
    /// Used by e2e codegen to check whether a call function is routed through
    /// a streaming adapter and, if so, to retrieve its `request_type` so the
    /// generated test wraps the raw `mock_url` binding in the typed request
    /// constructor.
    pub fn adapter_for_function(&self, fn_name: &str) -> Option<&AdapterConfig> {
        self.adapters.iter().find(|a| a.name == fn_name)
    }
    /// Get the package output directory for a language.
    /// Uses resolved output_paths (from [crates.output] config) if available,
    /// otherwise falls back to scaffold_output overrides and hardcoded defaults.
    /// For Node and Wasm, checks `crate_dir` override before the default formula.
    pub fn package_dir(&self, lang: Language) -> String {
        if !matches!(
            lang,
            Language::Python
                | Language::Node
                | Language::Wasm
                | Language::Elixir
                | Language::Ruby
                | Language::Java
                | Language::Swift
                | Language::Zig
                | Language::Dart
                | Language::Kotlin
                | Language::KotlinAndroid
                | Language::R
                | Language::Go
        ) && let Some(output_path) = self.output_paths.get(&lang.to_string())
        {
            return output_path.to_string_lossy().to_string();
        }

        let override_path = match lang {
            Language::Python => self.python.as_ref().and_then(|c| c.scaffold_output.as_ref()),
            Language::Node => self.node.as_ref().and_then(|c| c.scaffold_output.as_ref()),
            Language::Ruby => self.ruby.as_ref().and_then(|c| c.scaffold_output.as_ref()),
            Language::Php => self.php.as_ref().and_then(|c| c.scaffold_output.as_ref()),
            Language::Elixir => self.elixir.as_ref().and_then(|c| c.scaffold_output.as_ref()),
            _ => None,
        };
        if let Some(p) = override_path {
            return p.to_string_lossy().to_string();
        }

        match lang {
            Language::Python => "packages/python".to_string(),
            Language::Node => self
                .node
                .as_ref()
                .and_then(|c| c.crate_dir.as_ref())
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("crates/{}-node", self.name)),
            Language::Wasm => self
                .wasm
                .as_ref()
                .and_then(|c| c.crate_dir.as_ref())
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("crates/{}-wasm", self.name)),
            Language::Ruby => "packages/ruby".to_string(),
            Language::Php => "packages/php".to_string(),
            Language::Elixir => "packages/elixir".to_string(),
            Language::Go => match self.go.as_ref().and_then(|c| c.module_major) {
                Some(n) if n >= 2 => format!("packages/go/v{n}"),
                _ => "packages/go".to_string(),
            },
            Language::KotlinAndroid => "packages/kotlin-android".to_string(),
            _ => format!("packages/{lang}"),
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
            Language::KotlinAndroid => self.kotlin_android.as_ref().and_then(|c| c.run_wrapper.as_deref()),
            Language::Dart => self.dart.as_ref().and_then(|c| c.run_wrapper.as_deref()),
            Language::Swift => self.swift.as_ref().and_then(|c| c.run_wrapper.as_deref()),
            Language::Gleam => self.gleam.as_ref().and_then(|c| c.run_wrapper.as_deref()),
            Language::Zig => self.zig.as_ref().and_then(|c| c.run_wrapper.as_deref()),
            Language::Ffi | Language::Rust | Language::C | Language::Jni => None,
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
            Language::KotlinAndroid => self
                .kotlin_android
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
            Language::Ffi | Language::Rust | Language::C | Language::Jni => &[],
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

    /// Resolve the test-app run configuration for a registry test-app `name`.
    ///
    /// `name` is an `[e2e].languages` entry — usually a language slug (`python`,
    /// `node`, …) but also string-only registry targets such as `brew`. Returns
    /// the explicit `[crates.e2e.registry.run.<name>]` override if present, else
    /// the default from [`test_apps_run_defaults`]. The test app lives under the
    /// registry `output` directory (default `test_apps`).
    pub fn test_apps_run_config_for_name(&self, name: &str) -> TestAppRunConfig {
        let test_apps_dir = self
            .e2e
            .as_ref()
            .map(|e2e| e2e.registry.output.as_str())
            .unwrap_or("test_apps");
        if let Some(explicit) = self.e2e.as_ref().and_then(|e2e| e2e.registry.run.get(name)) {
            return explicit.clone();
        }
        let ctx = LangContext {
            tools: &self.tools,
            run_wrapper: None,
            extra_lint_paths: &[],
            project_file: None,
        };
        let parsed: Result<Language, _> = toml::Value::String(name.to_string()).try_into();
        match parsed {
            Ok(lang) => {
                let published_version = self
                    .e2e
                    .as_ref()
                    .and_then(|e2e| e2e.resolve_package(name))
                    .and_then(|pkg| pkg.version)
                    .or_else(|| self.resolved_version());
                let go_module = if lang == Language::Go {
                    Some(self.go_module())
                } else {
                    None
                };
                test_apps_run_defaults::default_test_apps_run_config(
                    lang,
                    test_apps_dir,
                    &ctx,
                    published_version.as_deref(),
                    go_module.as_deref(),
                )
            }
            Err(_) => test_apps_run_defaults::default_test_apps_run_config_for_name(name, test_apps_dir, &ctx),
        }
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
        let output_dir = self.package_dir(lang);
        let run_wrapper = self.run_wrapper_for_language(lang);
        let project_file = self.project_file_for_language(lang);
        let ctx = LangContext {
            tools: &self.tools,
            run_wrapper,
            extra_lint_paths: &[],
            project_file,
        };
        let default = build_defaults::default_build_config(lang, &output_dir, &self.name, &ctx);
        if let Some(explicit) = self.build_commands.get(&lang_str) {
            default.merge_overlay(explicit)
        } else {
            default
        }
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
            Language::Go => self.go.as_ref().and_then(|c| c.features.as_deref()),
            Language::Java => self.java.as_ref().and_then(|c| c.features.as_deref()),
            Language::Kotlin => self.kotlin.as_ref().and_then(|c| c.features.as_deref()),
            Language::KotlinAndroid => self.kotlin_android.as_ref().and_then(|c| c.features.as_deref()),
            Language::Csharp => self.csharp.as_ref().and_then(|c| c.features.as_deref()),
            Language::R => self.r.as_ref().and_then(|c| c.features.as_deref()),
            Language::Zig => self.zig.as_ref().and_then(|c| c.features.as_deref()),
            Language::Dart => self.dart.as_ref().and_then(|c| c.features.as_deref()),
            Language::Swift => self.swift.as_ref().and_then(|c| c.features.as_deref()),
            Language::Gleam => self.gleam.as_ref().and_then(|c| c.features.as_deref()),
            Language::Rust | Language::C | Language::Jni => None,
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
            Language::Dart => self.dart.as_ref().map(|c| &c.extra_dependencies),
            Language::Swift => self.swift.as_ref().map(|c| &c.extra_dependencies),
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
    use crate::core::config::extras::Language;
    use crate::core::config::new_config::NewAlefConfig;

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
    fn resolved_extra_deps_includes_swift_overrides() {
        let r = resolved_one(
            r#"
[workspace]
languages = ["swift"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.extra_dependencies]
serde = "1"

[crates.swift.extra_dependencies]
tokio = "1"
"#,
        );
        let deps = r.extra_deps_for_language(Language::Swift);
        assert!(deps.contains_key("serde"), "crate-level dep should be present");
        assert!(deps.contains_key("tokio"), "Swift dep should be present");
    }

    #[test]
    fn package_dir_defaults_are_correct() {
        let r = minimal();
        assert_eq!(r.package_dir(Language::Python), "packages/python");
        assert_eq!(r.package_dir(Language::Node), format!("crates/{}-node", r.name));
        assert_eq!(r.package_dir(Language::Wasm), format!("crates/{}-wasm", r.name));
        assert_eq!(r.package_dir(Language::Ruby), "packages/ruby");
        assert_eq!(r.package_dir(Language::Go), "packages/go");
        assert_eq!(r.package_dir(Language::Java), "packages/java");
        assert_eq!(r.package_dir(Language::Kotlin), "packages/kotlin");
        assert_eq!(r.package_dir(Language::KotlinAndroid), "packages/kotlin-android");
    }

    #[test]
    fn package_dir_go_with_module_major() {
        let r_none = resolved_one(
            r#"
[workspace]
languages = ["go"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
"#,
        );
        assert_eq!(
            r_none.package_dir(Language::Go),
            "packages/go",
            "Go without module_major should default to 'packages/go'"
        );

        let r_v1 = resolved_one(
            r#"
[workspace]
languages = ["go"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.go]
module_major = 1
"#,
        );
        assert_eq!(
            r_v1.package_dir(Language::Go),
            "packages/go",
            "Go with module_major = 1 should use 'packages/go'"
        );

        let r_v2 = resolved_one(
            r#"
[workspace]
languages = ["go"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.go]
module_major = 2
"#,
        );
        assert_eq!(
            r_v2.package_dir(Language::Go),
            "packages/go/v2",
            "Go with module_major = 2 should use 'packages/go/v2'"
        );

        let r_v5 = resolved_one(
            r#"
[workspace]
languages = ["go"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.go]
module_major = 5
"#,
        );
        assert_eq!(
            r_v5.package_dir(Language::Go),
            "packages/go/v5",
            "Go with module_major = 5 should use 'packages/go/v5'"
        );
    }

    #[test]
    fn package_dir_r_ignores_source_output_override() {
        let r = resolved_one(
            r#"
[workspace]
languages = ["r"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]

[crates.output]
r = "packages/r/src/rust/src/"
"#,
        );
        assert_eq!(r.package_dir(Language::R), "packages/r");
    }

    #[test]
    fn package_dir_python_ignores_source_output_override() {
        let r = resolved_one(
            r#"
[workspace]
languages = ["python"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]

[crates.output]
python = "crates/demo-py/src/"
"#,
        );
        assert_eq!(r.package_dir(Language::Python), "packages/python");
    }

    #[test]
    fn package_dir_kotlin_ignores_source_output_override() {
        let r = resolved_one(
            r#"
[workspace]
languages = ["kotlin"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]

[crates.kotlin]
package = "dev.demo"
target = "jvm"

[crates.output]
kotlin = "packages/kotlin/src/main/kotlin/dev/demo/kt/"
"#,
        );
        assert_eq!(r.package_dir(Language::Kotlin), "packages/kotlin");
    }

    #[test]
    fn package_dir_node_crate_dir_override_takes_precedence() {
        let r = resolved_one(
            r#"
[workspace]
languages = ["node"]

[[crates]]
name = "sample-markdown-rs"
sources = ["src/lib.rs"]

[crates.node]
crate_dir = "crates/sample-markdown-node"
"#,
        );
        assert_eq!(
            r.package_dir(Language::Node),
            "crates/sample-markdown-node",
            "crate_dir override should be used instead of default formula"
        );
    }

    #[test]
    fn package_dir_wasm_crate_dir_override_takes_precedence() {
        let r = resolved_one(
            r#"
[workspace]
languages = ["wasm"]

[[crates]]
name = "sample-markdown-rs"
sources = ["src/lib.rs"]

[crates.wasm]
crate_dir = "crates/sample-markdown-wasm"
"#,
        );
        assert_eq!(
            r.package_dir(Language::Wasm),
            "crates/sample-markdown-wasm",
            "crate_dir override should be used instead of default formula"
        );
    }

    #[test]
    fn package_dir_node_without_override_uses_default_formula() {
        let r = resolved_one(
            r#"
[workspace]
languages = ["node"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
"#,
        );
        assert_eq!(
            r.package_dir(Language::Node),
            "crates/test-lib-node",
            "default formula should apply when crate_dir is not set"
        );
    }

    #[test]
    fn package_dir_wasm_without_override_uses_default_formula() {
        let r = resolved_one(
            r#"
[workspace]
languages = ["wasm"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
"#,
        );
        assert_eq!(
            r.package_dir(Language::Wasm),
            "crates/test-lib-wasm",
            "default formula should apply when crate_dir is not set"
        );
    }

    #[test]
    fn package_dir_sample_markdown_scenario_with_both_overrides() {
        let r = resolved_one(
            r#"
[workspace]
languages = ["node", "wasm"]

[[crates]]
name = "sample-markdown-rs"
sources = ["src/lib.rs"]

[crates.node]
crate_dir = "crates/sample-markdown-node"

[crates.wasm]
crate_dir = "crates/sample-markdown-wasm"
"#,
        );
        assert_eq!(
            r.package_dir(Language::Node),
            "crates/sample-markdown-node",
            "Node override should return exact path without formula"
        );
        assert_eq!(
            r.package_dir(Language::Wasm),
            "crates/sample-markdown-wasm",
            "Wasm override should return exact path without formula"
        );
    }
}
