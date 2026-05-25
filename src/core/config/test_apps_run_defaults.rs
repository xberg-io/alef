use super::extras::Language;
use super::output::{StringOrVec, TestAppRunConfig};
use super::tools::{LangContext, require_tool};

/// Return the default test-app run configuration for a language.
///
/// `test_apps_dir` is the registry-mode output directory (e.g. `test_apps`); the
/// per-language test app lives at `{test_apps_dir}/<lang-subdir>`, where the
/// subdir matches exactly what the test-apps generator (`src/e2e/codegen`) writes
/// for that language — usually the language name, but `swift` is emitted under
/// `swift_e2e` to give the SwiftPM package a distinct identity. `ctx` provides the
/// package-manager selection. Executed by `alef test-apps run` to install the
/// published package into the test app and exercise it.
pub fn default_test_apps_run_config(lang: Language, test_apps_dir: &str, ctx: &LangContext) -> TestAppRunConfig {
    match lang {
        Language::Rust => TestAppRunConfig {
            // Rust has no separate package manager — cargo handles install + test.
            precondition: Some(require_tool("cargo")),
            before: None,
            run: Some(StringOrVec::Single(format!("cd {test_apps_dir}/rust && cargo test"))),
        },
        Language::Python => {
            let pm = ctx.tools.python_pm();
            let run = match pm {
                "pip" => format!("cd {test_apps_dir}/python && pip install -e . && pytest"),
                "poetry" => format!("cd {test_apps_dir}/python && poetry install && poetry run pytest"),
                _ => format!("cd {test_apps_dir}/python && uv sync && uv run pytest"),
            };
            TestAppRunConfig {
                precondition: Some(require_tool(pm)),
                before: None,
                run: Some(StringOrVec::Single(run)),
            }
        }
        Language::Node => {
            let pm = ctx.tools.node_pm();
            let run = match pm {
                "npm" => format!("cd {test_apps_dir}/node && npm install && npm test"),
                "yarn" => format!("cd {test_apps_dir}/node && yarn install && yarn test"),
                _ => format!("cd {test_apps_dir}/node && pnpm install && pnpm test"),
            };
            TestAppRunConfig {
                precondition: Some(require_tool(pm)),
                before: None,
                run: Some(StringOrVec::Single(run)),
            }
        }
        Language::Wasm => {
            let pm = ctx.tools.node_pm();
            let run = match pm {
                "npm" => format!("cd {test_apps_dir}/wasm && npm install && npm test"),
                "yarn" => format!("cd {test_apps_dir}/wasm && yarn install && yarn test"),
                _ => format!("cd {test_apps_dir}/wasm && pnpm install && pnpm test"),
            };
            TestAppRunConfig {
                precondition: Some(require_tool(pm)),
                before: None,
                run: Some(StringOrVec::Single(run)),
            }
        }
        Language::Ruby => TestAppRunConfig {
            precondition: Some(require_tool("bundle")),
            before: None,
            run: Some(StringOrVec::Single(format!(
                "cd {test_apps_dir}/ruby && bundle install && bundle exec rspec"
            ))),
        },
        Language::Php => TestAppRunConfig {
            precondition: Some(require_tool("composer")),
            before: None,
            // PHP extensions are a special composer case: `composer install`
            // cannot satisfy a `type: php-ext` package because the platform
            // resolver consults `php -m` for `ext-<name>`. Alef emits an
            // `install.sh` next to composer.json that bootstraps PIE
            // (`composer global require php/pie:^1.4`) and runs
            // `pie install kreuzberg/<crate>:<version>` to drop the .so into
            // the running PHP's extension dir. Once the extension is loaded,
            // `composer install` resolves cleanly and `composer test` runs.
            run: Some(StringOrVec::Single(format!(
                "cd {test_apps_dir}/php && bash install.sh && composer install && composer test"
            ))),
        },
        Language::Elixir => TestAppRunConfig {
            precondition: Some(require_tool("mix")),
            before: None,
            run: Some(StringOrVec::Single(format!(
                "cd {test_apps_dir}/elixir && mix deps.get && mix test"
            ))),
        },
        Language::Go => TestAppRunConfig {
            precondition: Some(require_tool("go")),
            before: None,
            // GOWORK=off prevents a consumer repo's go.work from absorbing
            // test_apps/go into the outer workspace, which would cause `go test
            // ./...` to resolve the module graph via the workspace root and
            // reject the test app's go.mod as a non-member module.
            //
            // `go mod tidy` populates `go.sum` with full module + package
            // checksums from the require directives in `go.mod`. Alef emits a
            // go.mod with the published-module require but no go.sum (the
            // hashes resolve at fetch time, not at manifest emission), and
            // `go test` refuses to proceed without complete checksums.
            // `go mod download` alone only writes `/go.mod` hashes — `tidy`
            // adds the package content hashes that `go test` actually checks.
            // `tidy` is idempotent once the sum is complete.
            run: Some(StringOrVec::Single(format!(
                "cd {test_apps_dir}/go && GOWORK=off go mod tidy && GOWORK=off go test ./..."
            ))),
        },
        Language::Java => TestAppRunConfig {
            precondition: Some(require_tool("mvn")),
            before: None,
            run: Some(StringOrVec::Single(format!("cd {test_apps_dir}/java && mvn -q test"))),
        },
        Language::Csharp => TestAppRunConfig {
            precondition: Some(require_tool("dotnet")),
            before: None,
            run: Some(StringOrVec::Single(format!("cd {test_apps_dir}/csharp && dotnet test"))),
        },
        Language::Kotlin => TestAppRunConfig {
            precondition: Some(require_tool("gradle")),
            before: None,
            run: Some(StringOrVec::Single(format!(
                "cd {test_apps_dir}/kotlin && gradle test --no-daemon"
            ))),
        },
        Language::KotlinAndroid => TestAppRunConfig {
            // The published AAR contains Android-only native binaries — a
            // host JVM cannot load them, so `gradle test` on a workstation
            // without an Android emulator/device fails at runtime. Gate the
            // run on gradle + adb being installed AND at least one device
            // showing up in `adb devices` with state `device`. When the
            // precondition fails, `alef test-apps run` skips gracefully
            // with a warning rather than reporting a spurious test failure.
            precondition: Some(format!(
                "{} && {} && adb devices | grep -q 'device$'",
                require_tool("gradle"),
                require_tool("adb")
            )),
            before: None,
            run: Some(StringOrVec::Single(format!(
                "cd {test_apps_dir}/kotlin_android && gradle test --no-daemon"
            ))),
        },
        Language::Dart => TestAppRunConfig {
            precondition: Some(require_tool("dart")),
            before: None,
            run: Some(StringOrVec::Single(format!(
                "cd {test_apps_dir}/dart && dart pub get && dart test"
            ))),
        },
        Language::Swift => TestAppRunConfig {
            // The Swift test app is emitted under `swift_e2e/` (not `swift/`) so the
            // SwiftPM package identity is distinct from any sibling package — see
            // `src/e2e/codegen/swift.rs` (`output_base = ...join("swift_e2e")`).
            precondition: Some(require_tool("swift")),
            before: None,
            run: Some(StringOrVec::Single(format!(
                "cd {test_apps_dir}/swift_e2e && swift test"
            ))),
        },
        Language::Zig => TestAppRunConfig {
            precondition: Some(require_tool("zig")),
            before: None,
            run: Some(StringOrVec::Single(format!("cd {test_apps_dir}/zig && zig build test"))),
        },
        Language::Gleam => TestAppRunConfig {
            precondition: Some(require_tool("gleam")),
            before: None,
            run: Some(StringOrVec::Single(format!("cd {test_apps_dir}/gleam && gleam test"))),
        },
        Language::R => TestAppRunConfig {
            precondition: Some(require_tool("Rscript")),
            before: None,
            run: Some(StringOrVec::Single(format!(
                "cd {test_apps_dir}/r && Rscript -e \"devtools::test()\""
            ))),
        },
        Language::C => TestAppRunConfig {
            // The C test app is a Makefile-driven harness with a `test` target.
            precondition: Some(require_tool("make")),
            before: None,
            run: Some(StringOrVec::Single(format!("cd {test_apps_dir}/c && make test"))),
        },
        // FFI is the shared native artifact, not a standalone test app; the JNI shim
        // is exercised via the Kotlin/Android app. Neither has its own run command.
        Language::Ffi | Language::Jni => TestAppRunConfig {
            precondition: None,
            before: None,
            run: None,
        },
    }
}

