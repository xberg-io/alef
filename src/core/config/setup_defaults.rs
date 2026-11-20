mod commands;

use std::path::PathBuf;

use super::extras::Language;
use super::output::{SetupConfig, StringOrVec};
use super::tools::{LangContext, require_ruby_bundler, require_tool, ruby_bundle};

/// Return the default working directory (relative to repo root) for a language's
/// setup commands. Languages whose manifest file lives outside the repo root
/// (Swift's `Package.swift`, Kotlin-Android's `gradlew`, Dart's `pubspec.yaml`,
/// Zig's `build.zig`) need install commands run from their package directory or
/// the underlying tool will not find the manifest.
pub fn default_setup_workdir(lang: Language) -> Option<PathBuf> {
    match lang {
        Language::Swift => Some(PathBuf::from("packages/swift")),
        Language::KotlinAndroid => Some(PathBuf::from("packages/kotlin-android")),
        Language::Dart => Some(PathBuf::from("packages/dart")),
        Language::Zig => Some(PathBuf::from("packages/zig")),
        _ => None,
    }
}

/// Stand-alone factory matching the spec's `setup_config_for_language(lang)`
/// signature: returns a setup config with only the `workdir` field populated
/// (per-language default). Used by tests and callers that only need the workdir
/// default without resolving the full command pipeline (which requires
/// `output_dir` and `LangContext`).
pub fn setup_config_for_language(lang: Language) -> SetupConfig {
    SetupConfig {
        precondition: None,
        before: None,
        install: None,
        timeout_seconds: 1800,
        workdir: default_setup_workdir(lang),
    }
}

/// Return the default setup configuration for a language.
///
/// The `output_dir` is the package directory where scaffolded files live
/// (e.g. `packages/python`). It is substituted into command templates.
/// `ctx` provides the package manager selection.
pub(crate) fn default_setup_config(lang: Language, output_dir: &str, ctx: &LangContext) -> SetupConfig {
    let output_dir = super::shell::quote_word(output_dir);
    match lang {
        Language::Rust => commands::rust_config(lang, ctx),
        Language::Python => commands::python_config(lang, &output_dir, ctx),
        Language::Node | Language::Wasm => commands::node_wasm_config(lang, &output_dir, ctx),
        Language::Go => commands::go_config(lang, &output_dir),
        Language::Ruby => commands::ruby_config(lang, &output_dir),
        Language::Php => commands::php_config(lang, &output_dir),
        Language::Java => commands::java_config(lang, &output_dir),
        Language::Csharp => commands::csharp_config(lang, &output_dir),
        Language::Elixir => commands::elixir_config(lang, &output_dir),
        Language::R => commands::r_config(lang, &output_dir),
        Language::Ffi => commands::ffi_config(lang),
        Language::C => commands::c_config(lang),
        Language::Kotlin | Language::KotlinAndroid => commands::kotlin_kotlin_android_config(lang),
        Language::Swift => commands::swift_config(lang),
        Language::Dart => commands::dart_config(lang),
        Language::Zig => commands::zig_config(lang),
        Language::Gleam => commands::gleam_config(lang, &output_dir),
        Language::Jni => commands::jni_config(lang),
    }
}

#[cfg(test)]
mod tests {
    use super::super::tools::ToolsConfig;
    use super::*;

    fn all_languages() -> Vec<Language> {
        vec![
            Language::Python,
            Language::Node,
            Language::Wasm,
            Language::Ruby,
            Language::Php,
            Language::Go,
            Language::Java,
            Language::Csharp,
            Language::Elixir,
            Language::R,
            Language::Ffi,
            Language::Rust,
            Language::Kotlin,
            Language::Swift,
            Language::Dart,
            Language::Gleam,
            Language::Zig,
        ]
    }

    fn cfg(lang: Language, dir: &str) -> SetupConfig {
        let tools = ToolsConfig::default();
        let ctx = LangContext::default(&tools);
        default_setup_config(lang, dir, &ctx)
    }

    #[test]
    fn generated_setup_quotes_configured_output_directory() {
        let malicious = "packages/python; touch /tmp/alef-setup; #";
        let commands = cfg(Language::Python, malicious)
            .install
            .expect("python setup command")
            .commands()
            .join(" ");
        assert!(commands.contains(&format!("cd {}", super::super::shell::quote_word(malicious))));
    }

