use super::extras::Language;
use super::output::{StringOrVec, TestAppRunConfig};
use super::tools::{LangContext, require_tool};

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
    match lang {
        Language::Rust => TestAppRunConfig {
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
                "npm" => format!("cd {test_apps_dir}/node && npm install --no-package-lock && npm test"),
                "yarn" => format!("cd {test_apps_dir}/node && yarn install && yarn test"),
                _ => format!(
                    "cd {test_apps_dir}/node && pnpm install --no-frozen-lockfile --config.minimumReleaseAge=0 && pnpm --config.minimumReleaseAge=0 test"
                ),
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
                "npm" => format!("cd {test_apps_dir}/wasm && npm install --no-package-lock && npm test"),
                "yarn" => format!("cd {test_apps_dir}/wasm && yarn install && yarn test"),
                _ => format!(
                    "cd {test_apps_dir}/wasm && pnpm install --no-frozen-lockfile --config.minimumReleaseAge=0 && pnpm --config.minimumReleaseAge=0 test"
                ),
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
        Language::Php => {
            let version_arg = published_version
                .map(strip_version_constraint)
                .filter(|v| !v.is_empty())
                .map(|v| format!(" {v}"))
                .unwrap_or_default();
            TestAppRunConfig {
                precondition: Some(require_tool("composer")),
                before: None,
                run: Some(StringOrVec::Single(format!(
                    "cd {test_apps_dir}/php && bash install.sh{version_arg} && composer install && composer test"
                ))),
            }
        }
        Language::Elixir => TestAppRunConfig {
            precondition: Some(require_tool("mix")),
            before: None,
            run: Some(StringOrVec::Single(format!(
                "cd {test_apps_dir}/elixir && mix deps.get && mix test"
            ))),
        },
        Language::Go => {
            // download logic. The tool ships behind a `//go:build ignore` guard so it
            let run_cmd = if let Some(mod_path) = go_module_path {
                format!(
                    r#"cd {test_apps_dir}/go && GOWORK=off go mod tidy && \
{{ \
  SRC="$(GOWORK=off go list -m -f '{{{{.Dir}}}}' {mod_path})" && \
  DST="$(mktemp -d)/gomod" && \
  cp -R "$SRC" "$DST" && \
  chmod -R u+w "$DST" && \
  perl -ni -e 'print unless m{{^//go:build ignore$}} || m{{^//\s*\+build ignore$}}' "$DST/cmd/download_ffi/main.go" && \
  ( cd "$DST" && GOWORK=off GOFLAGS=-mod=mod go run ./cmd/download_ffi ) && \
  GOWORK=off go mod edit -replace {mod_path}="$DST" && \
  GOWORK=off go mod tidy && \
  GOWORK=off go test ./...; rc=$?; git checkout go.mod go.sum; exit $rc; \
}}"#
                )
            } else {
                format!("cd {test_apps_dir}/go && GOWORK=off go mod tidy && GOWORK=off go test ./...")
            };
            TestAppRunConfig {
                precondition: Some(require_tool("go")),
                before: None,
                run: Some(StringOrVec::Single(run_cmd)),
            }
        }
        Language::Java => TestAppRunConfig {
            precondition: Some(require_tool("mvn")),
            before: None,
            run: Some(StringOrVec::Single(format!(
                "cd {test_apps_dir}/java && mvn --batch-mode --no-transfer-progress test"
            ))),
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
            precondition: Some(require_tool("gradle")),
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
            precondition: Some(require_tool("swift")),
            before: None,
            run: Some(StringOrVec::Single(format!(
                "cd {test_apps_dir}/swift_e2e && swift test"
            ))),
        },
        Language::Zig => TestAppRunConfig {
            precondition: Some(require_tool("zig")),
            before: None,
            run: Some(StringOrVec::Single(format!(
                r#"cd {test_apps_dir}/zig && rm -rf zig-pkg .zig-cache && python3 - <<'PYEOF'
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
            precondition: Some(require_tool("make")),
            before: None,
            run: Some(StringOrVec::Single(format!("cd {test_apps_dir}/c && make test"))),
        },
        Language::Ffi => TestAppRunConfig {
            precondition: None,
            before: None,
            run: None,
        },
        Language::Jni => TestAppRunConfig {
            precondition: Some(require_tool("gradle")),
            before: None,
            run: Some(StringOrVec::Single(format!(
                "cd {test_apps_dir}/kotlin_android && gradle test --no-daemon"
            ))),
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
            let c = default_test_apps_run_config(Language::Python, "test_apps", &ctx, None, None);
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
                "npm install --no-package-lock && npm test",
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
        assert!(run.contains("cd test_apps/swift_e2e"), "got: {run}");
        assert!(
            !run.contains("cd test_apps/swift "),
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
        let run = c.run.unwrap().commands().join(" ");
        assert!(
            run.contains("GOWORK=off go test"),
            "expected GOWORK=off in go run command, got: {run}"
        );
        assert!(
            run.contains("GOWORK=off go mod tidy"),
            "expected `go mod tidy` to populate go.sum before test, got: {run}"
        );
        assert!(run.contains("cd test_apps/go"), "expected cd test_apps/go, got: {run}");
    }

    #[test]
    fn go_with_module_path_provisions_ffi_via_binding_downloader() {
        let c = default_test_apps_run_config(
            Language::Go,
            "test_apps",
            &LangContext::default(&ToolsConfig::default()),
            None,
            Some("github.com/example/mylib/packages/go"),
        );
        let run = c.run.unwrap().commands().join(" ");
        assert!(
            !run.contains("go mod vendor") && !run.contains("-mod=vendor"),
            "must not use vendor mode, got: {run}"
        );
        assert!(
            run.contains(r#"go list -m -f '{{.Dir}}'"#),
            "expected the module dir to be resolved via `go list -m`, got: {run}"
        );
        assert!(
            run.contains("go run ./cmd/download_ffi"),
            "expected the binding's own download_ffi tool to be invoked, got: {run}"
        );
        assert!(
            !run.contains("curl")
                && !run.contains("releases/download")
                && !run.contains("github.com/example/mylib/releases/download"),
            "runner must be generic — no project-specific download logic, got: {run}"
        );
        assert!(
            run.contains(r#"go mod edit -replace github.com/example/mylib/packages/go="$DST""#),
            "expected replace to the writable copy, got: {run}"
        );
        assert!(
            run.contains("git checkout go.mod go.sum"),
            "expected go.mod/go.sum to be restored unconditionally, got: {run}"
        );
        assert!(
            run.contains("GOWORK=off go test ./..."),
            "expected plain `go test ./...`, got: {run}"
        );
        assert!(run.contains("cd test_apps/go"), "expected cd test_apps/go, got: {run}");
    }

    #[test]
    fn zig_runs_zig_build_test() {
        let c = cfg(Language::Zig, "test_apps");
        let run = c.run.unwrap().commands().join(" ");
        assert!(run.contains("cd test_apps/zig"), "got: {run}");
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
        assert!(run.contains("cd test_apps/wasm"), "got: {run}");
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
    fn dart_run_does_not_invoke_download_libs() {
        let c = cfg(Language::Dart, "test_apps");
        let run = c.run.expect("dart should have a run command").commands().join(" ");
        assert_eq!(c.precondition.as_deref(), Some("command -v dart >/dev/null 2>&1"));
        assert!(run.contains("cd test_apps/dart"), "got: {run}");
        assert!(run.contains("dart pub get"), "got: {run}");
        assert!(run.contains("dart test"), "got: {run}");
        assert!(
            !run.contains("download_libs"),
            "dart run must not invoke download_libs (no GH Release tarball exists); got: {run}"
        );
        assert!(
            !run.contains("DART_PKG"),
            "dart run must not derive a DART_PKG var for the dropped download_libs call; got: {run}"
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