/// Default run config for a registry test-app target that is NOT a [`Language`]
/// enum variant — i.e. a string-only `[e2e].languages` entry. Today those are
/// the Homebrew formula apps: the legacy CLI-only `brew` target (emitted by
/// `BrewCodegen` under `test_apps/brew/`) and the newer combined CLI+FFI
/// `homebrew` target (emitted by `HomebrewCodegen` under `test_apps/homebrew/`).
/// Each `run_tests.sh` installs the published formulas via `brew install` and
/// exercises them. The target name is the subdir name — `name="brew"` cds into
/// `test_apps/brew`, `name="homebrew"` cds into `test_apps/homebrew`. Unknown
/// names get no run.
pub fn default_test_apps_run_config_for_name(name: &str, test_apps_dir: &str, _ctx: &LangContext) -> TestAppRunConfig {
    match name {
        "brew" | "homebrew" => TestAppRunConfig {
            precondition: Some(require_tool("brew")),
            before: None,
            run: Some(StringOrVec::Single(format!(
                "cd {test_apps_dir}/{name} && bash run_tests.sh"
            ))),
        },
        _ => TestAppRunConfig {
            precondition: None,
            before: None,
            run: None,
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
            Language::Rust,
            Language::Kotlin,
            Language::KotlinAndroid,
            Language::Swift,
            Language::Dart,
            Language::Gleam,
            Language::Zig,
            Language::C,
        ]
    }

    fn cfg(lang: Language, dir: &str) -> TestAppRunConfig {
        let tools = ToolsConfig::default();
        let ctx = LangContext::default(&tools);
        default_test_apps_run_config(lang, dir, &ctx)
    }

    #[test]
    fn ffi_and_jni_have_no_run_command() {
        for lang in [Language::Ffi, Language::Jni] {
            let c = cfg(lang, "test_apps");
            assert!(c.run.is_none(), "{lang} should have no run command");
            assert!(c.precondition.is_none(), "{lang} should have no precondition");
        }
    }

    #[test]
    fn runnable_languages_have_run_and_precondition() {
        for lang in all_languages() {
            let c = cfg(lang, "test_apps");
            assert!(c.run.is_some(), "{lang} should have a default run command");
            let pre = c
                .precondition
                .unwrap_or_else(|| panic!("{lang} should have a precondition"));
            assert!(
                pre.starts_with("command -v "),
                "{lang} precondition should gate on a tool"
            );
        }
    }

    #[test]
    fn rust_runs_cargo_test() {
        let c = cfg(Language::Rust, "test_apps");
        let run = c.run.unwrap().commands().join(" ");
        assert_eq!(c.precondition.as_deref(), Some("command -v cargo >/dev/null 2>&1"));
        assert!(run.contains("cd test_apps/rust"), "got: {run}");
        assert!(run.contains("cargo test"), "got: {run}");
    }

    #[test]
    fn python_runs_uv_by_default() {
        let c = cfg(Language::Python, "test_apps");
        let run = c.run.unwrap().commands().join(" ");
        assert_eq!(c.precondition.as_deref(), Some("command -v uv >/dev/null 2>&1"));
        assert!(run.contains("cd test_apps/python"), "got: {run}");
        assert!(run.contains("uv sync"), "got: {run}");
        assert!(run.contains("uv run pytest"), "got: {run}");
    }

    #[test]
    fn python_dispatches_on_package_manager() {
        for (pm, expected_pre, expected_cmd) in [
            ("pip", "command -v pip >/dev/null 2>&1", "pip install -e ."),
            ("poetry", "command -v poetry >/dev/null 2>&1", "poetry install"),
        ] {
            let tools = ToolsConfig {
                python_package_manager: Some(pm.to_string()),
                ..Default::default()
            };
            let ctx = LangContext::default(&tools);
            let c = default_test_apps_run_config(Language::Python, "test_apps", &ctx);
            assert_eq!(c.precondition.as_deref(), Some(expected_pre), "{pm} precondition");
            let run = c.run.unwrap().commands().join(" ");
            assert!(run.contains(expected_cmd), "{pm}: expected {expected_cmd}, got: {run}");
            assert!(run.contains("cd test_apps/python"), "{pm}: got: {run}");
        }
    }

    #[test]
    fn node_runs_pnpm_by_default() {
        let c = cfg(Language::Node, "test_apps");
        let run = c.run.unwrap().commands().join(" ");
        assert_eq!(c.precondition.as_deref(), Some("command -v pnpm >/dev/null 2>&1"));
        assert!(run.contains("cd test_apps/node"), "got: {run}");
        assert!(run.contains("pnpm install"), "got: {run}");
        assert!(run.contains("pnpm test"), "got: {run}");
    }

    #[test]
    fn node_dispatches_on_package_manager() {
        for (pm, expected_pre, expected_cmd) in [
            ("npm", "command -v npm >/dev/null 2>&1", "npm install && npm test"),
            ("yarn", "command -v yarn >/dev/null 2>&1", "yarn install && yarn test"),
        ] {
            let tools = ToolsConfig {
                node_package_manager: Some(pm.to_string()),
                ..Default::default()
            };
            let ctx = LangContext::default(&tools);
            let c = default_test_apps_run_config(Language::Node, "test_apps", &ctx);
            assert_eq!(c.precondition.as_deref(), Some(expected_pre), "{pm} precondition");
            let run = c.run.unwrap().commands().join(" ");
            assert!(run.contains(expected_cmd), "{pm}: expected {expected_cmd}, got: {run}");
        }
    }

    #[test]
    fn ruby_runs_bundle_rspec() {
        let c = cfg(Language::Ruby, "test_apps");
        let run = c.run.unwrap().commands().join(" ");
        assert_eq!(c.precondition.as_deref(), Some("command -v bundle >/dev/null 2>&1"));
        assert!(run.contains("cd test_apps/ruby"), "got: {run}");
        assert!(run.contains("bundle install && bundle exec rspec"), "got: {run}");
    }

    #[test]
    fn php_runs_composer_test() {
        let c = cfg(Language::Php, "test_apps");
        let run = c.run.unwrap().commands().join(" ");
        assert_eq!(c.precondition.as_deref(), Some("command -v composer >/dev/null 2>&1"));
        assert!(run.contains("cd test_apps/php"), "got: {run}");
        assert!(
            run.contains("bash install.sh"),
            "PHP run command must call alef-emitted install.sh (PIE bootstrap) before composer; got: {run}"
        );
        assert!(run.contains("composer install && composer test"), "got: {run}");
    }

    #[test]
    fn elixir_runs_mix_test() {
        let c = cfg(Language::Elixir, "test_apps");
        let run = c.run.unwrap().commands().join(" ");
        assert_eq!(c.precondition.as_deref(), Some("command -v mix >/dev/null 2>&1"));
        assert!(run.contains("cd test_apps/elixir"), "got: {run}");
        assert!(run.contains("mix deps.get && mix test"), "got: {run}");
    }

    #[test]
    fn swift_runs_under_swift_e2e_subdir() {
        let c = cfg(Language::Swift, "test_apps");
        let run = c.run.unwrap().commands().join(" ");
        assert_eq!(c.precondition.as_deref(), Some("command -v swift >/dev/null 2>&1"));
        // The generator emits the Swift app under `swift_e2e/`, not `swift/`.
        assert!(run.contains("cd test_apps/swift_e2e"), "got: {run}");
        assert!(
            !run.contains("cd test_apps/swift "),
            "must not use swift/ subdir, got: {run}"
        );
        assert!(run.contains("swift test"), "got: {run}");
    }

    #[test]
    fn go_runs_go_test_with_gowork_off() {
        let c = cfg(Language::Go, "test_apps");
        let run = c.run.unwrap().commands().join(" ");
        assert!(
            run.contains("GOWORK=off go test ./..."),
            "expected GOWORK=off in go run command, got: {run}"
        );
        assert!(
            run.contains("GOWORK=off go mod tidy"),
            "expected `go mod tidy` to populate go.sum before test, got: {run}"
        );
        assert!(run.contains("cd test_apps/go"), "expected cd test_apps/go, got: {run}");
    }

    #[test]
    fn zig_runs_zig_build_test() {
        let c = cfg(Language::Zig, "test_apps");
        let run = c.run.unwrap().commands().join(" ");
        assert!(run.contains("cd test_apps/zig && zig build test"), "got: {run}");
    }

    #[test]
    fn wasm_runs_under_wasm_subdir() {
        let c = cfg(Language::Wasm, "test_apps");
        let run = c.run.unwrap().commands().join(" ");
        assert!(run.contains("cd test_apps/wasm"), "got: {run}");
        assert!(run.contains("pnpm install && pnpm test"), "got: {run}");
    }

    #[test]
    fn test_apps_dir_is_substituted() {
        let c = cfg(Language::Go, "my/custom/apps");
        let run = c.run.unwrap().commands().join(" ");
        assert!(run.contains("cd my/custom/apps/go"), "got: {run}");
    }

    #[test]
    fn brew_target_runs_under_brew_subdir() {
        let tools = ToolsConfig::default();
        let ctx = LangContext::default(&tools);
        let c = default_test_apps_run_config_for_name("brew", "test_apps", &ctx);
        let run = c.run.expect("brew should have a run command").commands().join(" ");
        assert_eq!(c.precondition.as_deref(), Some("command -v brew >/dev/null 2>&1"));
        assert!(run.contains("cd test_apps/brew && bash run_tests.sh"), "got: {run}");
    }

    #[test]
    fn homebrew_target_runs_under_homebrew_subdir() {
        // The newer combined CLI+FFI Homebrew target (HomebrewCodegen) writes to
        // `test_apps/homebrew/`, not `test_apps/brew/`. The default run command
        // must cd into the matching subdir.
        let tools = ToolsConfig::default();
        let ctx = LangContext::default(&tools);
        let c = default_test_apps_run_config_for_name("homebrew", "test_apps", &ctx);
        let run = c.run.expect("homebrew should have a run command").commands().join(" ");
        assert_eq!(c.precondition.as_deref(), Some("command -v brew >/dev/null 2>&1"));
        assert!(run.contains("cd test_apps/homebrew && bash run_tests.sh"), "got: {run}");
        assert!(
            !run.contains("cd test_apps/brew "),
            "must not use brew/ subdir for homebrew target, got: {run}"
        );
    }

    #[test]
    fn kotlin_android_runs_under_kotlin_android_subdir() {
        let c = cfg(Language::KotlinAndroid, "test_apps");
        let run = c.run.unwrap().commands().join(" ");
        assert!(run.contains("cd test_apps/kotlin_android"), "got: {run}");
        assert!(
            !run.contains("cd test_apps/kotlin "),
            "must use kotlin_android/ subdir, not kotlin/, got: {run}"
        );
        assert!(run.contains("gradle test --no-daemon"), "got: {run}");
    }
}
