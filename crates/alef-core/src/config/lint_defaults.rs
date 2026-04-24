use super::extras::Language;
use super::output::{LintConfig, StringOrVec};

/// Return the default lint configuration for a language.
///
/// The `output_dir` is the package directory where scaffolded files live
/// (e.g. `packages/python`). It is substituted into command templates.
pub fn default_lint_config(lang: Language, output_dir: &str) -> LintConfig {
    match lang {
        Language::Python => LintConfig {
            format: Some(StringOrVec::Single(format!("ruff format {output_dir}"))),
            check: Some(StringOrVec::Single(format!("ruff check --fix {output_dir}"))),
            typecheck: Some(StringOrVec::Single(format!("mypy {output_dir}"))),
        },
        Language::Node | Language::Wasm => LintConfig {
            format: Some(StringOrVec::Single(format!("npx oxfmt {output_dir}"))),
            check: Some(StringOrVec::Single(format!(
                "npx oxlint --fix {output_dir}"
            ))),
            typecheck: None,
        },
        Language::Ruby => LintConfig {
            format: Some(StringOrVec::Single(format!("bundle exec rubocop -A {output_dir}"))),
            check: Some(StringOrVec::Single(format!("bundle exec rubocop {output_dir}"))),
            typecheck: None,
        },
        Language::Php => LintConfig {
            format: Some(StringOrVec::Single(format!("cd {output_dir} && composer run format"))),
            check: Some(StringOrVec::Single(format!("cd {output_dir} && composer run lint"))),
            typecheck: None,
        },
        Language::Go => LintConfig {
            format: Some(StringOrVec::Single(format!("gofmt -w {output_dir}"))),
            check: Some(StringOrVec::Single(format!(
                "cd {output_dir} && golangci-lint run ./..."
            ))),
            typecheck: None,
        },
        Language::Java => LintConfig {
            format: Some(StringOrVec::Single(format!(
                "mvn -f {output_dir}/pom.xml spotless:apply -q"
            ))),
            check: Some(StringOrVec::Single(format!(
                "mvn -f {output_dir}/pom.xml spotless:check checkstyle:check -q"
            ))),
            typecheck: None,
        },
        Language::Csharp => LintConfig {
            format: Some(StringOrVec::Single(format!("dotnet format {output_dir}"))),
            check: Some(StringOrVec::Single(format!(
                "dotnet format {output_dir} --verify-no-changes"
            ))),
            typecheck: None,
        },
        Language::Elixir => LintConfig {
            format: Some(StringOrVec::Single(format!("cd {output_dir} && mix format"))),
            check: Some(StringOrVec::Single(format!("cd {output_dir} && mix credo --strict"))),
            typecheck: None,
        },
        Language::R => LintConfig {
            format: Some(StringOrVec::Single(format!(
                "cd {output_dir} && Rscript -e \"styler::style_pkg()\""
            ))),
            check: Some(StringOrVec::Single(format!(
                "cd {output_dir} && Rscript -e \"lintr::lint_package()\""
            ))),
            typecheck: None,
        },
        Language::Ffi => LintConfig {
            format: Some(StringOrVec::Single(format!(
                "find {output_dir}/tests -name '*.c' -o -name '*.h' | xargs clang-format -i"
            ))),
            check: Some(StringOrVec::Single(format!("cppcheck {output_dir}/tests/"))),
            typecheck: None,
        },
        Language::Rust => LintConfig {
            format: Some(StringOrVec::Single("cargo fmt".to_string())),
            check: Some(StringOrVec::Single(
                "cargo clippy --fix --allow-dirty --allow-staged -- -D warnings".to_string(),
            )),
            typecheck: None,
        },
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
    fn every_language_has_format_default() {
        for lang in all_languages() {
            let cfg = default_lint_config(lang, "packages/test");
            assert!(
                cfg.format.is_some(),
                "{lang} should have a default format command"
            );
        }
    }

    #[test]
    fn every_language_has_check_default() {
        for lang in all_languages() {
            let cfg = default_lint_config(lang, "packages/test");
            assert!(
                cfg.check.is_some(),
                "{lang} should have a default check command"
            );
        }
    }

    #[test]
    fn python_defaults_use_ruff_and_mypy() {
        let cfg = default_lint_config(Language::Python, "packages/python");
        let fmt = cfg.format.unwrap().commands().join(" ");
        let check = cfg.check.unwrap().commands().join(" ");
        let tc = cfg.typecheck.unwrap().commands().join(" ");
        assert!(fmt.contains("ruff format"));
        assert!(check.contains("ruff check"));
        assert!(tc.contains("mypy"));
    }

    #[test]
    fn node_defaults_use_oxc() {
        let cfg = default_lint_config(Language::Node, "packages/node");
        let fmt = cfg.format.unwrap().commands().join(" ");
        let check = cfg.check.unwrap().commands().join(" ");
        assert!(fmt.contains("oxfmt"), "Node format should use oxfmt, got: {fmt}");
        assert!(
            check.contains("oxlint"),
            "Node check should use oxlint, got: {check}"
        );
        assert!(!fmt.contains("biome"), "Node should not reference biome");
    }

    #[test]
    fn wasm_defaults_match_node() {
        let node = default_lint_config(Language::Node, "packages/node");
        let wasm = default_lint_config(Language::Wasm, "packages/wasm");
        let node_fmt = node.format.unwrap().commands().join(" ");
        let wasm_fmt = wasm.format.unwrap().commands().join(" ");
        // Same tool, different dir
        assert!(node_fmt.contains("oxfmt"));
        assert!(wasm_fmt.contains("oxfmt"));
    }

    #[test]
    fn java_defaults_use_spotless() {
        let cfg = default_lint_config(Language::Java, "packages/java");
        let fmt = cfg.format.unwrap().commands().join(" ");
        let check = cfg.check.unwrap().commands().join(" ");
        assert!(fmt.contains("spotless:apply"));
        assert!(check.contains("spotless:check"));
        assert!(check.contains("checkstyle:check"));
    }

    #[test]
    fn rust_defaults_use_cargo() {
        let cfg = default_lint_config(Language::Rust, "packages/rust");
        let fmt = cfg.format.unwrap().commands().join(" ");
        let check = cfg.check.unwrap().commands().join(" ");
        assert!(fmt.contains("cargo fmt"));
        assert!(check.contains("cargo clippy"));
    }

    #[test]
    fn output_dir_substituted_in_commands() {
        let cfg = default_lint_config(Language::Go, "my/custom/dir");
        let fmt = cfg.format.unwrap().commands().join(" ");
        let check = cfg.check.unwrap().commands().join(" ");
        assert!(
            fmt.contains("my/custom/dir"),
            "Go format should contain output dir, got: {fmt}"
        );
        assert!(
            check.contains("my/custom/dir"),
            "Go check should contain output dir, got: {check}"
        );
    }

    #[test]
    fn only_python_has_typecheck_default() {
        for lang in all_languages() {
            let cfg = default_lint_config(lang, "packages/test");
            if lang == Language::Python {
                assert!(cfg.typecheck.is_some(), "Python should have typecheck");
            } else {
                assert!(
                    cfg.typecheck.is_none(),
                    "{lang} should not have typecheck default"
                );
            }
        }
    }
}