    #[test]
    fn php_install_runs_where_composer_json_is_and_never_prompts() {
        let command = cfg(Language::Php, "packages/php")
            .install
            .expect("php setup command")
            .commands()
            .join(" ");
        assert_eq!(
            command,
            "if [ -f packages/php/composer.json ]; then cd packages/php; fi && composer install --no-interaction"
        );
    }

    #[test]
    fn ffi_has_no_install_command() {
        let c = cfg(Language::Ffi, "packages/ffi");
        assert!(c.install.is_none());
    }

    #[test]
    fn non_ffi_languages_have_install_command() {
        for lang in all_languages() {
            if matches!(lang, Language::Ffi) {
                continue;
            }
            let c = cfg(lang, "packages/test");
            assert!(c.install.is_some(), "{lang} should have a default install command");
        }
    }

    #[test]
    fn non_ffi_languages_have_default_precondition() {
        for lang in all_languages() {
            if matches!(lang, Language::Ffi) {
                continue;
            }
            let c = cfg(lang, "packages/test");
            let pre = c
                .precondition
                .unwrap_or_else(|| panic!("{lang} should have a precondition"));
            assert!(pre.starts_with("command -v "));
        }
    }

    #[test]
    fn rust_install_lists_full_tool_set() {
        let c = cfg(Language::Rust, "packages/rust");
        let install = c.install.unwrap();
        let cmds = install.commands();
        let joined = cmds.join(" || ");
        assert!(joined.contains("rustup update stable"));
        for tool in super::super::tools::DEFAULT_RUST_DEV_TOOLS {
            assert!(
                joined.contains(&format!("cargo install {tool} --locked")),
                "Rust setup should install {tool}, got: {joined}"
            );
        }
        assert!(joined.contains("rustup component add rustfmt clippy"));
    }

    #[test]
    fn rust_install_respects_user_tool_list() {
        let tools = ToolsConfig {
            rust_dev_tools: Some(vec!["cargo-edit".to_string(), "cargo-foo".to_string()]),
            ..Default::default()
        };
        let ctx = LangContext::default(&tools);
        let c = default_setup_config(Language::Rust, "packages/rust", &ctx);
        let cmds = c.install.unwrap().commands().join(" || ");
        assert!(cmds.contains("cargo install cargo-edit --locked"));
        assert!(cmds.contains("cargo install cargo-foo --locked"));
        assert!(!cmds.contains("cargo install cargo-deny"));
    }

    fn python_tools(pm: &str) -> ToolsConfig {
        ToolsConfig {
            python_package_manager: Some(pm.to_string()),
            ..Default::default()
        }
    }

