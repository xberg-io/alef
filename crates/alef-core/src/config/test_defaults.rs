use super::extras::Language;
use super::output::{StringOrVec, TestConfig};

/// Return the default test configuration for a language.
///
/// The `output_dir` is the package directory where scaffolded files live
/// (e.g. `packages/python`). It is substituted into command templates.
pub(crate) fn default_test_config(lang: Language, output_dir: &str) -> TestConfig {
    match lang {
        Language::Python => TestConfig {
            command: Some(StringOrVec::Single(format!(
                "cd {output_dir} && uv run pytest"
            ))),
            e2e: None,
            coverage: Some(StringOrVec::Single(format!(
                "cd {output_dir} && uv run pytest --cov=. --cov-report=lcov"
            ))),
        },
        Language::Node | Language::Wasm => TestConfig {
            command: Some(StringOrVec::Single(format!("cd {output_dir} && pnpm test"))),
            e2e: None,
            coverage: Some(StringOrVec::Single(format!(
                "cd {output_dir} && pnpm test -- --coverage"
            ))),
        },
        Language::Go => TestConfig {
            command: Some(StringOrVec::Single(format!(
                "cd {output_dir} && go test ./..."
            ))),
            e2e: None,
            coverage: Some(StringOrVec::Single(format!(
                "cd {output_dir} && go test -coverprofile=coverage.out ./..."
            ))),
        },
        Language::Ruby => TestConfig {
            command: Some(StringOrVec::Single(format!(
                "cd {output_dir} && bundle exec rspec"
            ))),
            e2e: None,
            coverage: Some(StringOrVec::Single(format!(
                "cd {output_dir} && bundle exec rspec --format documentation"
            ))),
        },
        Language::Php => TestConfig {
            command: Some(StringOrVec::Single(format!(
                "cd {output_dir} && composer test"
            ))),
            e2e: None,
            coverage: Some(StringOrVec::Single(format!(
                "cd {output_dir} && composer test"
            ))),
        },
        Language::Java => TestConfig {
            command: Some(StringOrVec::Single(format!(
                "mvn -f {output_dir}/pom.xml test -q"
            ))),
            e2e: None,
            coverage: Some(StringOrVec::Single(format!(
                "mvn -f {output_dir}/pom.xml test jacoco:report -q"
            ))),
        },
        Language::Csharp => TestConfig {
            command: Some(StringOrVec::Single(format!("dotnet test {output_dir}"))),
            e2e: None,
            coverage: Some(StringOrVec::Single(format!(
                "dotnet test {output_dir} --collect:\"XPlat Code Coverage\""
            ))),
        },
        Language::Elixir => TestConfig {
            command: Some(StringOrVec::Single(format!("cd {output_dir} && mix test"))),
            e2e: None,
            coverage: Some(StringOrVec::Single(format!(
                "cd {output_dir} && mix test --cover"
            ))),
        },
        Language::R => TestConfig {
            command: Some(StringOrVec::Single(format!(
                "cd {output_dir} && Rscript -e \"testthat::test_dir('tests')\""
            ))),
            e2e: None,
            coverage: Some(StringOrVec::Single(format!(
                "cd {output_dir} && Rscript -e \"testthat::test_dir('tests')\""
            ))),
        },
        Language::Rust => TestConfig {
            command: Some(StringOrVec::Single("cargo test --workspace".to_string())),
            e2e: None,
            coverage: Some(StringOrVec::Single(
                "cargo llvm-cov --workspace --lcov --output-path coverage.lcov".to_string(),
            )),
        },
        Language::Ffi => TestConfig {
            command: None,
            e2e: None,
            coverage: None,
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
    fn ffi_has_no_test_commands() {
        let cfg = default_test_config(Language::Ffi, "packages/ffi");
        assert!(cfg.command.is_none());
        assert!(cfg.e2e.is_none());
        assert!(cfg.coverage.is_none());
    }

    #[test]
    fn non_ffi_languages_have_command_and_coverage() {
        for lang in all_languages() {
            if lang == Language::Ffi {
                continue;
            }
            let cfg = default_test_config(lang, "packages/test");
            assert!(
                cfg.command.is_some(),
                "{lang} should have a default test command"
            );
            assert!(
                cfg.coverage.is_some(),
                "{lang} should have a default coverage command"
            );
        }
    }

    #[test]
    fn e2e_is_always_none() {
        for lang in all_languages() {
            let cfg = default_test_config(lang, "packages/test");
            assert!(cfg.e2e.is_none(), "{lang} e2e should always be None (user-configured)");
        }
    }

    #[test]
    fn python_uses_pytest_and_uv() {
        let cfg = default_test_config(Language::Python, "packages/python");
        let cmd = cfg.command.unwrap().commands().join(" ");
        let cov = cfg.coverage.unwrap().commands().join(" ");
        assert!(cmd.contains("uv run pytest"));
        assert!(cov.contains("uv run pytest"));
        assert!(cov.contains("--cov=."));
        assert!(cov.contains("--cov-report=lcov"));
    }

    #[test]
    fn node_uses_pnpm() {
        let cfg = default_test_config(Language::Node, "packages/node");
        let cmd = cfg.command.unwrap().commands().join(" ");
        let cov = cfg.coverage.unwrap().commands().join(" ");
        assert!(cmd.contains("pnpm test"));
        assert!(cov.contains("pnpm test"));
        assert!(cov.contains("--coverage"));
    }

    #[test]
    fn wasm_matches_node() {
        let node = default_test_config(Language::Node, "packages/node");
        let wasm = default_test_config(Language::Wasm, "packages/wasm");
        let node_cmd = node.command.unwrap().commands().join(" ");
        let wasm_cmd = wasm.command.unwrap().commands().join(" ");
        assert!(node_cmd.contains("pnpm test"));
        assert!(wasm_cmd.contains("pnpm test"));
    }

    #[test]
    fn go_uses_go_test() {
        let cfg = default_test_config(Language::Go, "packages/go");
        let cmd = cfg.command.unwrap().commands().join(" ");
        let cov = cfg.coverage.unwrap().commands().join(" ");
        assert!(cmd.contains("go test ./..."));
        assert!(cov.contains("go test"));
        assert!(cov.contains("-coverprofile=coverage.out"));
    }

    #[test]
    fn ruby_uses_rspec() {
        let cfg = default_test_config(Language::Ruby, "packages/ruby");
        let cmd = cfg.command.unwrap().commands().join(" ");
        let cov = cfg.coverage.unwrap().commands().join(" ");
        assert!(cmd.contains("bundle exec rspec"));
        assert!(cov.contains("bundle exec rspec"));
        assert!(cov.contains("--format documentation"));
    }

    #[test]
    fn java_uses_maven() {
        let cfg = default_test_config(Language::Java, "packages/java");
        let cmd = cfg.command.unwrap().commands().join(" ");
        let cov = cfg.coverage.unwrap().commands().join(" ");
        assert!(cmd.contains("mvn"));
        assert!(cmd.contains("test"));
        assert!(cov.contains("jacoco:report"));
    }

    #[test]
    fn rust_uses_cargo_and_llvm_cov() {
        let cfg = default_test_config(Language::Rust, "packages/rust");
        let cmd = cfg.command.unwrap().commands().join(" ");
        let cov = cfg.coverage.unwrap().commands().join(" ");
        assert!(cmd.contains("cargo test --workspace"));
        assert!(cov.contains("cargo llvm-cov"));
        assert!(cov.contains("--lcov"));
    }

    #[test]
    fn output_dir_substituted_in_commands() {
        let cfg = default_test_config(Language::Python, "my/custom/dir");
        let cmd = cfg.command.unwrap().commands().join(" ");
        assert!(
            cmd.contains("my/custom/dir"),
            "Python test command should contain output dir, got: {cmd}"
        );
    }
}
