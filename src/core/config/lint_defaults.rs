use super::extras::Language;
use super::output::{LintConfig, StringOrVec};
use super::tools::{
    LangContext, append_paths, require_ruby_bundler, require_tool, ruby_bundle, ruby_bundle_exec, wrap_command as wrap,
};

/// Return the default lint configuration for a language.
///
/// The `output_dir` is the package directory where scaffolded files live
/// (e.g. `packages/python`). It is substituted into command templates.
/// `ctx` provides tool selection, run_wrapper, and extra_lint_paths.
pub fn default_lint_config(lang: Language, output_dir: &str, ctx: &LangContext) -> LintConfig {
    let output_dir = super::shell::quote_word(output_dir);
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
                append_paths(format!("pyrefly check {output_dir}"), ctx.extra_lint_paths),
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
        Language::Node => {
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
        Language::Wasm => LintConfig {
            precondition: Some(require_tool("cargo")),
            before: None,
            format: Some(StringOrVec::Single("cargo fmt --all".to_string())),
            check: Some(StringOrVec::Single(
                "cargo clippy --fix --allow-dirty --allow-staged -- -D warnings".to_string(),
            )),
            typecheck: None,
        },
        Language::Ruby => {
            // Project-local bundle install before rubocop so the gem (and its plugins) are
            // present — consumer repos used to hand-write this exact command as a
            // `[crates.lint.ruby]` override before 0.82.0 removed that table; it is now the only
            // way ruby lint runs at all. ~keep
            let before_cmd = wrap(
                format!("cd {output_dir} && {}", ruby_bundle("install")),
                ctx.run_wrapper,
            );
            let format_cmd = wrap(
                append_paths(
                    format!("cd {output_dir} && {}", ruby_bundle_exec("rubocop -A .")),
                    ctx.extra_lint_paths,
                ),
                ctx.run_wrapper,
            );
            let check_cmd = wrap(
                append_paths(
                    format!("cd {output_dir} && {}", ruby_bundle_exec("rubocop .")),
                    ctx.extra_lint_paths,
                ),
                ctx.run_wrapper,
            );
            LintConfig {
                precondition: Some(require_ruby_bundler()),
                before: Some(StringOrVec::Single(before_cmd)),
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
                // `[crates.java] project_file` is consumer-configured and reaches `sh -c` as
                // `mvn -f`'s value -- quote it the same way `output_dir` above is quoted. ~keep
                let proj = super::shell::quote_word(proj);
                (
                    format!("mvn -f {proj} spotless:apply --batch-mode --no-transfer-progress"),
                    format!("mvn -f {proj} spotless:check checkstyle:check --batch-mode --no-transfer-progress"),
                )
            } else {
                (
                    format!("mvn -f {output_dir}/pom.xml spotless:apply --batch-mode --no-transfer-progress"),
                    format!(
                        "mvn -f {output_dir}/pom.xml spotless:check checkstyle:check --batch-mode --no-transfer-progress"
                    ),
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
                // `[crates.csharp] project_file` is consumer-configured and reaches `sh -c` as
                // `dotnet format`'s value -- quote it the same way `output_dir` above is
                // quoted. ~keep
                let proj = super::shell::quote_word(proj);
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
            // `mix deps.get` before credo so deps (credo itself) are fetched — consumer repos
            // used to hand-write this exact command as a `[crates.lint.elixir]` override before
            // 0.82.0 removed that table; it is now the only way elixir lint runs at all.
            let before_cmd = wrap(format!("cd {output_dir} && mix deps.get"), ctx.run_wrapper);
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
                before: Some(StringOrVec::Single(before_cmd)),
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
        // Kotlin formats with ktfmt — the single Kotlin formatter across all
        // backends. ktlint is not wired anywhere; `build`/`.gradle` dirs are
        // pruned so generated/build artifacts aren't touched.
        Language::Kotlin => {
            let find_kt = format!(
                "find {output_dir} \\( -name build -o -name .gradle \\) -prune -o \
                 \\( -name '*.kt' -o -name '*.kts' \\) -type f -print0 | xargs -0 ktfmt --kotlinlang-style"
            );
            let format_cmd = wrap(find_kt.clone(), ctx.run_wrapper);
            let check_cmd = wrap(format!("{find_kt} --dry-run --set-exit-if-changed"), ctx.run_wrapper);
            LintConfig {
                precondition: Some(require_tool("ktfmt")),
                before: None,
                format: Some(StringOrVec::Single(format_cmd)),
                check: Some(StringOrVec::Single(check_cmd)),
                typecheck: None,
            }
        }
        // Kotlin-Android formats with ktfmt (not gradle ktlint): the Android
        // Gradle plugin is heavy to spin up for a format pass, so consumer repos
        // uniformly override to a `find … | xargs ktfmt --kotlinlang-style` sweep.
        // Making it the default lets them drop the identical override. `build`/
        // `.gradle` dirs are pruned so generated/build artifacts aren't touched.
        Language::KotlinAndroid => {
            let find_kt = format!(
                "find {output_dir} \\( -name build -o -name .gradle \\) -prune -o \
                 \\( -name '*.kt' -o -name '*.kts' \\) -type f -print0 | xargs -0 ktfmt --kotlinlang-style"
            );
            let format_cmd = wrap(find_kt.clone(), ctx.run_wrapper);
            let check_cmd = wrap(format!("{find_kt} --dry-run --set-exit-if-changed"), ctx.run_wrapper);
            LintConfig {
                precondition: Some(require_tool("ktfmt")),
                before: None,
                format: Some(StringOrVec::Single(format_cmd)),
                check: Some(StringOrVec::Single(check_cmd)),
                typecheck: None,
            }
        }
        Language::Swift => {
            // Only `Sources` (not `Tests`): consumer repos uniformly override to
            // drop `Tests`, so make it the default and let them delete the override.
            let format_cmd = wrap(
                format!("cd {output_dir} && swift format --in-place --recursive Sources"),
                ctx.run_wrapper,
            );
            let check_cmd = wrap(
                format!("cd {output_dir} && swift format lint --recursive Sources"),
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
        Language::Zig => {
            let format_cmd = wrap(format!("cd {output_dir} && zig fmt src build.zig"), ctx.run_wrapper);
            let check_cmd = wrap(
                format!("cd {output_dir} && zig fmt --check src build.zig"),
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
        Language::Gleam => {
            let format_cmd = wrap(format!("cd {output_dir} && gleam format"), ctx.run_wrapper);
            let check_cmd = wrap(format!("cd {output_dir} && gleam format --check"), ctx.run_wrapper);
            LintConfig {
                precondition: Some(require_tool("gleam")),
                before: None,
                format: Some(StringOrVec::Single(format_cmd)),
                check: Some(StringOrVec::Single(check_cmd)),
                typecheck: None,
            }
        }
        Language::C => LintConfig {
            precondition: None,
            before: None,
            format: None,
            check: None,
            typecheck: None,
        },
        Language::Jni => LintConfig {
            precondition: None,
            before: None,
            format: None,
            check: None,
            typecheck: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::super::tools::ToolsConfig;
    use super::*;

    /// The directory (or project file) as it is spelled *inside the emitted shell command* — a
    /// quoted word, not a bare path. Expectations derive it from `quote_word` rather than
    /// restating one quoting spelling, so a change to the escaping policy cannot silently repoint
    /// a command at a different directory: the escaping itself is proved separately, and once, by
    /// `shell::tests::quote_word_preserves_literal_shell_value`, which runs a hostile value
    /// through a real shell. ~keep
    fn quoted(dir: &str) -> String {
        super::super::shell::quote_word(dir)
    }

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
            Language::KotlinAndroid,
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
    fn python_defaults_use_ruff_and_pyrefly() {
        let c = cfg(Language::Python, "packages/python");
        let fmt = c.format.unwrap().commands().join(" ");
        let check = c.check.unwrap().commands().join(" ");
        let tc = c.typecheck.unwrap().commands().join(" ");
        assert!(fmt.contains("ruff format"));
        assert!(check.contains("ruff check"));
        assert!(tc.contains("pyrefly check"));
        assert_eq!(c.precondition.as_deref(), Some("command -v ruff >/dev/null 2>&1"));
    }

    #[test]
    fn node_defaults_use_oxc() {
        let c = cfg(Language::Node, "packages/node");
        let fmt = c.format.unwrap().commands().join(" ");
        let check = c.check.unwrap().commands().join(" ");
        assert!(fmt.contains("oxfmt"), "Node format should use oxfmt, got: {fmt}");
        assert!(check.contains("oxlint"), "Node check should use oxlint, got: {check}");
        assert!(
            !fmt.contains(concat!("bio", "me")),
            "Node should not reference the legacy formatter"
        );
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
    fn wasm_defaults_use_cargo_fmt() {
        let wasm = cfg(Language::Wasm, "packages/wasm");
        let fmt = wasm.format.unwrap().commands().join(" ");
        let check = wasm.check.unwrap().commands().join(" ");
        assert!(
            fmt.contains("cargo fmt"),
            "Wasm format should use cargo fmt, got: {fmt}"
        );
        assert!(
            check.contains("cargo clippy"),
            "Wasm check should use cargo clippy, got: {check}"
        );
        assert_eq!(wasm.precondition.as_deref(), Some("command -v cargo >/dev/null 2>&1"));
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
            fmt.contains(&format!("{} scripts", quoted("packages/python"))),
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
        let proj = quoted("pom.xml");
        assert!(
            fmt.contains(&format!("-f {proj}")),
            "format should use project_file: {fmt}"
        );
        assert!(
            !fmt.contains("packages/java/pom.xml"),
            "format should not use output_dir path"
        );
        assert!(
            check.contains(&format!("-f {proj}")),
            "check should use project_file: {check}"
        );
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
            fmt.contains(&format!("{} vendor", quoted("packages/go"))),
            "format should include extra paths: {fmt}"
        );
    }

    #[test]
    fn kotlin_uses_ktfmt() {
        let c = cfg(Language::Kotlin, "packages/kotlin");
        let fmt = c.format.unwrap().commands().join(" ");
        let check = c.check.unwrap().commands().join(" ");
        assert!(
            fmt.contains("ktfmt --kotlinlang-style"),
            "Kotlin format should use ktfmt, got: {fmt}"
        );
        assert!(
            !fmt.contains("gradle ktlint"),
            "Kotlin should not shell out to gradle ktlint, got: {fmt}"
        );
        assert!(
            check.contains("--dry-run --set-exit-if-changed"),
            "Kotlin check should be a non-mutating ktfmt run, got: {check}"
        );
        assert_eq!(c.precondition.as_deref(), Some("command -v ktfmt >/dev/null 2>&1"));
    }

    #[test]
    fn kotlin_android_uses_ktfmt() {
        let c = cfg(Language::KotlinAndroid, "packages/kotlin-android");
        let fmt = c.format.unwrap().commands().join(" ");
        let check = c.check.unwrap().commands().join(" ");
        assert!(
            fmt.contains("ktfmt --kotlinlang-style"),
            "Kotlin-Android format should use ktfmt, got: {fmt}"
        );
        assert!(
            !fmt.contains("gradle ktlint"),
            "Kotlin-Android should not shell out to gradle ktlint, got: {fmt}"
        );
        assert!(
            check.contains("--dry-run --set-exit-if-changed"),
            "Kotlin-Android check should be a non-mutating ktfmt run, got: {check}"
        );
        assert!(
            fmt.contains("packages/kotlin-android"),
            "output_dir should be substituted, got: {fmt}"
        );
        assert_eq!(c.precondition.as_deref(), Some("command -v ktfmt >/dev/null 2>&1"));
    }

    #[test]
    fn elixir_default_runs_deps_get_before() {
        let c = cfg(Language::Elixir, "packages/elixir");
        let before = c
            .before
            .expect("elixir default should fetch deps first")
            .commands()
            .join(" ");
        assert!(
            before.contains("mix deps.get"),
            "elixir before should run mix deps.get, got: {before}"
        );
    }

    #[test]
    fn ruby_default_runs_bundle_install_before() {
        let c = cfg(Language::Ruby, "packages/ruby");
        assert_eq!(
            c.precondition.as_deref(),
            Some(
                "command -v ruby >/dev/null 2>&1 && BUNDLE_PATH=vendor/bundle ruby -S bundle --version >/dev/null 2>&1"
            )
        );
        let before = c
            .before
            .expect("ruby default should install gems first")
            .commands()
            .join(" ");
        assert!(
            before.contains("BUNDLE_PATH=vendor/bundle ruby -S bundle install"),
            "ruby before should resolve Bundler through the active Ruby, got: {before}"
        );
    }

    #[test]
    fn ruby_defaults_resolve_bundler_through_the_active_ruby() {
        let config = cfg(Language::Ruby, "packages/ruby");
        let format = config.format.expect("ruby format command").commands().join(" ");
        let check = config.check.expect("ruby check command").commands().join(" ");

        assert!(
            format.contains("BUNDLE_PATH=vendor/bundle ruby -S bundle exec ruby -S rubocop -A ."),
            "ruby format must resolve Bundler through the active Ruby: {format}"
        );
        assert!(
            check.contains("BUNDLE_PATH=vendor/bundle ruby -S bundle exec ruby -S rubocop ."),
            "ruby lint must resolve Bundler through the active Ruby: {check}"
        );
    }

    #[test]
    fn swift_uses_swift_format() {
        let c = cfg(Language::Swift, "packages/swift");
        let fmt = c.format.unwrap().commands().join(" ");
        let check = c.check.unwrap().commands().join(" ");
        assert!(
            fmt.contains("swift format --in-place"),
            "Swift format should use swift format --in-place, got: {fmt}"
        );
        assert!(
            check.contains("swift format lint"),
            "Swift check should use swift format lint, got: {check}"
        );
        assert!(
            !fmt.contains("Tests") && !check.contains("Tests"),
            "Swift default should format only Sources, not Tests; got fmt: {fmt}, check: {check}"
        );
        assert_eq!(c.precondition.as_deref(), Some("command -v swift >/dev/null 2>&1"));
    }

    #[test]
    fn dart_uses_dart_format_and_analyze() {
        let c = cfg(Language::Dart, "packages/dart");
        let fmt = c.format.unwrap().commands().join(" ");
        let check = c.check.unwrap().commands().join(" ");
        assert!(
            fmt.contains("dart format"),
            "Dart format should use dart format, got: {fmt}"
        );
        assert!(
            check.contains("dart analyze"),
            "Dart check should use dart analyze, got: {check}"
        );
        assert_eq!(c.precondition.as_deref(), Some("command -v dart >/dev/null 2>&1"));
    }

    #[test]
    fn gleam_uses_gleam_format() {
        let c = cfg(Language::Gleam, "packages/gleam");
        let fmt = c.format.unwrap().commands().join(" ");
        let check = c.check.unwrap().commands().join(" ");
        assert!(
            fmt.contains("gleam format"),
            "Gleam format should use gleam format, got: {fmt}"
        );
        assert!(
            check.contains("gleam format --check"),
            "Gleam check should use gleam format --check, got: {check}"
        );
        assert_eq!(c.precondition.as_deref(), Some("command -v gleam >/dev/null 2>&1"));
    }

    #[test]
    fn zig_uses_zig_fmt() {
        let c = cfg(Language::Zig, "packages/zig");
        let fmt = c.format.unwrap().commands().join(" ");
        let check = c.check.unwrap().commands().join(" ");
        assert!(
            fmt.contains("zig fmt src build.zig"),
            "Zig format should use zig fmt src build.zig, got: {fmt}"
        );
        assert!(
            check.contains("zig fmt --check src build.zig"),
            "Zig check should use zig fmt --check src build.zig, got: {check}"
        );
        assert_eq!(c.precondition.as_deref(), Some("command -v zig >/dev/null 2>&1"));
    }
}