    fn node_tools(pm: &str) -> ToolsConfig {
        ToolsConfig {
            node_package_manager: Some(pm.to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn python_setup_dispatches_on_package_manager() {
        for (pm, expected_install, expected_pre) in [
            (
                "uv",
                "uv sync --no-install-project --no-install-workspace",
                "command -v uv >/dev/null 2>&1",
            ),
            ("pip", "pip install -e", "command -v pip >/dev/null 2>&1"),
            ("poetry", "poetry install", "command -v poetry >/dev/null 2>&1"),
        ] {
            let tools = python_tools(pm);
            let ctx = LangContext::default(&tools);
            let c = default_setup_config(Language::Python, "packages/python", &ctx);
            assert!(c.install.unwrap().commands().join(" ").contains(expected_install));
            assert_eq!(c.precondition.as_deref(), Some(expected_pre));
        }
    }

    #[test]
    fn node_setup_dispatches_on_package_manager() {
        for (pm, expected_install) in [
            ("pnpm", "pnpm install"),
            ("npm", "npm install"),
            ("yarn", "yarn install"),
        ] {
            let tools = node_tools(pm);
            let ctx = LangContext::default(&tools);
            let c = default_setup_config(Language::Node, "packages/node", &ctx);
            assert!(c.install.unwrap().commands().join(" ").contains(expected_install));
        }
    }

    #[test]
    fn python_uses_uv_sync_by_default() {
        let c = cfg(Language::Python, "packages/python");
        let install = c.install.unwrap().commands().join(" ");
        assert!(install.contains("uv sync"));
        assert!(install.contains("packages/python"));
    }

    #[test]
    fn node_uses_pnpm_install_by_default() {
        let c = cfg(Language::Node, "packages/node");
        let install = c.install.unwrap().commands().join(" ");
        assert!(install.contains("pnpm install"));
    }

    #[test]
    fn wasm_matches_node() {
        let node = cfg(Language::Node, "packages/foo");
        let wasm = cfg(Language::Wasm, "packages/foo");
        assert_eq!(
            node.install.unwrap().commands().join(" "),
            wasm.install.unwrap().commands().join(" "),
            "WASM and Node should share install command"
        );
    }

    #[test]
    fn go_uses_go_mod_download() {
        let c = cfg(Language::Go, "packages/go");
        let install = c.install.unwrap().commands().join(" ");
        assert!(install.contains("go mod download"));
    }

    #[test]
    fn ruby_uses_bundle_install() {
        let c = cfg(Language::Ruby, "packages/ruby");
        assert_eq!(
            c.precondition.as_deref(),
            Some(
                "command -v ruby >/dev/null 2>&1 && BUNDLE_PATH=vendor/bundle ruby -S bundle --version >/dev/null 2>&1"
            )
        );
        let install = c.install.unwrap().commands().join(" ");
        assert!(
            install.contains("BUNDLE_PATH=vendor/bundle ruby -S bundle install"),
            "got: {install}"
        );
        assert!(
            install.contains("BUNDLE_PATH=vendor/bundle ruby -S bundle lock --add-checksums"),
            "got: {install}"
        );
    }

    #[test]
    fn java_uses_maven_dependency_resolve() {
        let c = cfg(Language::Java, "packages/java");
        let install = c.install.unwrap().commands().join(" ");
        assert!(install.contains("mvn"));
        assert!(install.contains("dependency:resolve"));
    }

    #[test]
    fn csharp_uses_dotnet_restore() {
        let c = cfg(Language::Csharp, "packages/csharp");
        let install = c.install.unwrap().commands().join(" ");
        assert!(install.contains("dotnet restore"));
    }

    #[test]
    fn elixir_uses_mix_deps_get() {
        let c = cfg(Language::Elixir, "packages/elixir");
        let install = c.install.unwrap().commands().join(" ");
        assert!(install.contains("mix deps.get"));
    }

    #[test]
    fn r_uses_remotes_install_deps() {
        let c = cfg(Language::R, "packages/r");
        let install = c.install.unwrap().commands().join(" ");
        assert!(install.contains("remotes::install_deps()"));
    }

    #[test]
    fn gleam_uses_gleam_deps_download() {
        let c = cfg(Language::Gleam, "packages/gleam");
        let install = c.install.unwrap().commands().join(" ");
        assert!(
            install.contains("gleam deps download"),
            "Gleam setup should use gleam deps download, got: {install}"
        );
        assert_eq!(c.precondition.as_deref(), Some("command -v gleam >/dev/null 2>&1"));
    }

    #[test]
    fn output_dir_substituted_in_commands() {
        let c = cfg(Language::Go, "my/custom/path");
        let install = c.install.unwrap().commands().join(" ");
        assert!(install.contains("my/custom/path"));
    }

    /// 0.82.0 removed `[setup.<lang>].timeout_seconds` as an `alef.toml` escape hatch, so a
    /// language whose `bundle install`/equivalent genuinely needs longer than the 1800s every
    /// other language gets must have that need encoded in the built-in default itself -- a real
    /// consumer repo's Ruby setup timed out at 1800s once the override was the only way to raise
    /// it. Only Ruby gets a raised ceiling: every other language's install step has no comparable
    /// native-extension-compilation cost, so 1800s stays the default for all of them. ~keep
    #[test]
    fn ruby_gets_a_longer_timeout_than_every_other_language_for_native_extension_builds() {
        let ruby = cfg(Language::Ruby, "packages/ruby");
        assert_eq!(
            ruby.timeout_seconds, 3000,
            "ruby's bundle install must get the raised ceiling"
        );
        for lang in all_languages() {
            if lang == Language::Ruby {
                continue;
            }
            let c = cfg(lang, "packages/test");
            assert_eq!(
                c.timeout_seconds, 1800,
                "{lang} has no known reason to need more than the default 1800s ceiling"
            );
        }
    }
}
