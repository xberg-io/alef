use super::extras::Language;
use super::output::{StringOrVec, TestConfig};
use super::tools::{LangContext, require_tool, wrap_command as wrap};

/// Return the default test configuration for a language.
///
/// The `output_dir` is the package directory where scaffolded files live
/// (e.g. `packages/python`). It is substituted into command templates.
/// `ctx` provides tool selection and run_wrapper.
pub(crate) fn default_test_config(lang: Language, output_dir: &str, ctx: &LangContext) -> TestConfig {
    match lang {
        Language::Python => {
            let pm = ctx.tools.python_pm();
            // pytest is invoked via the package manager when one is present;
            // for `pip` we just call pytest directly.
            let (cmd, cov, pre_tool) = match pm {
                "pip" => (
                    wrap(format!("cd {output_dir} && pytest"), ctx.run_wrapper),
                    wrap(
                        format!("cd {output_dir} && pytest --cov=. --cov-report=lcov"),
                        ctx.run_wrapper,
                    ),
                    "pytest",
                ),
                "poetry" => (
                    wrap(format!("cd {output_dir} && poetry run pytest"), ctx.run_wrapper),
                    wrap(
                        format!("cd {output_dir} && poetry run pytest --cov=. --cov-report=lcov"),
                        ctx.run_wrapper,
                    ),
                    "poetry",
                ),
                _ => (
                    wrap(format!("cd {output_dir} && uv run pytest"), ctx.run_wrapper),
                    wrap(
                        format!("cd {output_dir} && uv run pytest --cov=. --cov-report=lcov"),
                        ctx.run_wrapper,
                    ),
                    "uv",
                ),
            };
            TestConfig {
                precondition: Some(require_tool(pre_tool)),
                before: None,
                command: Some(StringOrVec::Single(cmd)),
                e2e: None,
                coverage: Some(StringOrVec::Single(cov)),
            }
        }
        Language::Node | Language::Wasm => {
            let pm = ctx.tools.node_pm();
            let (cmd, cov) = match pm {
                "npm" => (
                    wrap(format!("cd {output_dir} && npm test"), ctx.run_wrapper),
                    wrap(format!("cd {output_dir} && npm test -- --coverage"), ctx.run_wrapper),
                ),
                "yarn" => (
                    wrap(format!("cd {output_dir} && yarn test"), ctx.run_wrapper),
                    wrap(format!("cd {output_dir} && yarn test --coverage"), ctx.run_wrapper),
                ),
                _ => (
                    wrap(format!("cd {output_dir} && pnpm test"), ctx.run_wrapper),
                    wrap(format!("cd {output_dir} && pnpm test -- --coverage"), ctx.run_wrapper),
                ),
            };
            TestConfig {
                precondition: Some(require_tool(pm)),
                before: None,
                command: Some(StringOrVec::Single(cmd)),
                e2e: None,
                coverage: Some(StringOrVec::Single(cov)),
            }
        }
        Language::Go => {
            let cmd = wrap(format!("cd {output_dir} && go test ./..."), ctx.run_wrapper);
            let cov = wrap(
                format!("cd {output_dir} && go test -coverprofile=coverage.out ./..."),
                ctx.run_wrapper,
            );
            TestConfig {
                precondition: Some(require_tool("go")),
                before: None,
                command: Some(StringOrVec::Single(cmd)),
                e2e: None,
                coverage: Some(StringOrVec::Single(cov)),
            }
        }
        Language::Ruby => {
            let cmd = wrap(format!("cd {output_dir} && bundle exec rspec"), ctx.run_wrapper);
            let cov = wrap(
                format!("cd {output_dir} && bundle exec rspec --format documentation"),
                ctx.run_wrapper,
            );
            TestConfig {
                precondition: Some(require_tool("bundle")),
                before: None,
                command: Some(StringOrVec::Single(cmd)),
                e2e: None,
                coverage: Some(StringOrVec::Single(cov)),
            }
        }
        Language::Php => {
            let cmd = wrap(format!("cd {output_dir} && composer test"), ctx.run_wrapper);
            let cov = wrap(format!("cd {output_dir} && composer test"), ctx.run_wrapper);
            TestConfig {
                precondition: Some(require_tool("composer")),
                before: None,
                command: Some(StringOrVec::Single(cmd)),
                e2e: None,
                coverage: Some(StringOrVec::Single(cov)),
            }
        }
        Language::Java => {
            let (cmd_path, cov_path) = if let Some(proj) = ctx.project_file {
                (
                    format!("mvn -f {proj} test -q"),
                    format!("mvn -f {proj} test jacoco:report -q"),
                )
            } else {
                (
                    format!("mvn -f {output_dir}/pom.xml test -q"),
                    format!("mvn -f {output_dir}/pom.xml test jacoco:report -q"),
                )
            };
            TestConfig {
                precondition: Some(require_tool("mvn")),
                before: None,
                command: Some(StringOrVec::Single(wrap(cmd_path, ctx.run_wrapper))),
                e2e: None,
                coverage: Some(StringOrVec::Single(wrap(cov_path, ctx.run_wrapper))),
            }
        }
        Language::Csharp => {
            let (cmd_path, cov_path) = if let Some(proj) = ctx.project_file {
                (
                    format!("dotnet test {proj}"),
                    format!("dotnet test {proj} --collect:\"XPlat Code Coverage\""),
                )
            } else {
                (
                    format!("dotnet test {output_dir}"),
                    format!("dotnet test {output_dir} --collect:\"XPlat Code Coverage\""),
                )
            };
            TestConfig {
                precondition: Some(require_tool("dotnet")),
                before: None,
                command: Some(StringOrVec::Single(wrap(cmd_path, ctx.run_wrapper))),
                e2e: None,
                coverage: Some(StringOrVec::Single(wrap(cov_path, ctx.run_wrapper))),
            }
        }
        Language::Elixir => {
            let cmd = wrap(format!("cd {output_dir} && mix test"), ctx.run_wrapper);
            let cov = wrap(format!("cd {output_dir} && mix test --cover"), ctx.run_wrapper);
            TestConfig {
                precondition: Some(require_tool("mix")),
                before: None,
                command: Some(StringOrVec::Single(cmd)),
                e2e: None,
                coverage: Some(StringOrVec::Single(cov)),
            }
        }
        Language::R => {
            let cmd = wrap(
                format!("cd {output_dir} && Rscript -e \"testthat::test_dir('tests')\""),
                ctx.run_wrapper,
            );
            let cov = wrap(
                format!("cd {output_dir} && Rscript -e \"testthat::test_dir('tests')\""),
                ctx.run_wrapper,
            );
            TestConfig {
                precondition: Some(require_tool("Rscript")),
                before: None,
                command: Some(StringOrVec::Single(cmd)),
                e2e: None,
                coverage: Some(StringOrVec::Single(cov)),
            }
        }
        Language::Rust => TestConfig {
            precondition: Some(require_tool("cargo")),
            before: None,
            command: Some(StringOrVec::Single("cargo test --workspace".to_string())),
            e2e: None,
            coverage: Some(StringOrVec::Single(
                "cargo llvm-cov --workspace --lcov --output-path coverage.lcov".to_string(),
            )),
        },
        Language::Ffi => TestConfig {
            precondition: None,
            before: None,
            command: None,
            e2e: None,
            coverage: None,
        },
        Language::Kotlin | Language::Swift | Language::Dart | Language::Gleam | Language::Zig => TestConfig {
            precondition: None,
            before: None,
            command: None,
            e2e: None,
            coverage: None,
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

    fn cfg(lang: Language, dir: &str) -> TestConfig {
        let tools = ToolsConfig::default();
        let ctx = LangContext::default(&tools);
        default_test_config(lang, dir, &ctx)
    }

    #[test]
    fn ffi_has_no_test_commands() {
        let c = cfg(Language::Ffi, "packages/ffi");
        assert!(c.command.is_none());
        assert!(c.e2e.is_none());
        assert!(c.coverage.is_none());
    }

    #[test]
    fn non_ffi_languages_have_command_and_coverage() {
        for lang in all_languages() {
            // Skip FFI and Phase 1 backends not yet implemented
            if matches!(
                lang,
                Language::Ffi | Language::Kotlin | Language::Swift | Language::Dart | Language::Gleam | Language::Zig
            ) {
                continue;
            }
            let c = cfg(lang, "packages/test");
            assert!(c.command.is_some(), "{lang} should have a default test command");
            assert!(c.coverage.is_some(), "{lang} should have a default coverage command");
        }
    }

    #[test]
    fn non_ffi_languages_have_default_precondition() {
        for lang in all_languages() {
            // Skip FFI and Phase 1 backends not yet implemented
            if matches!(
                lang,
                Language::Ffi | Language::Kotlin | Language::Swift | Language::Dart | Language::Gleam | Language::Zig
            ) {
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
    fn e2e_is_always_none() {
        for lang in all_languages() {
            let c = cfg(lang, "packages/test");
            assert!(c.e2e.is_none(), "{lang} e2e should always be None (user-configured)");
        }
    }

    #[test]
    fn python_uses_pytest_via_uv_by_default() {
        let c = cfg(Language::Python, "packages/python");
        let cmd = c.command.unwrap().commands().join(" ");
        assert!(cmd.contains("uv run pytest"));
    }

    #[test]
    fn python_test_dispatches_on_package_manager() {
        for (pm, expected) in [("pip", "&& pytest"), ("poetry", "poetry run pytest")] {
            let tools = ToolsConfig {
                python_package_manager: Some(pm.to_string()),
                ..Default::default()
            };
            let ctx = LangContext::default(&tools);
            let c = default_test_config(Language::Python, "packages/python", &ctx);
            assert!(
                c.command.unwrap().commands().join(" ").contains(expected),
                "{pm}: expected {expected}"
            );
        }
    }

    #[test]
    fn node_uses_pnpm_by_default() {
        let c = cfg(Language::Node, "packages/node");
        let cmd = c.command.unwrap().commands().join(" ");
        assert!(cmd.contains("pnpm test"));
    }

    #[test]
    fn node_test_dispatches_on_package_manager() {
        for (pm, expected) in [("npm", "npm test"), ("yarn", "yarn test")] {
            let tools = ToolsConfig {
                node_package_manager: Some(pm.to_string()),
                ..Default::default()
            };
            let ctx = LangContext::default(&tools);
            let c = default_test_config(Language::Node, "packages/node", &ctx);
            assert!(
                c.command.unwrap().commands().join(" ").contains(expected),
                "{pm}: expected {expected}"
            );
        }
    }

    #[test]
    fn go_uses_go_test() {
        let c = cfg(Language::Go, "packages/go");
        let cmd = c.command.unwrap().commands().join(" ");
        assert!(cmd.contains("go test ./..."));
    }

    #[test]
    fn ruby_uses_rspec() {
        let c = cfg(Language::Ruby, "packages/ruby");
        let cmd = c.command.unwrap().commands().join(" ");
        assert!(cmd.contains("bundle exec rspec"));
    }

    #[test]
    fn java_uses_maven() {
        let c = cfg(Language::Java, "packages/java");
        let cmd = c.command.unwrap().commands().join(" ");
        let cov = c.coverage.unwrap().commands().join(" ");
        assert!(cmd.contains("mvn"));
        assert!(cov.contains("jacoco:report"));
    }

    #[test]
    fn rust_uses_cargo_and_llvm_cov() {
        let c = cfg(Language::Rust, "packages/rust");
        let cmd = c.command.unwrap().commands().join(" ");
        let cov = c.coverage.unwrap().commands().join(" ");
        assert!(cmd.contains("cargo test --workspace"));
        assert!(cov.contains("cargo llvm-cov"));
    }

    #[test]
    fn output_dir_substituted_in_commands() {
        let c = cfg(Language::Python, "my/custom/dir");
        let cmd = c.command.unwrap().commands().join(" ");
        assert!(cmd.contains("my/custom/dir"));
    }
}
