use super::extras::Language;
use super::output::{StringOrVec, TestConfig};
use super::tools::{LangContext, require_ruby_bundler, require_tool, ruby_bundle_exec, wrap_command as wrap};

/// Return the default test configuration for a language.
///
/// The `output_dir` is the package directory where scaffolded files live
/// (e.g. `packages/python`). It is substituted into command templates.
/// `ctx` provides tool selection and run_wrapper.
pub(crate) fn default_test_config(lang: Language, output_dir: &str, ctx: &LangContext) -> TestConfig {
    let output_dir = super::shell::quote_word(output_dir);
    match lang {
        Language::Python => {
            let pm = ctx.tools.python_pm();
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
                    wrap(format!("cd {output_dir} && uv run --no-sync pytest"), ctx.run_wrapper),
                    wrap(
                        format!("cd {output_dir} && uv run --no-sync pytest --cov=. --cov-report=lcov"),
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
                e2e_precondition: None,
                coverage: Some(StringOrVec::Single(cov)),
            }
        }
        Language::Node | Language::Wasm => {
            let pm = ctx.tools.node_pm();
            let (cmd, cov) = match pm {
                "npm" => (
                    wrap(format!("cd {output_dir} && pnpm test"), ctx.run_wrapper),
                    wrap(format!("cd {output_dir} && pnpm test -- --coverage"), ctx.run_wrapper),
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
                e2e_precondition: None,
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
                e2e_precondition: None,
                coverage: Some(StringOrVec::Single(cov)),
            }
        }
        Language::Ruby => {
            let cmd = wrap(
                format!("cd {output_dir} && {}", ruby_bundle_exec("rspec")),
                ctx.run_wrapper,
            );
            let cov = wrap(
                format!(
                    "cd {output_dir} && {}",
                    ruby_bundle_exec("rspec --format documentation")
                ),
                ctx.run_wrapper,
            );
            TestConfig {
                precondition: Some(require_ruby_bundler()),
                before: None,
                command: Some(StringOrVec::Single(cmd)),
                e2e: None,
                e2e_precondition: None,
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
                e2e_precondition: None,
                coverage: Some(StringOrVec::Single(cov)),
            }
        }
        Language::Java => {
            let (cmd_path, cov_path) = if let Some(proj) = ctx.project_file {
                // `[crates.java] project_file` is consumer-configured and reaches `sh -c` as
                // `mvn -f`'s value -- quote it the same way `output_dir` is quoted. ~keep
                let proj = super::shell::quote_word(proj);
                (
                    format!("mvn -f {proj} test --batch-mode --no-transfer-progress"),
                    format!("mvn -f {proj} test jacoco:report --batch-mode --no-transfer-progress"),
                )
            } else {
                (
                    format!("mvn -f {output_dir}/pom.xml test --batch-mode --no-transfer-progress"),
                    format!("mvn -f {output_dir}/pom.xml test jacoco:report --batch-mode --no-transfer-progress"),
                )
            };
            TestConfig {
                precondition: Some(require_tool("mvn")),
                before: None,
                command: Some(StringOrVec::Single(wrap(cmd_path, ctx.run_wrapper))),
                e2e: None,
                e2e_precondition: None,
                coverage: Some(StringOrVec::Single(wrap(cov_path, ctx.run_wrapper))),
            }
        }
        Language::Csharp => {
            let (cmd_path, cov_path) = if let Some(proj) = ctx.project_file {
                // `[crates.csharp] project_file` is consumer-configured and reaches `sh -c` as
                // `dotnet test`'s value -- quote it the same way `output_dir` is quoted. ~keep
                let proj = super::shell::quote_word(proj);
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
                e2e_precondition: None,
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
                e2e_precondition: None,
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
                e2e_precondition: None,
                coverage: Some(StringOrVec::Single(cov)),
            }
        }
        Language::Rust => TestConfig {
            precondition: Some(require_tool("cargo")),
            before: None,
            command: Some(StringOrVec::Single("cargo test --workspace".to_string())),
            e2e: None,
            e2e_precondition: None,
            coverage: Some(StringOrVec::Single(
                "cargo llvm-cov --workspace --lcov --output-path coverage.lcov".to_string(),
            )),
        },
        Language::Ffi => TestConfig {
            precondition: None,
            before: None,
            command: None,
            e2e: None,
            e2e_precondition: None,
            coverage: None,
        },
        Language::C => TestConfig {
            precondition: None,
            before: None,
            command: None,
            e2e: None,
            e2e_precondition: None,
            coverage: None,
        },
        Language::Kotlin | Language::KotlinAndroid => {
            let cmd = wrap(format!("cd {output_dir} && gradle test"), ctx.run_wrapper);
            TestConfig {
                precondition: Some(require_tool("gradle")),
                before: None,
                command: Some(StringOrVec::Single(cmd.clone())),
                e2e: None,
                e2e_precondition: None,
                coverage: Some(StringOrVec::Single(cmd)),
            }
        }
        Language::Swift => {
            let cmd = wrap(format!("swift test --package-path {output_dir}"), ctx.run_wrapper);
            TestConfig {
                precondition: Some(require_tool("swift")),
                before: None,
                command: Some(StringOrVec::Single(cmd.clone())),
                e2e: None,
                e2e_precondition: None,
                coverage: Some(StringOrVec::Single(cmd)),
            }
        }
        Language::Dart => {
            let cmd = wrap(format!("cd {output_dir} && dart test"), ctx.run_wrapper);
            let e2e_cmd = wrap(
                "cd e2e/dart && dart pub get && dart test --concurrency=1".to_string(),
                ctx.run_wrapper,
            );
            TestConfig {
                precondition: Some(require_tool("dart")),
                before: None,
                command: Some(StringOrVec::Single(cmd.clone())),
                e2e: Some(StringOrVec::Single(e2e_cmd)),
                e2e_precondition: None,
                coverage: Some(StringOrVec::Single(cmd)),
            }
        }
        Language::Zig => {
            let cmd = wrap(format!("cd {output_dir} && zig build test"), ctx.run_wrapper);
            TestConfig {
                precondition: Some(require_tool("zig")),
                before: None,
                command: Some(StringOrVec::Single(cmd.clone())),
                e2e: None,
                e2e_precondition: None,
                coverage: Some(StringOrVec::Single(cmd)),
            }
        }
        Language::Gleam => {
            let cmd = wrap(format!("cd {output_dir} && gleam test"), ctx.run_wrapper);
            TestConfig {
                precondition: Some(require_tool("gleam")),
                before: None,
                command: Some(StringOrVec::Single(cmd.clone())),
                e2e: None,
                e2e_precondition: None,
                coverage: Some(StringOrVec::Single(cmd)),
            }
        }
        Language::Jni => TestConfig {
            precondition: None,
            before: None,
            command: None,
            e2e: None,
            e2e_precondition: None,
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

    /// The directory as it is spelled *inside the emitted shell command* — a quoted word, not a
    /// bare path. Expectations derive it from `quote_word` rather than restating one quoting
    /// spelling, so a change to the escaping policy cannot silently repoint a command at a
    /// different directory: the escaping itself is proved separately, and once, by
    /// `shell::tests::quote_word_preserves_literal_shell_value`, which runs a hostile value
    /// through a real shell. ~keep
    fn quoted(dir: &str) -> String {
        super::super::shell::quote_word(dir)
    }

    #[test]
    fn generated_test_quotes_configured_output_directory() {
        let malicious = "packages/python; touch /tmp/alef-test; #";
        let commands = cfg(Language::Python, malicious)
            .command
            .expect("python test command")
            .commands()
            .join(" ");
        assert!(commands.contains(&format!("cd {}", super::super::shell::quote_word(malicious))));
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
            if matches!(lang, Language::Ffi) {
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
    fn e2e_is_none_for_non_dart_languages() {
        for lang in all_languages() {
            if matches!(lang, Language::Dart) {
                continue;
            }
            let c = cfg(lang, "packages/test");
            assert!(c.e2e.is_none(), "{lang} e2e should always be None (user-configured)");
        }
    }

    #[test]
    fn dart_has_default_e2e_command_using_dart_sdk() {
        let c = cfg(Language::Dart, "packages/dart");
        let e2e = c.e2e.expect("dart should have a default e2e command");
        let cmd = e2e.commands().join(" ");
        assert!(
            cmd.contains("e2e/dart"),
            "dart e2e should target e2e/dart directory, got: {cmd}"
        );
        assert!(
            cmd.contains("dart pub get"),
            "dart e2e should use dart pub get (not flutter pub get), got: {cmd}"
        );
        assert!(
            cmd.contains("dart test"),
            "dart e2e should use dart test (not flutter test), got: {cmd}"
        );
        assert!(
            cmd.contains("--concurrency=1"),
            "dart e2e should run with --concurrency=1, got: {cmd}"
        );
        assert!(
            !cmd.contains("flutter"),
            "dart e2e must not use flutter (impellerc fails in headless env), got: {cmd}"
        );
    }

    #[test]
    fn python_uses_pytest_via_uv_by_default() {
        let c = cfg(Language::Python, "packages/python");
        let cmd = c.command.unwrap().commands().join(" ");
        assert!(cmd.contains("uv run --no-sync pytest"));
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
        assert_eq!(
            c.precondition.as_deref(),
            Some(
                "command -v ruby >/dev/null 2>&1 && BUNDLE_PATH=vendor/bundle ruby -S bundle --version >/dev/null 2>&1"
            )
        );
        let cmd = c.command.unwrap().commands().join(" ");
        let coverage = c.coverage.unwrap().commands().join(" ");
        assert!(
            cmd.contains("BUNDLE_PATH=vendor/bundle ruby -S bundle exec ruby -S rspec"),
            "got: {cmd}"
        );
        assert!(
            coverage.contains("BUNDLE_PATH=vendor/bundle ruby -S bundle exec ruby -S rspec --format documentation"),
            "got: {coverage}"
        );
    }

    #[test]
    fn java_uses_maven() {
        let c = cfg(Language::Java, "packages/java");
        let cmd = c.command.unwrap().commands().join(" ");
        let cov = c.coverage.unwrap().commands().join(" ");
        assert!(cmd.contains("mvn"));
        assert!(cov.contains("jacoco:report"));
    }

    /// `[crates.java] project_file` is consumer-configured and reaches `sh -c` as `mvn -f`'s
    /// value -- same shell-quoting requirement as `output_dir`. ~keep
    #[test]
    fn java_project_file_is_quoted() {
        let ctx = LangContext {
            tools: &ToolsConfig::default(),
            run_wrapper: None,
            extra_lint_paths: &[],
            project_file: Some("pom.xml"),
        };
        let c = default_test_config(Language::Java, "packages/java", &ctx);
        let cmd = c.command.unwrap().commands().join(" ");
        let cov = c.coverage.unwrap().commands().join(" ");
        let proj = quoted("pom.xml");
        assert!(
            cmd.contains(&format!("-f {proj}")),
            "test command should use project_file: {cmd}"
        );
        assert!(!cmd.contains("packages/java/pom.xml"), "should not use output_dir path");
        assert!(
            cov.contains(&format!("-f {proj}")),
            "coverage command should use project_file: {cov}"
        );
    }

    /// `[crates.csharp] project_file` is consumer-configured and reaches `sh -c` as `dotnet
    /// test`'s value -- same shell-quoting requirement as `output_dir`. ~keep
    #[test]
    fn csharp_project_file_is_quoted() {
        let ctx = LangContext {
            tools: &ToolsConfig::default(),
            run_wrapper: None,
            extra_lint_paths: &[],
            project_file: Some("my project.csproj"),
        };
        let c = default_test_config(Language::Csharp, "packages/csharp", &ctx);
        let cmd = c.command.unwrap().commands().join(" ");
        let cov = c.coverage.unwrap().commands().join(" ");
        let proj = quoted("my project.csproj");
        assert!(
            cmd.contains(&format!("dotnet test {proj}")),
            "test command should use quoted project_file: {cmd}"
        );
        assert!(!cmd.contains("packages/csharp"), "should not use output_dir path");
        assert!(
            cov.contains(&format!("dotnet test {proj}")),
            "coverage command should use quoted project_file: {cov}"
        );
    }

    /// A hostile `project_file` reaching `sh -c` must not execute a command substitution --
    /// proved against a real `sh` rather than by reading, matching the java update-defaults
    /// injection-witness test. ~keep
    #[cfg(unix)]
    #[test]
    fn java_project_file_keeps_a_hostile_value_out_of_shell_text() {
        let root = tempfile::tempdir().expect("tempdir");
        let hostile = "pom.xml$(touch witness)#";
        let witness = root.path().join("witness");
        let ctx = LangContext {
            tools: &ToolsConfig::default(),
            run_wrapper: None,
            extra_lint_paths: &[],
            project_file: Some(hostile),
        };
        let c = default_test_config(Language::Java, "packages/java", &ctx);
        let cmd = c.command.unwrap().commands().join(" ");

        std::process::Command::new("sh")
            .args(["-c", &cmd])
            .current_dir(root.path())
            .output()
            .expect("shell should start");

        assert!(
            !witness.exists(),
            "command substitution in project_file must not run: {cmd}"
        );
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

    #[test]
    fn kotlin_uses_gradle_test() {
        let c = cfg(Language::Kotlin, "packages/kotlin");
        let cmd = c.command.unwrap().commands().join(" ");
        assert!(
            cmd.contains("gradle test"),
            "Kotlin test should use gradle test, got: {cmd}"
        );
        assert_eq!(c.precondition.as_deref(), Some("command -v gradle >/dev/null 2>&1"));
    }

    #[test]
    fn swift_uses_swift_test_with_package_path() {
        let c = cfg(Language::Swift, "packages/swift");
        let cmd = c.command.unwrap().commands().join(" ");
        assert!(
            cmd.contains("swift test"),
            "Swift test should use swift test, got: {cmd}"
        );
        assert!(
            cmd.contains(&format!("--package-path {}", quoted("packages/swift"))),
            "Swift test should include package path, got: {cmd}"
        );
    }

    #[test]
    fn dart_uses_dart_test() {
        let c = cfg(Language::Dart, "packages/dart");
        let cmd = c.command.unwrap().commands().join(" ");
        assert!(cmd.contains("dart test"), "Dart test should use dart test, got: {cmd}");
        assert_eq!(c.precondition.as_deref(), Some("command -v dart >/dev/null 2>&1"));
    }

    #[test]
    fn gleam_uses_gleam_test() {
        let c = cfg(Language::Gleam, "packages/gleam");
        let cmd = c.command.unwrap().commands().join(" ");
        assert!(
            cmd.contains("gleam test"),
            "Gleam test should use gleam test, got: {cmd}"
        );
        assert_eq!(c.precondition.as_deref(), Some("command -v gleam >/dev/null 2>&1"));
    }

    #[test]
    fn zig_uses_zig_build_test() {
        let c = cfg(Language::Zig, "packages/zig");
        let cmd = c.command.unwrap().commands().join(" ");
        assert!(
            cmd.contains("zig build test"),
            "Zig test should use zig build test, got: {cmd}"
        );
        assert_eq!(c.precondition.as_deref(), Some("command -v zig >/dev/null 2>&1"));
    }
}
