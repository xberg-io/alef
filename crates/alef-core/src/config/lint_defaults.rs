use super::extras::Language;
use super::output::{LintConfig, StringOrVec};
use super::tools::{LangContext, append_paths, require_tool, wrap_command as wrap};

/// Return the default lint configuration for a language.
///
/// The `output_dir` is the package directory where scaffolded files live
/// (e.g. `packages/python`). It is substituted into command templates.
/// `ctx` provides tool selection, run_wrapper, and extra_lint_paths.
pub fn default_lint_config(lang: Language, output_dir: &str, ctx: &LangContext) -> LintConfig {
    match lang {
        Language::Python => {
            let format_cmd = wrap(
                append_paths(format!("ruff format {output_dir}"), ctx.extra_lint_paths),
                ctx.run_wrapper,
            );
            let check_cmd = wrap(
                append_paths(format!("ruff check --fix {output_dir}"), ctx.extra_lint_paths),
                ctx.run_wrapper,
            );
            let typecheck_cmd = wrap(
                append_paths(format!("mypy {output_dir}"), ctx.extra_lint_paths),
                ctx.run_wrapper,
            );
            LintConfig {
                precondition: Some(require_tool("ruff")),
                before: None,
                format: Some(StringOrVec::Single(format_cmd)),
                check: Some(StringOrVec::Single(check_cmd)),
                typecheck: Some(StringOrVec::Single(typecheck_cmd)),
            }
        }
        Language::Node | Language::Wasm => {
            let pm = ctx.tools.node_pm();
            let runner: &str = match pm {
                "pnpm" => "pnpm exec",
                "yarn" => "yarn dlx",
                _ => "npx",
            };
            let format_cmd = wrap(
                append_paths(format!("{runner} oxfmt {output_dir}"), ctx.extra_lint_paths),
                ctx.run_wrapper,
            );
            let check_cmd = wrap(
                append_paths(format!("{runner} oxlint --fix {output_dir}"), ctx.extra_lint_paths),
                ctx.run_wrapper,
            );
            LintConfig {
                precondition: Some(require_tool(pm)),
                before: None,
                format: Some(StringOrVec::Single(format_cmd)),
                check: Some(StringOrVec::Single(check_cmd)),
                typecheck: None,
            }
        }
        Language::Ruby => {
            let format_cmd = wrap(
                append_paths(
                    format!("cd {output_dir} && bundle exec rubocop -A ."),
                    ctx.extra_lint_paths,
                ),
                ctx.run_wrapper,
            );
            let check_cmd = wrap(
                append_paths(
                    format!("cd {output_dir} && bundle exec rubocop ."),
                    ctx.extra_lint_paths,
                ),
                ctx.run_wrapper,
            );
            LintConfig {
                precondition: Some(require_tool("bundle")),
                before: None,
                format: Some(StringOrVec::Single(format_cmd)),
                check: Some(StringOrVec::Single(check_cmd)),
                typecheck: None,
            }
        }
        Language::Php => {
            let format_cmd = wrap(
                append_paths(format!("cd {output_dir} && composer run format"), ctx.extra_lint_paths),
                ctx.run_wrapper,
            );
            let check_cmd = wrap(
                append_paths(format!("cd {output_dir} && composer run lint"), ctx.extra_lint_paths),
                ctx.run_wrapper,
            );
            LintConfig {
                precondition: Some(require_tool("composer")),
                before: None,
                format: Some(StringOrVec::Single(format_cmd)),
                check: Some(StringOrVec::Single(check_cmd)),
                typecheck: None,
            }
        }
        Language::Go => {
            let format_cmd = wrap(
                append_paths(format!("gofmt -w {output_dir}"), ctx.extra_lint_paths),
                ctx.run_wrapper,
            );
            let check_cmd = wrap(
                append_paths(
                    format!("cd {output_dir} && golangci-lint run ./..."),
                    ctx.extra_lint_paths,
                ),
                ctx.run_wrapper,
            );
            LintConfig {
                precondition: Some(require_tool("gofmt")),
                before: None,
                format: Some(StringOrVec::Single(format_cmd)),
                check: Some(StringOrVec::Single(check_cmd)),
                typecheck: None,
            }
        }
        Language::Java => {
            let (format_path, check_path) = if let Some(proj) = ctx.project_file {
                (
                    format!("mvn -f {proj} spotless:apply -q"),
                    format!("mvn -f {proj} spotless:check checkstyle:check -q"),
                )
            } else {
                (
                    format!("mvn -f {output_dir}/pom.xml spotless:apply -q"),
                    format!("mvn -f {output_dir}/pom.xml spotless:check checkstyle:check -q"),
                )
            };
            LintConfig {
                precondition: Some(require_tool("mvn")),
                before: None,
                format: Some(StringOrVec::Single(wrap(format_path, ctx.run_wrapper))),
                check: Some(StringOrVec::Single(wrap(check_path, ctx.run_wrapper))),
                typecheck: None,
            }
        }
        Language::Csharp => {
            let (format_path, check_path) = if let Some(proj) = ctx.project_file {
                (
                    format!("dotnet format {proj}"),
                    format!("dotnet format {proj} --verify-no-changes"),
                )
            } else {
                (
                    format!("dotnet format {output_dir}"),
                    format!("dotnet format {output_dir} --verify-no-changes"),
                )
            };
            LintConfig {
                precondition: Some(require_tool("dotnet")),
                before: None,
                format: Some(StringOrVec::Single(wrap(format_path, ctx.run_wrapper))),
                check: Some(StringOrVec::Single(wrap(check_path, ctx.run_wrapper))),
                typecheck: None,
            }
        }
        Language::Elixir => {
            let format_cmd = wrap(
                append_paths(format!("cd {output_dir} && mix format"), ctx.extra_lint_paths),
                ctx.run_wrapper,
            );
            let check_cmd = wrap(
                append_paths(format!("cd {output_dir} && mix credo --strict"), ctx.extra_lint_paths),
                ctx.run_wrapper,
            );
            LintConfig {
                precondition: Some(require_tool("mix")),
                before: None,
                format: Some(StringOrVec::Single(format_cmd)),
                check: Some(StringOrVec::Single(check_cmd)),
                typecheck: None,
            }
        }
        Language::R => {
            let format_cmd = wrap(
                append_paths(
                    format!("cd {output_dir} && Rscript -e \"styler::style_pkg()\""),
                    ctx.extra_lint_paths,
                ),
                ctx.run_wrapper,
            );
            let check_cmd = wrap(
                append_paths(
                    format!("cd {output_dir} && Rscript -e \"lintr::lint_package()\""),
                    ctx.extra_lint_paths,
                ),
                ctx.run_wrapper,
            );
            LintConfig {
                precondition: Some(require_tool("Rscript")),
                before: None,
                format: Some(StringOrVec::Single(format_cmd)),
                check: Some(StringOrVec::Single(check_cmd)),
                typecheck: None,
            }
        }
        Language::Ffi => LintConfig {
            precondition: Some(require_tool("clang-format")),
            before: None,
            format: Some(StringOrVec::Single(format!(
                "find {output_dir} -name '*.c' -o -name '*.h' | xargs clang-format -i"
            ))),
            check: Some(StringOrVec::Single(format!(
                "cppcheck --std=c11 --enable=warning,style,performance --suppress=missingIncludeSystem {output_dir}"
            ))),
            typecheck: None,
        },
        Language::Rust => LintConfig {
            precondition: Some(require_tool("cargo")),
            before: None,
            format: Some(StringOrVec::Single("cargo fmt".to_string())),
            check: Some(StringOrVec::Single(
                "cargo clippy --fix --allow-dirty --allow-staged -- -D warnings".to_string(),
            )),
            typecheck: None,
        },
        Language::Kotlin => {
            let format_cmd = wrap(
                format!("cd {output_dir} && gradle ktlintFormat"),
                ctx.run_wrapper,
            );
            let check_cmd = wrap(
                format!("cd {output_dir} && gradle ktlintCheck"),
                ctx.run_wrapper,
            );
            LintConfig {
                precondition: Some(require_tool("gradle")),
                before: None,
                format: Some(StringOrVec::Single(format_cmd)),
                check: Some(StringOrVec::Single(check_cmd)),
                typecheck: None,
            }
        }
        Language::Swift => {
            let format_cmd = wrap(
                format!("cd {output_dir} && swift format --in-place --recursive Sources Tests"),
                ctx.run_wrapper,
            );
            let check_cmd = wrap(
                format!("cd {output_dir} && swift format lint --recursive Sources Tests"),
                ctx.run_wrapper,
            );
            LintConfig {
                precondition: Some(require_tool("swift")),
                before: None,
                format: Some(StringOrVec::Single(format_cmd)),
                check: Some(StringOrVec::Single(check_cmd)),
                typecheck: None,
            }
        }
        Language::Dart => {
            let format_cmd = wrap(
                append_paths(format!("dart format {output_dir}"), ctx.extra_lint_paths),
                ctx.run_wrapper,
            );
            let check_cmd = wrap(
                append_paths(format!("dart analyze {output_dir}"), ctx.extra_lint_paths),
                ctx.run_wrapper,
            );
            LintConfig {
                precondition: Some(require_tool("dart")),
                before: None,
                format: Some(StringOrVec::Single(format_cmd)),
                check: Some(StringOrVec::Single(check_cmd)),
                typecheck: None,
            }
        }
        Language::Gleam => {
            let format_cmd = wrap(
                format!("cd {output_dir} && gleam format"),
                ctx.run_wrapper,
            );
            let check_cmd = wrap(
                format!("cd {output_dir} && gleam format --check"),
                ctx.run_wrapper,
            );
            LintConfig {
                precondition: Some(require_tool("gleam")),
                before: None,
                format: Some(StringOrVec::Single(format_cmd)),
                check: Some(StringOrVec::Single(check_cmd)),
                typecheck: None,
            }
        }
        Language::Zig => {
            let format_cmd = wrap(
                format!("cd {output_dir} && zig fmt src"),
                ctx.run_wrapper,
            );
            let check_cmd = wrap(
                format!("cd {output_dir} && zig fmt --check src"),
                ctx.run_wrapper,
            );
            LintConfig {
                precondition: Some(require_tool("zig")),
                before: None,
                format: Some(StringOrVec::Single(format_cmd)),
                check: Some(StringOrVec::Single(check_cmd)),
                typecheck: None,
            }
        }
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

    fn cfg(lang: Language, dir: &str) -> LintConfig {
        let tools = ToolsConfig::default();
        let ctx = LangContext::default(&tools);
        default_lint_config(lang, dir, &ctx)
    }

    #[test]
    fn every_language_has_format_default() {
        for lang in all_languages() {
            let c = cfg(lang, "packages/test");
            assert!(c.format.is_some(), "{lang} should have a default format command");
        }
    }

    #[test]
    fn every_language_has_check_default() {
        for lang in all_languages() {
            let c = cfg(lang, "packages/test");
            assert!(c.check.is_some(), "{lang} should have a default check command");
        }
    }

    #[test]
    fn every_language_has_default_precondition() {
        for lang in all_languages() {
            let c = cfg(lang, "packages/test");
            let pre = c
                .precondition
                .unwrap_or_else(|| panic!("{lang} default lint should have a precondition"));
            assert!(
                pre.starts_with("command -v "),
                "{lang} precondition should use POSIX `command -v`, got: {pre}"
            );
        }
    }

    #[test]
    fn python_defaults_use_ruff_and_mypy() {
        let c = cfg(Language::Python, "packages/python");
        let fmt = c.format.unwrap().commands().join(" ");
        let check = c.check.unwrap().commands().join(" ");
        let tc = c.typecheck.unwrap().commands().join(" ");
        assert!(fmt.contains("ruff format"));
        assert!(check.contains("ruff check"));
        assert!(tc.contains("mypy"));
        assert_eq!(c.precondition.as_deref(), Some("command -v ruff >/dev/null 2>&1"));
    }

    #[test]
    fn node_defaults_use_oxc() {
        let c = cfg(Language::Node, "packages/node");
        let fmt = c.format.unwrap().commands().join(" ");
        let check = c.check.unwrap().commands().join(" ");
        assert!(fmt.contains("oxfmt"), "Node format should use oxfmt, got: {fmt}");
        assert!(check.contains("oxlint"), "Node check should use oxlint, got: {check}");
        assert!(!fmt.contains("biome"), "Node should not reference biome");
    }

    #[test]
    fn node_lint_dispatches_on_package_manager() {
        let mk = |pm: &str| ToolsConfig {
            node_package_manager: Some(pm.to_string()),
            ..Default::default()
        };

        let cases = [
            ("pnpm", "command -v pnpm >/dev/null 2>&1", "pnpm exec"),
            ("yarn", "command -v yarn >/dev/null 2>&1", "yarn dlx"),
            ("npm", "command -v npm >/dev/null 2>&1", "npx"),
        ];

        for (pm, expected_pre, expected_runner) in cases {
            let tools = mk(pm);
            let ctx = LangContext::default(&tools);
            let c = default_lint_config(Language::Node, "packages/node", &ctx);
            assert_eq!(
                c.precondition.as_deref(),
                Some(expected_pre),
                "{pm}: precondition mismatch"
            );
            let fmt = c.format.unwrap().commands().join(" ");
            let check = c.check.unwrap().commands().join(" ");
            assert!(
                fmt.contains(&format!("{expected_runner} oxfmt")),
                "{pm}: format should use `{expected_runner} oxfmt`, got: {fmt}"
            );
            assert!(
                check.contains(&format!("{expected_runner} oxlint")),
                "{pm}: check should use `{expected_runner} oxlint`, got: {check}"
            );
        }
    }

    #[test]
    fn wasm_defaults_match_node() {
        let node = cfg(Language::Node, "packages/node");
        let wasm = cfg(Language::Wasm, "packages/wasm");
        let node_fmt = node.format.unwrap().commands().join(" ");
        let wasm_fmt = wasm.format.unwrap().commands().join(" ");
        assert!(node_fmt.contains("oxfmt"));
        assert!(wasm_fmt.contains("oxfmt"));
    }

    #[test]
    fn java_defaults_use_spotless() {
        let c = cfg(Language::Java, "packages/java");
        let fmt = c.format.unwrap().commands().join(" ");
        let check = c.check.unwrap().commands().join(" ");
        assert!(fmt.contains("spotless:apply"));
        assert!(check.contains("spotless:check"));
        assert!(check.contains("checkstyle:check"));
    }

    #[test]
    fn rust_defaults_use_cargo() {
        let c = cfg(Language::Rust, "packages/rust");
        let fmt = c.format.unwrap().commands().join(" ");
        let check = c.check.unwrap().commands().join(" ");
        assert!(fmt.contains("cargo fmt"));
        assert!(check.contains("cargo clippy"));
    }

    #[test]
    fn output_dir_substituted_in_commands() {
        let c = cfg(Language::Go, "my/custom/dir");
        let fmt = c.format.unwrap().commands().join(" ");
        let check = c.check.unwrap().commands().join(" ");
        assert!(fmt.contains("my/custom/dir"));
        assert!(check.contains("my/custom/dir"));
    }

    #[test]
    fn only_python_has_typecheck_default() {
        for lang in all_languages() {
            let c = cfg(lang, "packages/test");
            if lang == Language::Python {
                assert!(c.typecheck.is_some(), "Python should have typecheck");
            } else {
                assert!(c.typecheck.is_none(), "{lang} should not have typecheck default");
            }
        }
    }

    #[test]
    fn python_run_wrapper_prefixes_all_commands() {
        let ctx = LangContext {
            tools: &ToolsConfig::default(),
            run_wrapper: Some("uv run --no-sync"),
            extra_lint_paths: &[],
            project_file: None,
        };
        let c = default_lint_config(Language::Python, "packages/python", &ctx);
        let fmt = c.format.unwrap().commands().join(" ");
        let check = c.check.unwrap().commands().join(" ");
        let tc = c.typecheck.unwrap().commands().join(" ");
        assert!(fmt.starts_with("uv run --no-sync"), "format should be wrapped: {fmt}");
        assert!(
            check.starts_with("uv run --no-sync"),
            "check should be wrapped: {check}"
        );
        assert!(tc.starts_with("uv run --no-sync"), "typecheck should be wrapped: {tc}");
    }

    #[test]
    fn python_extra_lint_paths_appended() {
        let ctx = LangContext {
            tools: &ToolsConfig::default(),
            run_wrapper: None,
            extra_lint_paths: &["scripts".to_string()],
            project_file: None,
        };
        let c = default_lint_config(Language::Python, "packages/python", &ctx);
        let fmt = c.format.unwrap().commands().join(" ");
        assert!(
            fmt.contains("packages/python scripts"),
            "format should include both paths: {fmt}"
        );
    }

    #[test]
    fn java_project_file_replaces_output_dir() {
        let ctx = LangContext {
            tools: &ToolsConfig::default(),
            run_wrapper: None,
            extra_lint_paths: &[],
            project_file: Some("pom.xml"),
        };
        let c = default_lint_config(Language::Java, "packages/java", &ctx);
        let fmt = c.format.unwrap().commands().join(" ");
        let check = c.check.unwrap().commands().join(" ");
        assert!(fmt.contains("-f pom.xml"), "format should use project_file: {fmt}");
        assert!(
            !fmt.contains("packages/java/pom.xml"),
            "format should not use output_dir path"
        );
        assert!(check.contains("-f pom.xml"), "check should use project_file: {check}");
    }

    #[test]
    fn csharp_project_file_replaces_output_dir() {
        let ctx = LangContext {
            tools: &ToolsConfig::default(),
            run_wrapper: None,
            extra_lint_paths: &[],
            project_file: Some("MyProject.csproj"),
        };
        let c = default_lint_config(Language::Csharp, "packages/csharp", &ctx);
        let fmt = c.format.unwrap().commands().join(" ");
        let check = c.check.unwrap().commands().join(" ");
        assert!(
            fmt.contains("MyProject.csproj"),
            "format should use project_file: {fmt}"
        );
        assert!(!fmt.contains("packages/csharp"), "format should not use output_dir");
        assert!(
            check.contains("MyProject.csproj"),
            "check should use project_file: {check}"
        );
    }

    #[test]
    fn go_run_wrapper_and_extra_paths() {
        let ctx = LangContext {
            tools: &ToolsConfig::default(),
            run_wrapper: Some("time"),
            extra_lint_paths: &["vendor".to_string()],
            project_file: None,
        };
        let c = default_lint_config(Language::Go, "packages/go", &ctx);
        let fmt = c.format.unwrap().commands().join(" ");
        assert!(
            fmt.starts_with("time gofmt"),
            "format should be wrapped with time: {fmt}"
        );
        assert!(
            fmt.contains("packages/go vendor"),
            "format should include extra paths: {fmt}"
        );
    }

    #[test]
    fn kotlin_uses_gradle_ktlint() {
        let c = cfg(Language::Kotlin, "packages/kotlin");
        let fmt = c.format.unwrap().commands().join(" ");
        let check = c.check.unwrap().commands().join(" ");
        assert!(fmt.contains("gradle ktlintFormat"), "Kotlin format should use gradle ktlintFormat, got: {fmt}");
        assert!(check.contains("gradle ktlintCheck"), "Kotlin check should use gradle ktlintCheck, got: {check}");
        assert_eq!(c.precondition.as_deref(), Some("command -v gradle >/dev/null 2>&1"));
    }

    #[test]
    fn swift_uses_swift_format() {
        let c = cfg(Language::Swift, "packages/swift");
        let fmt = c.format.unwrap().commands().join(" ");
        let check = c.check.unwrap().commands().join(" ");
        assert!(fmt.contains("swift format --in-place"), "Swift format should use swift format --in-place, got: {fmt}");
        assert!(check.contains("swift format lint"), "Swift check should use swift format lint, got: {check}");
        assert_eq!(c.precondition.as_deref(), Some("command -v swift >/dev/null 2>&1"));
    }

    #[test]
    fn dart_uses_dart_format_and_analyze() {
        let c = cfg(Language::Dart, "packages/dart");
        let fmt = c.format.unwrap().commands().join(" ");
        let check = c.check.unwrap().commands().join(" ");
        assert!(fmt.contains("dart format"), "Dart format should use dart format, got: {fmt}");
        assert!(check.contains("dart analyze"), "Dart check should use dart analyze, got: {check}");
        assert_eq!(c.precondition.as_deref(), Some("command -v dart >/dev/null 2>&1"));
    }

    #[test]
    fn gleam_uses_gleam_format() {
        let c = cfg(Language::Gleam, "packages/gleam");
        let fmt = c.format.unwrap().commands().join(" ");
        let check = c.check.unwrap().commands().join(" ");
        assert!(fmt.contains("gleam format"), "Gleam format should use gleam format, got: {fmt}");
        assert!(check.contains("gleam format --check"), "Gleam check should use gleam format --check, got: {check}");
        assert_eq!(c.precondition.as_deref(), Some("command -v gleam >/dev/null 2>&1"));
    }

    #[test]
    fn zig_uses_zig_fmt() {
        let c = cfg(Language::Zig, "packages/zig");
        let fmt = c.format.unwrap().commands().join(" ");
        let check = c.check.unwrap().commands().join(" ");
        assert!(fmt.contains("zig fmt src"), "Zig format should use zig fmt src, got: {fmt}");
        assert!(check.contains("zig fmt --check src"), "Zig check should use zig fmt --check src, got: {check}");
        assert_eq!(c.precondition.as_deref(), Some("command -v zig >/dev/null 2>&1"));
    }
}
