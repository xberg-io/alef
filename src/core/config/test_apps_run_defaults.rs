use super::extras::Language;
use super::output::{ArgvRunConfig, ArgvStep, StringOrVec, TestAppRunConfig};
use super::shell::quote_word;
use super::tools::{LangContext, require_ruby_bundler, require_tool, ruby_bundle, ruby_bundle_exec};

#[cfg(all(test, unix))]
mod shell_safety_tests;

/// Strip a leading package-manager version-constraint prefix (`^`, `~`, `>`,
/// `<`, `=`) from a version string, returning the bare version. A concrete
/// installer tag (e.g. PIE's `pie install pkg:<version>`) must not carry a
/// constraint operator.
fn strip_version_constraint(version: &str) -> &str {
    version.trim_start_matches(['^', '~', '>', '<', '='])
}

/// Return the default test-app run configuration for a language.
///
/// `test_apps_dir` is the registry-mode output directory (e.g. `test_apps`); the
/// per-language test app lives at `{test_apps_dir}/<lang-subdir>`, where the
/// subdir matches exactly what the test-apps generator (`src/e2e/codegen`) writes
/// for that language — usually the language name, but `swift` is emitted under
/// `swift_e2e` to give the SwiftPM package a distinct identity. `ctx` provides the
/// package-manager selection. `published_version` is the published package version
/// for this language (when known); some run commands need to forward it to a
/// generated installer script. `go_module_path` is the Go module path used for
/// vendoring cgo-linked native libraries (Go language only). Executed by `alef test-apps run`
/// to install the published package into the test app and exercise it.
pub fn default_test_apps_run_config(
    lang: Language,
    test_apps_dir: &str,
    ctx: &LangContext,
    published_version: Option<&str>,
    go_module_path: Option<&str>,
) -> TestAppRunConfig {
    // `test_apps_dir` is `[crates.e2e.registry].output` -- a free-form, user-authored config
    // value with no syntax restrictions, and the Rust side already treats it as a literal path
    // (`base_dir.join(&e2e.registry.output)` in `cli::pipeline::commands::test_apps`). Every
    // default below that still needs a shell (a `&&` chain, a heredoc, a command substitution)
    // splices it into `cd <dir>/<lang>`, so it must arrive as exactly one literal shell word:
    // unquoted, a `;`, backtick, or `$(...)` inside `output` executed arbitrary commands during
    // `alef test-apps run`. Defaults that need no shell at all (Go, PHP, brew/homebrew) pass the
    // raw value as an `ArgvRunConfig::work_dir` instead, where `Command::current_dir` takes it as
    // an opaque path -- quoting is the fallback for the arms a shell is genuinely required for,
    // not the goal. ~keep
    let dir = quote_word(test_apps_dir);
    match lang {
        Language::Rust => TestAppRunConfig {
            precondition: Some(require_tool("cargo")),
            before: None,
            run: Some(StringOrVec::Single(format!("cd {dir}/rust && cargo test"))),
            argv_run: None,
        },
        Language::Python => {
            let pm = ctx.tools.python_pm();
            let run = match pm {
                "pip" => format!("cd {dir}/python && pip install -e . && pytest"),
                "poetry" => format!("cd {dir}/python && poetry install && poetry run pytest"),
                _ => format!("cd {dir}/python && uv sync && uv run pytest"),
            };
            TestAppRunConfig {
                precondition: Some(require_tool(pm)),
                before: None,
                run: Some(StringOrVec::Single(run)),
                argv_run: None,
            }
        }
        Language::Node => {
            let pm = ctx.tools.node_pm();
            let run = match pm {
                "npm" => format!("cd {dir}/node && pnpm add --no-package-lock && pnpm test"),
                "yarn" => format!("cd {dir}/node && yarn install && yarn test"),
                _ => format!(
                    "cd {dir}/node && pnpm install --no-frozen-lockfile --config.minimumReleaseAge=0 && pnpm --config.minimumReleaseAge=0 test"
                ),
            };
            TestAppRunConfig {
                precondition: Some(require_tool(pm)),
                before: None,
                run: Some(StringOrVec::Single(run)),
                argv_run: None,
            }
        }
        Language::Wasm => {
            let pm = ctx.tools.node_pm();
            let run = match pm {
                "npm" => format!("cd {dir}/wasm && pnpm add --no-package-lock && pnpm test"),
                "yarn" => format!("cd {dir}/wasm && yarn install && yarn test"),
                _ => format!(
                    "cd {dir}/wasm && pnpm install --no-frozen-lockfile --config.minimumReleaseAge=0 && pnpm --config.minimumReleaseAge=0 test"
                ),
            };
            TestAppRunConfig {
                precondition: Some(require_tool(pm)),
                before: None,
                run: Some(StringOrVec::Single(run)),
                argv_run: None,
            }
        }
        Language::Ruby => TestAppRunConfig {
            precondition: Some(require_ruby_bundler()),
            before: None,
            run: Some(StringOrVec::Single(format!(
                "cd {dir}/ruby && {} && {}",
                ruby_bundle("install"),
                ruby_bundle_exec("rspec")
            ))),
            argv_run: None,
        },
        Language::Php => {
            // `published_version` is `[crates.e2e.registry.packages.php].version` (falling back
            // to the crate's resolved version) -- a free-form config value with no syntax
            // restrictions. It used to be spliced unquoted into `bash install.sh {version}`, so
            // `;`, backticks, or `$(...)` inside it executed arbitrary commands during `alef
            // test-apps run`. It is now one literal argv element handed to `install.sh`, and
            // `test_apps_dir` is the argv work_dir, so neither value reaches a shell at all.
            // The three steps replace a `&&` chain exactly: `run_test_app_target` stops at the
            // first step that exits non-zero. ~keep
            let mut install_args = vec!["install.sh".to_owned()];
            if let Some(version) = published_version
                .map(strip_version_constraint)
                .filter(|v| !v.is_empty())
            {
                install_args.push(version.to_owned());
            }
            TestAppRunConfig {
                precondition: Some(require_tool("composer")),
                before: None,
                run: None,
                argv_run: Some(ArgvRunConfig {
                    work_dir: format!("{test_apps_dir}/php"),
                    env: Vec::new(),
                    steps: vec![
                        ArgvStep {
                            command: "bash".to_owned(),
                            args: install_args,
                        },
                        ArgvStep {
                            command: "composer".to_owned(),
                            args: vec!["install".to_owned()],
                        },
                        ArgvStep {
                            command: "composer".to_owned(),
                            args: vec!["test".to_owned()],
                        },
                    ],
                }),
            }
        }
        Language::Elixir => TestAppRunConfig {
            precondition: Some(require_tool("mix")),
            before: None,
            run: Some(StringOrVec::Single(format!(
                "cd {dir}/elixir && mix deps.get && mix test"
            ))),
            argv_run: None,
        },
        Language::Go => {
            // `cmd/setup` downloads the platform native library from the GitHub release ~keep
            // into a per-user cache and writes a machine-local cgo link shim into the test ~keep
            // app's own package — `go run <module>/cmd/setup` works directly against the ~keep
            // module fetched from the proxy (or replaced locally), no copy-out-of-the ~keep
            // read-only-module-cache workaround needed. ~keep
            //
            // `go_module_path` is `[go] module` -- a free-form, user-authored value with no
            // syntax restrictions, so it must never be interpolated into shell text again (it
            // used to be, via an unquoted `format!`, and that let `;`, backticks, or `$(...)`
            // in the module path execute arbitrary commands during `alef test-apps run`).
            // Every step below runs as literal argv via `ArgvStep` instead: the module path is
            // passed as a single opaque argument to `go run`, so a shell never gets a chance to
            // reinterpret it. ~keep
            let mut steps = vec![ArgvStep {
                command: "go".to_owned(),
                args: vec!["mod".to_owned(), "tidy".to_owned()],
            }];
            if let Some(mod_path) = go_module_path {
                steps.push(ArgvStep {
                    command: "go".to_owned(),
                    args: vec!["run".to_owned(), format!("{mod_path}/cmd/setup")],
                });
            }
            steps.push(ArgvStep {
                command: "go".to_owned(),
                args: vec!["test".to_owned(), "./...".to_owned()],
            });
            TestAppRunConfig {
                precondition: Some(require_tool("go")),
                before: None,
                run: None,
                argv_run: Some(ArgvRunConfig {
                    work_dir: format!("{test_apps_dir}/go"),
                    env: vec![("GOWORK".to_owned(), "off".to_owned())],
                    steps,
                }),
            }
        }
        Language::Java => TestAppRunConfig {
            precondition: Some(require_tool("mvn")),
            before: None,
            run: Some(StringOrVec::Single(format!(
                "cd {dir}/java && mvn --batch-mode --no-transfer-progress test"
            ))),
            argv_run: None,
        },
        Language::Csharp => TestAppRunConfig {
            precondition: Some(require_tool("dotnet")),
            before: None,
            run: Some(StringOrVec::Single(format!("cd {dir}/csharp && dotnet test"))),
            argv_run: None,
        },
        Language::Kotlin => TestAppRunConfig {
            precondition: Some(require_tool("gradle")),
            before: None,
            run: Some(StringOrVec::Single(format!(
                "cd {dir}/kotlin && gradle test --no-daemon"
            ))),
            argv_run: None,
        },
        Language::KotlinAndroid => TestAppRunConfig {
            precondition: Some(require_tool("gradle")),
            before: None,
            run: Some(StringOrVec::Single(format!(
                "cd {dir}/kotlin_android && gradle test --no-daemon"
            ))),
            argv_run: None,
        },
        Language::Dart => TestAppRunConfig {
            // Pub.dev cannot ship the native libraries inside the Dart package — the full ~keep
            // per-platform native set exceeds pub.dev's 100 MB tarball cap — so the published ~keep
            // package ships a `bin/download_libs.dart` helper (exposed as an `executables:` ~keep
            // entry in its pubspec.yaml) that fetches the platform-specific dylib/so/dll from ~keep
            // the GitHub release into the pub-cache's `lib/src/native/<rid>/` directory, where ~keep
            // the FRB loader resolves it. Without invoking that executable between `pub get` ~keep
            // and `dart test`, `RustLib.init()` fails in `setUpAll` with ~keep
            // "Native library for <pkg> (<os>-<arch>) was not found ... Download it with ~keep
            // `dart run <pkg>:download_libs`". ~keep
            // ~keep
            // Extract the under-test package name (the first `dependencies:` entry in the ~keep
            // test_app pubspec.yaml, per alef's test_apps codegen convention) and invoke ~keep
            // `dart run <pkg>:download_libs`. Keep stderr attached so a real failure ~keep
            // (HTTP 404, network, asset-name mismatch) surfaces here rather than as a ~keep
            // confusing `dlopen` rejection inside `dart test`. ~keep
            precondition: Some(require_tool("dart")),
            before: None,
            run: Some(StringOrVec::Single(format!(
                "cd {dir}/dart && \
                dart pub get && \
                DART_PKG=$(awk '/^dependencies:$/{{f=1;next}} f && /^  [a-z]/{{sub(/:.*/,\"\");sub(/^  /,\"\");print;exit}}' pubspec.yaml) && \
                dart run \"${{DART_PKG}}:download_libs\" && \
                dart test"
            ))),
            argv_run: None,
        },
        Language::Swift => TestAppRunConfig {
            precondition: Some(require_tool("swift")),
            before: None,
            run: Some(StringOrVec::Single(format!("cd {dir}/swift_e2e && swift test"))),
            argv_run: None,
        },
        Language::Zig => TestAppRunConfig {
            precondition: Some(require_tool("zig")),
            before: None,
            run: Some(StringOrVec::Single(format!(
                r#"cd {dir}/zig && rm -rf zig-pkg .zig-cache && python3 - <<'PYEOF'
import pathlib, re, subprocess
zon = pathlib.Path('build.zig.zon')
content = zon.read_text()
# Strip any pre-existing `.hash` lines (e.g. the STALE placeholder emitted from
# `[crates.e2e.registry.packages.zig].hash`). We recompute every dependency hash
# from its published tarball below; leaving the placeholder in place would yield
# two `.hash` keys per dep and zig honors the last (stale) one, breaking fetch.
content = re.sub(r'\n[ \t]*\.hash\s*=\s*"[^"]*",', '', content)
deps = re.findall(r'\.([a-z_0-9]+)\s*=\s*\.\{{[^}}]*?\.url\s*=\s*"([^"]+)"', content, re.DOTALL)
for name, url in deps:
    h = subprocess.run(['zig', 'fetch', url], capture_output=True, text=True, check=True).stdout.strip()
    pat = re.compile(r'(\.' + re.escape(name) + r'\s*=\s*\.\{{[^}}]*?\.url\s*=\s*"' + re.escape(url) + r'",)(\s*\n)(\s*)', re.DOTALL)
    content = pat.sub(lambda m: m.group(1) + m.group(2) + m.group(3) + '.hash = "' + h + '",\n' + m.group(3), content, count=1)
zon.write_text(content)
PYEOF
zig build test"#
            ))),
            argv_run: None,
        },
        Language::Gleam => TestAppRunConfig {
            precondition: Some(require_tool("gleam")),
            before: None,
            run: Some(StringOrVec::Single(format!("cd {dir}/gleam && gleam test"))),
            argv_run: None,
        },
        Language::R => TestAppRunConfig {
            precondition: Some(require_tool("Rscript")),
            before: None,
            run: Some(StringOrVec::Single(format!(
                "cd {dir}/r && Rscript -e \"devtools::test()\""
            ))),
            argv_run: None,
        },
        Language::C => TestAppRunConfig {
            precondition: Some(require_tool("make")),
            before: None,
            run: Some(StringOrVec::Single(format!("cd {dir}/c && make test"))),
            argv_run: None,
        },
        Language::Ffi => TestAppRunConfig {
            precondition: None,
            before: None,
            run: None,
            argv_run: None,
        },
        Language::Jni => TestAppRunConfig {
            precondition: Some(require_tool("gradle")),
            before: None,
            run: Some(StringOrVec::Single(format!(
                "cd {dir}/kotlin_android && gradle test --no-daemon"
            ))),
            argv_run: None,
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
        // `test_apps_dir` is free-form user config; `bash run_tests.sh` needs no shell features
        // of its own, so the whole step runs as argv with the directory as the work_dir --
        // `Command::current_dir` takes it as an opaque path and no shell ever parses it. ~keep
        "brew" | "homebrew" => TestAppRunConfig {
            precondition: Some(require_tool("brew")),
            before: None,
            run: None,
            argv_run: Some(ArgvRunConfig {
                work_dir: format!("{test_apps_dir}/{name}"),
                env: Vec::new(),
                steps: vec![ArgvStep {
                    command: "bash".to_owned(),
                    args: vec!["run_tests.sh".to_owned()],
                }],
            }),
        },
        _ => TestAppRunConfig {
            precondition: None,
            before: None,
            run: None,
            argv_run: None,
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
        default_test_apps_run_config(lang, dir, &ctx, None, None)
    }

    #[test]
    fn ffi_has_no_run_command() {
        let c = cfg(Language::Ffi, "test_apps");
        assert!(c.run.is_none(), "FFI should have no run command");
        assert!(c.precondition.is_none(), "FFI should have no precondition");
    }

    #[test]
    fn jni_runs_kotlin_android_host_jvm_tests() {
        let c = cfg(Language::Jni, "test_apps");
        let run = c.run.expect("JNI should have a run command");
        let cmd = match run {
            StringOrVec::Single(s) => s,
            _ => panic!("JNI run should be a single command"),
        };
        assert!(
            cmd.contains("kotlin_android"),
            "JNI should run via kotlin_android: {cmd}"
        );
        assert!(cmd.contains("gradle test"), "JNI should run gradle test: {cmd}");
        assert!(c.precondition.is_some(), "JNI should require gradle");
    }

    #[test]
    fn runnable_languages_have_run_and_precondition() {
        for lang in all_languages() {
            let c = cfg(lang, "test_apps");
            assert!(
                c.run.is_some() || c.argv_run.is_some(),
                "{lang} should have a default run command"
            );
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
        assert!(run.contains("cd 'test_apps'/rust"), "got: {run}");
        assert!(run.contains("cargo test"), "got: {run}");
    }

    #[test]
    fn python_runs_uv_by_default() {
        let c = cfg(Language::Python, "test_apps");
        let run = c.run.unwrap().commands().join(" ");
        assert_eq!(c.precondition.as_deref(), Some("command -v uv >/dev/null 2>&1"));
        assert!(run.contains("cd 'test_apps'/python"), "got: {run}");
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
            let c = default_test_apps_run_config(Language::Python, "test_apps", &ctx, None, None);
            assert_eq!(c.precondition.as_deref(), Some(expected_pre), "{pm} precondition");
            let run = c.run.unwrap().commands().join(" ");
            assert!(run.contains(expected_cmd), "{pm}: expected {expected_cmd}, got: {run}");
            assert!(run.contains("cd 'test_apps'/python"), "{pm}: got: {run}");
        }
    }

    #[test]
    fn node_runs_pnpm_by_default() {
        let c = cfg(Language::Node, "test_apps");
        let run = c.run.unwrap().commands().join(" ");
        assert_eq!(c.precondition.as_deref(), Some("command -v pnpm >/dev/null 2>&1"));
        assert!(run.contains("cd 'test_apps'/node"), "got: {run}");
        assert!(
            run.contains("pnpm install --no-frozen-lockfile --config.minimumReleaseAge=0"),
            "got: {run}"
        );
        assert!(
            run.contains("pnpm --config.minimumReleaseAge=0 test"),
            "pnpm test must pass minimumReleaseAge=0 for pnpm 11.3+ compatibility; got: {run}"
        );
    }

    #[test]
    fn node_dispatches_on_package_manager() {
        for (pm, expected_pre, expected_cmd) in [
            (
                "npm",
                "command -v npm >/dev/null 2>&1",
                "pnpm add --no-package-lock && pnpm test",
            ),
            ("yarn", "command -v yarn >/dev/null 2>&1", "yarn install && yarn test"),
        ] {
            let tools = ToolsConfig {
                node_package_manager: Some(pm.to_string()),
                ..Default::default()
            };
            let ctx = LangContext::default(&tools);
            let c = default_test_apps_run_config(Language::Node, "test_apps", &ctx, None, None);
            assert_eq!(c.precondition.as_deref(), Some(expected_pre), "{pm} precondition");
            let run = c.run.unwrap().commands().join(" ");
            assert!(run.contains(expected_cmd), "{pm}: expected {expected_cmd}, got: {run}");
        }
    }

    #[test]
    fn ruby_runs_bundle_rspec() {
        let c = cfg(Language::Ruby, "test_apps");
        let run = c.run.unwrap().commands().join(" ");
        assert_eq!(
            c.precondition.as_deref(),
            Some(
                "command -v ruby >/dev/null 2>&1 && BUNDLE_PATH=vendor/bundle ruby -S bundle --version >/dev/null 2>&1"
            )
        );
        assert!(run.contains("cd 'test_apps'/ruby"), "got: {run}");
        assert!(
            run.contains(
                "BUNDLE_PATH=vendor/bundle ruby -S bundle install && BUNDLE_PATH=vendor/bundle ruby -S bundle exec ruby -S rspec"
            ),
            "got: {run}"
        );
    }

    #[test]
    fn php_runs_composer_test() {
        let c = cfg(Language::Php, "test_apps");
        assert_eq!(c.precondition.as_deref(), Some("command -v composer >/dev/null 2>&1"));
        assert!(
            c.run.is_none(),
            "php's default run must be argv-only, not a shell string: {:?}",
            c.run
        );
        let argv = c.argv_run.expect("php should have an argv run command");
        assert_eq!(argv.work_dir, "test_apps/php");
        assert_eq!(argv.steps.len(), 3, "install.sh, composer install, composer test");
        assert_eq!(argv.steps[0].command, "bash");
        assert_eq!(
            argv.steps[0].args,
            vec!["install.sh"],
            "PHP must call the alef-emitted install.sh (PIE bootstrap) before composer"
        );
        assert_eq!(argv.steps[1].command, "composer");
        assert_eq!(argv.steps[1].args, vec!["install"]);
        assert_eq!(argv.steps[2].command, "composer");
        assert_eq!(argv.steps[2].args, vec!["test"]);
    }

    #[test]
    fn php_forwards_the_published_version_as_one_literal_argument() {
        let tools = ToolsConfig::default();
        let ctx = LangContext::default(&tools);
        let c = default_test_apps_run_config(Language::Php, "test_apps", &ctx, Some("^1.2.3"), None);
        let argv = c.argv_run.expect("php should have an argv run command");
        assert_eq!(
            argv.steps[0].args,
            vec!["install.sh", "1.2.3"],
            "the constraint prefix is stripped and the bare version is a separate argv element"
        );
    }

    #[test]
    fn elixir_runs_mix_test() {
        let c = cfg(Language::Elixir, "test_apps");
        let run = c.run.unwrap().commands().join(" ");
        assert_eq!(c.precondition.as_deref(), Some("command -v mix >/dev/null 2>&1"));
        assert!(run.contains("cd 'test_apps'/elixir"), "got: {run}");
        assert!(run.contains("mix deps.get && mix test"), "got: {run}");
    }

    #[test]
    fn swift_runs_under_swift_e2e_subdir() {
        let c = cfg(Language::Swift, "test_apps");
        let run = c.run.unwrap().commands().join(" ");
        assert_eq!(c.precondition.as_deref(), Some("command -v swift >/dev/null 2>&1"));
        assert!(run.contains("cd 'test_apps'/swift_e2e"), "got: {run}");
        assert!(
            !run.contains("cd 'test_apps'/swift "),
            "must not use swift/ subdir, got: {run}"
        );
        assert!(run.contains("swift test"), "got: {run}");
        assert!(
            !run.contains("download_swift_artifact"),
            "swift run command must not invoke the legacy artifact-bundle download script, got: {run}"
        );
    }

    #[test]
    fn go_runs_go_test_with_gowork_off() {
        let c = cfg(Language::Go, "test_apps");
        assert!(
            c.run.is_none(),
            "go's default run must be argv-only, not a shell string: {:?}",
            c.run
        );
        let argv = c.argv_run.expect("go should have an argv run command");
        assert_eq!(argv.work_dir, "test_apps/go");
        assert_eq!(argv.env, vec![("GOWORK".to_owned(), "off".to_owned())]);
        assert_eq!(argv.steps.len(), 2, "no module path -> tidy, test");
        assert_eq!(argv.steps[0].command, "go");
        assert_eq!(argv.steps[0].args, vec!["mod", "tidy"]);
        assert_eq!(argv.steps[1].command, "go");
        assert_eq!(argv.steps[1].args, vec!["test", "./..."]);
    }

    #[test]
    fn go_with_module_path_provisions_ffi_via_cmd_setup() {
        let c = default_test_apps_run_config(
            Language::Go,
            "test_apps",
            &LangContext::default(&ToolsConfig::default()),
            None,
            Some("github.com/example/mylib/packages/go"),
        );
        assert!(c.run.is_none(), "go's default run must be argv-only: {:?}", c.run);
        let argv = c.argv_run.expect("go should have an argv run command");
        assert_eq!(argv.work_dir, "test_apps/go");
        assert_eq!(argv.steps.len(), 3, "with a module path -> tidy, run cmd/setup, test");
        assert_eq!(argv.steps[0].command, "go");
        assert_eq!(argv.steps[0].args, vec!["mod", "tidy"]);
        assert_eq!(argv.steps[1].command, "go");
        assert_eq!(
            argv.steps[1].args,
            vec!["run", "github.com/example/mylib/packages/go/cmd/setup"],
            "cmd/setup must be invoked directly against the module path, as one literal argument"
        );
        assert_eq!(argv.steps[2].command, "go");
        assert_eq!(argv.steps[2].args, vec!["test", "./..."]);
    }

    /// RED (pre-fix)/GREEN (post-fix): `[go] module` is a free-form, user-authored value with
    /// no syntax restrictions. It used to be spliced unquoted into a `format!("cd {dir} &&
    /// GOWORK=off go mod tidy && GOWORK=off go run {mod_path}/cmd/setup && ...")` shell string,
    /// so `;`, backticks, or `$(...)` in the module path executed arbitrary commands during
    /// `alef test-apps run`. It must now survive as a single literal argv element: shell
    /// metacharacters inside it must never be split, joined, or reinterpreted.
    #[test]
    fn go_module_path_survives_shell_metacharacters_as_a_single_argument() {
        let malicious = "github.com/example/mylib; touch pwned; echo";
        let c = default_test_apps_run_config(
            Language::Go,
            "test_apps",
            &LangContext::default(&ToolsConfig::default()),
            None,
            Some(malicious),
        );
        assert!(
            c.run.is_none(),
            "must be argv-only, not a shell string that could reinterpret this payload"
        );
        let argv = c.argv_run.expect("go should have an argv run command");
        let setup_step = &argv.steps[1];
        assert_eq!(setup_step.command, "go");
        assert_eq!(
            setup_step.args,
            vec!["run".to_owned(), format!("{malicious}/cmd/setup")],
            "the entire payload, including `;`, must arrive as one literal argument"
        );
    }

    #[test]
    fn zig_runs_zig_build_test() {
        let c = cfg(Language::Zig, "test_apps");
        let run = c.run.unwrap().commands().join(" ");
        assert!(run.contains("cd 'test_apps'/zig"), "got: {run}");
        assert!(run.contains("python3"), "got: {run}");
        assert!(run.contains("'zig', 'fetch'"), "got: {run}");
        assert!(run.contains("zig build test"), "got: {run}");
        let python_idx = run.find("python3").unwrap();
        let build_idx = run.find("zig build test").unwrap();
        assert!(
            python_idx < build_idx,
            "python3 hash-population must run before zig build test, got: {run}"
        );
    }

    #[test]
    fn wasm_runs_under_wasm_subdir() {
        let c = cfg(Language::Wasm, "test_apps");
        let run = c.run.unwrap().commands().join(" ");
        assert!(run.contains("cd 'test_apps'/wasm"), "got: {run}");
        assert!(
            run.contains("pnpm install --no-frozen-lockfile --config.minimumReleaseAge=0"),
            "got: {run}"
        );
        assert!(
            run.contains("pnpm --config.minimumReleaseAge=0 test"),
            "pnpm test must also pass minimumReleaseAge=0 flag for pnpm 11.3+ compatibility; got: {run}"
        );
    }

    #[test]
    fn test_apps_dir_is_substituted() {
        let c = cfg(Language::Go, "my/custom/apps");
        let argv = c.argv_run.expect("go should have an argv run command");
        assert_eq!(argv.work_dir, "my/custom/apps/go");
    }

    #[test]
    fn brew_target_runs_under_brew_subdir() {
        let tools = ToolsConfig::default();
        let ctx = LangContext::default(&tools);
        let c = default_test_apps_run_config_for_name("brew", "test_apps", &ctx);
        assert_eq!(c.precondition.as_deref(), Some("command -v brew >/dev/null 2>&1"));
        assert!(c.run.is_none(), "brew's default run must be argv-only: {:?}", c.run);
        let argv = c.argv_run.expect("brew should have an argv run command");
        assert_eq!(argv.work_dir, "test_apps/brew");
        assert_eq!(argv.steps.len(), 1);
        assert_eq!(argv.steps[0].command, "bash");
        assert_eq!(argv.steps[0].args, vec!["run_tests.sh"]);
    }

    #[test]
    fn homebrew_target_runs_under_homebrew_subdir() {
        let tools = ToolsConfig::default();
        let ctx = LangContext::default(&tools);
        let c = default_test_apps_run_config_for_name("homebrew", "test_apps", &ctx);
        assert_eq!(c.precondition.as_deref(), Some("command -v brew >/dev/null 2>&1"));
        assert!(c.run.is_none(), "homebrew's default run must be argv-only: {:?}", c.run);
        let argv = c.argv_run.expect("homebrew should have an argv run command");
        assert_eq!(
            argv.work_dir, "test_apps/homebrew",
            "must not use the brew/ subdir for the homebrew target"
        );
        assert_eq!(argv.steps.len(), 1);
        assert_eq!(argv.steps[0].command, "bash");
        assert_eq!(argv.steps[0].args, vec!["run_tests.sh"]);
    }

    #[test]
    fn dart_run_invokes_download_libs_before_test() {
        // Pub.dev cannot bundle the full per-platform native set (exceeds the 100 MB tarball ~keep
        // cap), so the published package ships a `download_libs` executable that fetches the ~keep
        // native from the GitHub release. Without invoking it between `pub get` and `dart test`, ~keep
        // `RustLib.init()` fails in setUpAll with "Native library ... was not found". The ~keep
        // default command must derive the package name and run `dart run <pkg>:download_libs`. ~keep
        let c = cfg(Language::Dart, "test_apps");
        let run = c.run.expect("dart should have a run command").commands().join(" ");
        assert_eq!(c.precondition.as_deref(), Some("command -v dart >/dev/null 2>&1"));
        assert!(run.contains("cd 'test_apps'/dart"), "got: {run}");
        assert!(run.contains("dart pub get"), "got: {run}");
        assert!(run.contains("dart test"), "got: {run}");
        assert!(
            run.contains("download_libs"),
            "dart run must invoke download_libs to fetch the native from the GH release; got: {run}"
        );
        assert!(
            run.contains("DART_PKG"),
            "dart run must derive the under-test package name for the download_libs call; got: {run}"
        );
        // download_libs must run before `dart test` so the native is present at init time. ~keep
        let dl = run.find("download_libs").expect("download_libs present");
        let test = run.rfind("dart test").expect("dart test present");
        assert!(dl < test, "download_libs must run before dart test; got: {run}");
    }

    #[test]
    fn kotlin_android_runs_under_kotlin_android_subdir() {
        let c = cfg(Language::KotlinAndroid, "test_apps");
        let run = c.run.unwrap().commands().join(" ");
        assert!(run.contains("cd 'test_apps'/kotlin_android"), "got: {run}");
        assert!(
            !run.contains("cd 'test_apps'/kotlin "),
            "must use kotlin_android/ subdir, not kotlin/, got: {run}"
        );
        assert!(run.contains("gradle test --no-daemon"), "got: {run}");
    }
}
