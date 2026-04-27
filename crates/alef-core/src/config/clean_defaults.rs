use super::extras::Language;
use super::output::{CleanConfig, StringOrVec};
use super::tools::{LangContext, require_tool};

/// Return the default clean configuration for a language.
///
/// The `output_dir` is the package directory where scaffolded files live
/// (e.g. `packages/python`). It is substituted into command templates.
/// `ctx` is provided but not used; clean commands don't depend on the
/// chosen package manager.
///
/// Languages whose clean command relies only on POSIX shell builtins
/// (e.g. plain `rm -rf`) leave `precondition` as `None` since `rm` is
/// effectively always present on supported platforms.
pub(crate) fn default_clean_config(lang: Language, output_dir: &str, _ctx: &LangContext) -> CleanConfig {
    match lang {
        Language::Rust => CleanConfig {
            precondition: Some(require_tool("cargo")),
            before: None,
            clean: Some(StringOrVec::Single("cargo clean".to_string())),
        },
        Language::Python => CleanConfig {
            // Pure shell `rm` — no toolchain dependency.
            precondition: None,
            before: None,
            clean: Some(StringOrVec::Single(format!(
                "cd {output_dir} && rm -rf __pycache__ .pytest_cache .mypy_cache .ruff_cache dist"
            ))),
        },
        Language::Node | Language::Wasm => CleanConfig {
            // Pure shell `rm`.
            precondition: None,
            before: None,
            clean: Some(StringOrVec::Single("rm -rf node_modules dist .turbo".to_string())),
        },
        Language::Go => CleanConfig {
            precondition: Some(require_tool("go")),
            before: None,
            clean: Some(StringOrVec::Single(format!("cd {output_dir} && go clean -cache"))),
        },
        Language::Ruby => CleanConfig {
            // Pure shell `rm`.
            precondition: None,
            before: None,
            clean: Some(StringOrVec::Single(format!(
                "cd {output_dir} && rm -rf tmp vendor .bundle"
            ))),
        },
        Language::Php => CleanConfig {
            // Pure shell `rm`.
            precondition: None,
            before: None,
            clean: Some(StringOrVec::Single(format!("cd {output_dir} && rm -rf vendor var"))),
        },
        Language::Java => CleanConfig {
            precondition: Some(require_tool("mvn")),
            before: None,
            clean: Some(StringOrVec::Single(format!("mvn -f {output_dir}/pom.xml clean -q"))),
        },
        Language::Csharp => CleanConfig {
            precondition: Some(require_tool("dotnet")),
            before: None,
            clean: Some(StringOrVec::Single(format!("dotnet clean {output_dir}"))),
        },
        Language::Elixir => CleanConfig {
            precondition: Some(require_tool("mix")),
            before: None,
            clean: Some(StringOrVec::Single(format!(
                "cd {output_dir} && mix clean && rm -rf deps _build"
            ))),
        },
        Language::R => CleanConfig {
            // Pure shell `rm`.
            precondition: None,
            before: None,
            clean: Some(StringOrVec::Single(format!(
                "cd {output_dir} && rm -rf src/rust/target"
            ))),
        },
        Language::Ffi => CleanConfig {
            precondition: None,
            before: None,
            clean: None,
        },
        Language::Kotlin => CleanConfig {
            precondition: Some(require_tool("gradle")),
            before: None,
            clean: Some(StringOrVec::Single("gradle clean".to_string())),
        },
        Language::Swift => CleanConfig {
            // Pure swift toolchain command.
            precondition: Some(require_tool("swift")),
            before: None,
            clean: Some(StringOrVec::Single("swift package clean".to_string())),
        },
        Language::Dart => CleanConfig {
            precondition: Some(require_tool("dart")),
            before: None,
            clean: Some(StringOrVec::Single("dart clean".to_string())),
        },
        Language::Gleam => CleanConfig {
            // Gleam has no `gleam clean`; it uses a `build/` directory.
            precondition: None,
            before: None,
            clean: Some(StringOrVec::Single("rm -rf build".to_string())),
        },
        Language::Zig => CleanConfig {
            // Pure shell `rm` — zig-out and cache dirs.
            precondition: None,
            before: None,
            clean: Some(StringOrVec::Single(
                "rm -rf zig-out zig-cache .zig-cache".to_string(),
            )),
        },
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

    fn cfg(lang: Language, dir: &str) -> CleanConfig {
        let tools = ToolsConfig::default();
        let ctx = LangContext::default(&tools);
        default_clean_config(lang, dir, &ctx)
    }

    #[test]
    fn ffi_has_no_clean_command() {
        let c = cfg(Language::Ffi, "packages/ffi");
        assert!(c.clean.is_none());
    }

    #[test]
    fn non_ffi_languages_have_clean_command() {
        for lang in all_languages() {
            if matches!(lang, Language::Ffi) {
                continue;
            }
            let c = cfg(lang, "packages/test");
            assert!(c.clean.is_some(), "{lang} should have a default clean command");
        }
    }

    #[test]
    fn toolchain_clean_has_precondition() {
        // Languages whose clean uses a toolchain (cargo/go/mvn/dotnet/mix) get a precondition.
        for lang in [
            Language::Rust,
            Language::Go,
            Language::Java,
            Language::Csharp,
            Language::Elixir,
        ] {
            let c = cfg(lang, "packages/test");
            let pre = c
                .precondition
                .unwrap_or_else(|| panic!("{lang} should have a precondition"));
            assert!(pre.starts_with("command -v "));
        }
    }

    #[test]
    fn pure_shell_clean_omits_precondition() {
        // Languages whose clean is just `rm -rf` don't need a precondition.
        for lang in [
            Language::Python,
            Language::Node,
            Language::Wasm,
            Language::Ruby,
            Language::Php,
            Language::R,
        ] {
            let c = cfg(lang, "packages/test");
            assert!(
                c.precondition.is_none(),
                "{lang} pure-shell clean should not have a precondition"
            );
        }
    }

    #[test]
    fn rust_uses_cargo_clean() {
        let c = cfg(Language::Rust, "packages/rust");
        let clean = c.clean.unwrap().commands().join(" ");
        assert!(clean.contains("cargo clean"));
    }

    #[test]
    fn python_removes_pycache_and_dist() {
        let c = cfg(Language::Python, "packages/python");
        let clean = c.clean.unwrap().commands().join(" ");
        assert!(clean.contains("__pycache__"));
        assert!(clean.contains(".pytest_cache"));
        assert!(clean.contains("dist"));
    }

    #[test]
    fn node_removes_node_modules() {
        let c = cfg(Language::Node, "packages/node");
        let clean = c.clean.unwrap().commands().join(" ");
        assert!(clean.contains("node_modules"));
        assert!(clean.contains("dist"));
    }

    #[test]
    fn wasm_matches_node() {
        let node = cfg(Language::Node, "packages/node");
        let wasm = cfg(Language::Wasm, "packages/wasm");
        assert_eq!(
            node.clean.unwrap().commands().join(" "),
            wasm.clean.unwrap().commands().join(" "),
        );
    }

    #[test]
    fn go_uses_go_clean() {
        let c = cfg(Language::Go, "packages/go");
        let clean = c.clean.unwrap().commands().join(" ");
        assert!(clean.contains("go clean -cache"));
    }

    #[test]
    fn java_uses_maven_clean() {
        let c = cfg(Language::Java, "packages/java");
        let clean = c.clean.unwrap().commands().join(" ");
        assert!(clean.contains("mvn"));
        assert!(clean.contains("clean"));
    }

    #[test]
    fn csharp_uses_dotnet_clean() {
        let c = cfg(Language::Csharp, "packages/csharp");
        let clean = c.clean.unwrap().commands().join(" ");
        assert!(clean.contains("dotnet clean"));
    }

    #[test]
    fn elixir_uses_mix_clean() {
        let c = cfg(Language::Elixir, "packages/elixir");
        let clean = c.clean.unwrap().commands().join(" ");
        assert!(clean.contains("mix clean"));
        assert!(clean.contains("deps"));
        assert!(clean.contains("_build"));
    }

    #[test]
    fn r_removes_rust_target() {
        let c = cfg(Language::R, "packages/r");
        let clean = c.clean.unwrap().commands().join(" ");
        assert!(clean.contains("src/rust/target"));
    }

    #[test]
    fn output_dir_substituted_in_commands() {
        let c = cfg(Language::Go, "my/custom/path");
        let clean = c.clean.unwrap().commands().join(" ");
        assert!(clean.contains("my/custom/path"));
    }
}
