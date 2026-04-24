use super::extras::Language;
use super::output::{CleanConfig, StringOrVec};

/// Return the default clean configuration for a language.
///
/// The `output_dir` is the package directory where scaffolded files live
/// (e.g. `packages/python`). It is substituted into command templates.
pub(crate) fn default_clean_config(lang: Language, output_dir: &str) -> CleanConfig {
    match lang {
        Language::Rust => CleanConfig {
            clean: Some(StringOrVec::Single("cargo clean".to_string())),
        },
        Language::Python => CleanConfig {
            clean: Some(StringOrVec::Single(format!(
                "cd {output_dir} && rm -rf __pycache__ .pytest_cache .mypy_cache .ruff_cache dist"
            ))),
        },
        Language::Node | Language::Wasm => CleanConfig {
            clean: Some(StringOrVec::Single(
                "rm -rf node_modules dist .turbo".to_string(),
            )),
        },
        Language::Go => CleanConfig {
            clean: Some(StringOrVec::Single(format!(
                "cd {output_dir} && go clean -cache"
            ))),
        },
        Language::Ruby => CleanConfig {
            clean: Some(StringOrVec::Single(format!(
                "cd {output_dir} && rm -rf tmp vendor .bundle"
            ))),
        },
        Language::Php => CleanConfig {
            clean: Some(StringOrVec::Single(format!(
                "cd {output_dir} && rm -rf vendor var"
            ))),
        },
        Language::Java => CleanConfig {
            clean: Some(StringOrVec::Single(format!(
                "mvn -f {output_dir}/pom.xml clean -q"
            ))),
        },
        Language::Csharp => CleanConfig {
            clean: Some(StringOrVec::Single(format!("dotnet clean {output_dir}"))),
        },
        Language::Elixir => CleanConfig {
            clean: Some(StringOrVec::Single(format!(
                "cd {output_dir} && mix clean && rm -rf deps _build"
            ))),
        },
        Language::R => CleanConfig {
            clean: Some(StringOrVec::Single(format!(
                "cd {output_dir} && rm -rf src/rust/target"
            ))),
        },
        Language::Ffi => CleanConfig { clean: None },
    }
}

#[cfg(test)]
mod tests {
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
        ]
    }

    #[test]
    fn ffi_has_no_clean_command() {
        let cfg = default_clean_config(Language::Ffi, "packages/ffi");
        assert!(cfg.clean.is_none());
    }

    #[test]
    fn non_ffi_languages_have_clean_command() {
        for lang in all_languages() {
            if lang == Language::Ffi {
                continue;
            }
            let cfg = default_clean_config(lang, "packages/test");
            assert!(
                cfg.clean.is_some(),
                "{lang} should have a default clean command"
            );
        }
    }

    #[test]
    fn rust_uses_cargo_clean() {
        let cfg = default_clean_config(Language::Rust, "packages/rust");
        let clean = cfg.clean.unwrap().commands().join(" ");
        assert!(clean.contains("cargo clean"));
    }

    #[test]
    fn python_removes_pycache_and_dist() {
        let cfg = default_clean_config(Language::Python, "packages/python");
        let clean = cfg.clean.unwrap().commands().join(" ");
        assert!(clean.contains("__pycache__"));
        assert!(clean.contains(".pytest_cache"));
        assert!(clean.contains("dist"));
    }

    #[test]
    fn node_removes_node_modules() {
        let cfg = default_clean_config(Language::Node, "packages/node");
        let clean = cfg.clean.unwrap().commands().join(" ");
        assert!(clean.contains("node_modules"));
        assert!(clean.contains("dist"));
    }

    #[test]
    fn wasm_matches_node() {
        let node = default_clean_config(Language::Node, "packages/node");
        let wasm = default_clean_config(Language::Wasm, "packages/wasm");
        let node_clean = node.clean.unwrap().commands().join(" ");
        let wasm_clean = wasm.clean.unwrap().commands().join(" ");
        assert_eq!(node_clean, wasm_clean, "WASM and Node should share clean command");
    }

    #[test]
    fn go_uses_go_clean() {
        let cfg = default_clean_config(Language::Go, "packages/go");
        let clean = cfg.clean.unwrap().commands().join(" ");
        assert!(clean.contains("go clean -cache"));
    }

    #[test]
    fn java_uses_maven_clean() {
        let cfg = default_clean_config(Language::Java, "packages/java");
        let clean = cfg.clean.unwrap().commands().join(" ");
        assert!(clean.contains("mvn"));
        assert!(clean.contains("clean"));
    }

    #[test]
    fn csharp_uses_dotnet_clean() {
        let cfg = default_clean_config(Language::Csharp, "packages/csharp");
        let clean = cfg.clean.unwrap().commands().join(" ");
        assert!(clean.contains("dotnet clean"));
    }

    #[test]
    fn elixir_uses_mix_clean() {
        let cfg = default_clean_config(Language::Elixir, "packages/elixir");
        let clean = cfg.clean.unwrap().commands().join(" ");
        assert!(clean.contains("mix clean"));
        assert!(clean.contains("deps"));
        assert!(clean.contains("_build"));
    }

    #[test]
    fn r_removes_rust_target() {
        let cfg = default_clean_config(Language::R, "packages/r");
        let clean = cfg.clean.unwrap().commands().join(" ");
        assert!(clean.contains("src/rust/target"));
    }

    #[test]
    fn output_dir_substituted_in_commands() {
        let cfg = default_clean_config(Language::Go, "my/custom/path");
        let clean = cfg.clean.unwrap().commands().join(" ");
        assert!(
            clean.contains("my/custom/path"),
            "Go clean should contain output dir, got: {clean}"
        );
    }
}
