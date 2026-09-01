use super::extras::Language;
use super::output::{BuildCommandConfig, StringOrVec};
use super::python_build;
use super::tools::{LangContext, require_tool, wrap_command as wrap};
use crate::core::template_versions as tv;

/// `maturin develop`'s own environment resolution, minus its parent-directory walk: it needs
/// `VIRTUAL_ENV`, `CONDA_PREFIX`, or a `.venv` directory, and without one it exits in tens of
/// milliseconds — long before any compilation could have started. A build failure that fast is
/// never a defect in generated code, so the check that would have caught it belongs here rather
/// than in a reader's head. ~keep
const PYTHON_ENVIRONMENT_CHECK: &str = r#"[ -n "$VIRTUAL_ENV" ] || [ -n "$CONDA_PREFIX" ] || [ -d .venv ]"#;

/// The one command that creates the interpreter environment `maturin develop` installs into,
/// phrased for whichever package manager `[tools] python_package_manager` selected.
fn python_environment_remediation(package_manager: &str) -> String {
    match package_manager {
        "poetry" => "poetry install".to_string(),
        "uv" => "uv venv".to_string(),
        _ => "python3 -m venv .venv".to_string(),
    }
}

/// `mix compile` refuses to run against unfetched dependencies ("the dependency is not available,
/// run `mix deps.get`"), and `deps/` is what `mix deps.get` creates — untracked, so absent on
/// every fresh checkout.
///
/// Gating on `mix.lock` instead was considered and rejected: alef does not scaffold a lockfile,
/// so a check that only fires when one exists would pass on exactly the pristine checkout that
/// motivated it and examine nothing. The residual cost is a dependency-free mix project, which
/// would be skipped forever — but the mix.exs alef scaffolds always declares `rustler`,
/// `rustler_precompiled`, `credo`, and `ex_doc` (see `scaffold::languages::elixir`), so that
/// project cannot be one alef generated. A user who hand-writes one overrides
/// `dependency_precondition`. ~keep
fn mix_dependency_check(output_dir: &str) -> String {
    format!("[ -d {output_dir}/deps ]")
}

/// The `gradle` task alef runs for a language's build, keyed by `Language` rather than by
/// the shared `"gradle"` tool string: `Kotlin` and `KotlinAndroid` both build through gradle,
/// but `KotlinAndroid`'s release must stay the variant-scoped `assembleRelease` task, never
/// the umbrella `build` task, which also assembles and verifies the debug variant.
///
/// `assembleRelease` runs the emitted `validateJniLibsForRelease` guard, which demands a
/// per-ABI `lib<crate>_jni.so` under `src/main/jniLibs/<abi>/`. Nothing in this build contract
/// can produce those: a host `cargo build` yields a host-architecture library, and cross-linking
/// against an Android NDK is a gradle-side concern that must also hold when a publish workflow
/// invokes gradle directly, bypassing `alef build` entirely. The satisfier therefore lives next
/// to the guard, as the generated `buildAndroidJniLibs` task in
/// `backends::kotlin_android::gen_build_gradle` — do not re-add it as a `before` step here, or
/// the two will disagree about which ABIs get built. Shared by this
/// module's own defaults below and by `build_command_for`'s `"gradle"` arm in
/// `src/cli/pipeline/commands/build/build_command.rs`, so both call sites derive the same
/// task from `Language` instead of two independent, tool-keyed and Language-keyed answers
/// that can silently drift apart (xberg-io/alef#259). ~keep
pub(crate) fn gradle_build_task(lang: Language, release: bool) -> &'static str {
    match (lang, release) {
        (Language::KotlinAndroid, false) => "assembleDebug",
        (Language::KotlinAndroid, true) => "assembleRelease",
        (_, false) => "build",
        (_, true) => "build -Prelease",
    }
}

/// Return the default build configuration for a language.
///
/// The `output_dir` is the package directory where scaffolded files live
/// (e.g. `packages/python`). The `crate_name` is the name of the core crate
/// (e.g. `my-lib`). Both are substituted into command templates. `ctx`
/// provides tool selection and run_wrapper.
pub(crate) fn default_build_config(
    lang: Language,
    output_dir: &str,
    crate_name: &str,
    ctx: &LangContext,
) -> BuildCommandConfig {
    let output_dir = super::shell::quote_word(output_dir);
    match lang {
        Language::Rust => BuildCommandConfig {
            precondition: Some(require_tool("cargo")),
            dependency_precondition: None,
            dependency_remediation: None,
            before: None,
            build: Some(StringOrVec::Single("cargo build --workspace".to_string())),
            build_release: Some(StringOrVec::Single("cargo build --release --workspace".to_string())),
            timeout_seconds: None,
        },
        Language::Python => {
            // Both the develop and the release command are composed here rather than written
            // out twice: the package-manager prefix and the `--features` flag are one fact each,
            // and `build_release` used to restate the whole invocation and so inherited every
            // gap `build` had. ~keep
            let feature = python_build::PYO3_EXTENSION_MODULE_FEATURE;
            let maturin = |release: bool| {
                let release_flag = if release { " --release" } else { "" };
                let command = format!(
                    "maturin develop --manifest-path crates/{crate_name}-py/Cargo.toml \
                     --features {feature}{release_flag}"
                );
                python_build::run_through_python_package_manager(command, ctx.tools)
            };
            BuildCommandConfig {
                precondition: Some(require_tool(python_build::python_build_precondition_tool(ctx.tools))),
                dependency_precondition: Some(PYTHON_ENVIRONMENT_CHECK.to_string()),
                dependency_remediation: Some(python_environment_remediation(ctx.tools.python_pm())),
                before: None,
                build: Some(StringOrVec::Single(maturin(false))),
                build_release: Some(StringOrVec::Single(maturin(true))),
                timeout_seconds: None,
            }
        }
        Language::Node => BuildCommandConfig {
            precondition: Some(require_tool("npm")),
            dependency_precondition: None,
            dependency_remediation: None,
            before: None,
            // `--package-json-path` pins napi-rs to the binding crate's own `package.json`
            // instead of letting it default to `<cwd>/package.json` (the repo root alef always
            // invokes it from), which otherwise bakes the wrong package name into the generated
            // loader whenever the repo also has a workspace-root `package.json`. See alef#368.
            build: Some(StringOrVec::Single(format!(
                "npx --yes -p @napi-rs/cli@3.7.3 napi build --manifest-path crates/{crate_name}-node/Cargo.toml -o crates/{crate_name}-node --package-json-path crates/{crate_name}-node/package.json --dts {}",
                tv::npm::NAPI_AUTO_DTS_FILENAME
            ))),
            build_release: Some(StringOrVec::Single(format!(
                "npx --yes -p @napi-rs/cli@3.7.3 napi build --manifest-path crates/{crate_name}-node/Cargo.toml -o crates/{crate_name}-node --package-json-path crates/{crate_name}-node/package.json --dts {} --release",
                tv::npm::NAPI_AUTO_DTS_FILENAME
            ))),
            timeout_seconds: None,
        },
        Language::Wasm => BuildCommandConfig {
            precondition: Some(require_tool("wasm-pack")),
            dependency_precondition: None,
            dependency_remediation: None,
            before: None,
            build: Some(StringOrVec::Single(format!(
                "wasm-pack build crates/{crate_name}-wasm --dev"
            ))),
            build_release: Some(StringOrVec::Single(format!(
                "wasm-pack build crates/{crate_name}-wasm --release"
            ))),
            timeout_seconds: None,
        },
        Language::Go => {
            let cmd = format!("cd {output_dir} && go build ./...");
            BuildCommandConfig {
                precondition: Some(require_tool("go")),
                dependency_precondition: None,
                dependency_remediation: None,
                before: None,
                build: Some(StringOrVec::Single(wrap(cmd.clone(), ctx.run_wrapper))),
                build_release: Some(StringOrVec::Single(wrap(cmd, ctx.run_wrapper))),
                timeout_seconds: None,
            }
        }
        Language::Ruby => BuildCommandConfig {
            precondition: Some(require_tool("cargo")),
            dependency_precondition: None,
            dependency_remediation: None,
            before: None,
            build: Some(StringOrVec::Single(format!("cargo build -p {crate_name}-rb"))),
            build_release: Some(StringOrVec::Single(format!("cargo build --release -p {crate_name}-rb"))),
            timeout_seconds: None,
        },
        Language::Php => BuildCommandConfig {
            precondition: Some(require_tool("cargo")),
            dependency_precondition: None,
            dependency_remediation: None,
            before: None,
            build: Some(StringOrVec::Single(format!("cargo build -p {crate_name}-php"))),
            build_release: Some(StringOrVec::Single(format!(
                "cargo build --release -p {crate_name}-php"
            ))),
            timeout_seconds: None,
        },
        Language::Ffi => BuildCommandConfig {
            precondition: Some(require_tool("cargo")),
            dependency_precondition: None,
            dependency_remediation: None,
            before: None,
            build: Some(StringOrVec::Single(format!(
                "cargo build --manifest-path {output_dir}/Cargo.toml"
            ))),
            build_release: Some(StringOrVec::Single(format!(
                "cargo build --release --manifest-path {output_dir}/Cargo.toml"
            ))),
            timeout_seconds: None,
        },
        Language::Java => {
            let (build_path, release_path) = if let Some(proj) = ctx.project_file {
                // `[crates.java] project_file` is consumer-configured and reaches `sh -c` as
                // `mvn -f`'s value -- quote it the same way `output_dir` above is quoted. ~keep
                let proj = super::shell::quote_word(proj);
                (
                    format!("mvn -f {proj} package -DskipTests --batch-mode --no-transfer-progress"),
                    format!("mvn -f {proj} package -DskipTests --batch-mode --no-transfer-progress"),
                )
            } else {
                (
                    format!("mvn -f {output_dir}/pom.xml package -DskipTests --batch-mode --no-transfer-progress"),
                    format!("mvn -f {output_dir}/pom.xml package -DskipTests --batch-mode --no-transfer-progress"),
                )
            };
            BuildCommandConfig {
                precondition: Some(require_tool("mvn")),
                dependency_precondition: None,
                dependency_remediation: None,
                before: None,
                build: Some(StringOrVec::Single(wrap(build_path, ctx.run_wrapper))),
                build_release: Some(StringOrVec::Single(wrap(release_path, ctx.run_wrapper))),
                timeout_seconds: None,
            }
        }
        Language::Csharp => {
            let (build_path, release_path) = if let Some(proj) = ctx.project_file {
                // `[crates.csharp] project_file` is consumer-configured and reaches `sh -c` as
                // `dotnet build`'s value -- quote it the same way `output_dir` above is quoted. ~keep
                let proj = super::shell::quote_word(proj);
                (
                    format!("dotnet build {proj} --configuration Debug -q"),
                    format!("dotnet build {proj} --configuration Release -q"),
                )
            } else {
                (
                    format!("dotnet build {output_dir} --configuration Debug -q"),
                    format!("dotnet build {output_dir} --configuration Release -q"),
                )
            };
            BuildCommandConfig {
                precondition: Some(require_tool("dotnet")),
                dependency_precondition: None,
                dependency_remediation: None,
                before: None,
                build: Some(StringOrVec::Single(wrap(build_path, ctx.run_wrapper))),
                build_release: Some(StringOrVec::Single(wrap(release_path, ctx.run_wrapper))),
                timeout_seconds: None,
            }
        }
        Language::Elixir => BuildCommandConfig {
            precondition: Some(require_tool("mix")),
            dependency_precondition: Some(mix_dependency_check(&output_dir)),
            dependency_remediation: Some(format!("cd {output_dir} && mix deps.get")),
            before: None,
            build: Some(StringOrVec::Single(format!("cd {output_dir} && mix compile"))),
            build_release: Some(StringOrVec::Single(format!("cd {output_dir} && mix compile"))),
            timeout_seconds: None,
        },
        Language::R => BuildCommandConfig {
            precondition: Some(require_tool("cargo")),
            dependency_precondition: None,
            dependency_remediation: None,
            before: None,
            build: Some(StringOrVec::Single(format!("cargo build -p {crate_name}-r"))),
            build_release: Some(StringOrVec::Single(format!("cargo build --release -p {crate_name}-r"))),
            timeout_seconds: None,
        },
        Language::Kotlin => BuildCommandConfig {
            precondition: Some(require_tool("gradle")),
            dependency_precondition: None,
            dependency_remediation: None,
            before: None,
            build: Some(StringOrVec::Single(wrap(
                format!("cd {output_dir} && gradle {}", gradle_build_task(lang, false)),
                ctx.run_wrapper,
            ))),
            build_release: Some(StringOrVec::Single(wrap(
                format!("cd {output_dir} && gradle {}", gradle_build_task(lang, true)),
                ctx.run_wrapper,
            ))),
            timeout_seconds: None,
        },
        Language::KotlinAndroid => BuildCommandConfig {
            precondition: Some(require_tool("gradle")),
            dependency_precondition: None,
            dependency_remediation: None,
            before: None,
            build: Some(StringOrVec::Single(wrap(
                format!("cd {output_dir} && gradle {}", gradle_build_task(lang, false)),
                ctx.run_wrapper,
            ))),
            build_release: Some(StringOrVec::Single(wrap(
                format!("cd {output_dir} && gradle {}", gradle_build_task(lang, true)),
                ctx.run_wrapper,
            ))),
            timeout_seconds: None,
        },
        Language::Swift => BuildCommandConfig {
            precondition: Some(require_tool("swift")),
            dependency_precondition: None,
            dependency_remediation: None,
            before: None,
            build: Some(StringOrVec::Single(wrap(
                format!("swift build --package-path {output_dir}"),
                ctx.run_wrapper,
            ))),
            build_release: Some(StringOrVec::Single(wrap(
                format!("swift build --package-path {output_dir} --configuration release"),
                ctx.run_wrapper,
            ))),
            timeout_seconds: None,
        },
        Language::Dart => BuildCommandConfig {
            precondition: Some(require_tool("dart")),
            dependency_precondition: None,
            dependency_remediation: None,
            before: None,
            build: Some(StringOrVec::Single(wrap(
                format!("cd {output_dir} && dart pub get"),
                ctx.run_wrapper,
            ))),
            build_release: Some(StringOrVec::Single(wrap(
                format!("cd {output_dir} && dart pub get"),
                ctx.run_wrapper,
            ))),
            timeout_seconds: None,
        },
        Language::Zig => BuildCommandConfig {
            precondition: Some(require_tool("zig")),
            dependency_precondition: None,
            dependency_remediation: None,
            before: None,
            build: Some(StringOrVec::Single(wrap(
                format!("cd {output_dir} && zig build"),
                ctx.run_wrapper,
            ))),
            build_release: Some(StringOrVec::Single(wrap(
                format!("cd {output_dir} && zig build --release=fast"),
                ctx.run_wrapper,
            ))),
            timeout_seconds: None,
        },
        Language::Gleam => BuildCommandConfig {
            precondition: Some(require_tool("gleam")),
            dependency_precondition: None,
            dependency_remediation: None,
            before: None,
            build: Some(StringOrVec::Single(wrap(
                format!("cd {output_dir} && gleam build"),
                ctx.run_wrapper,
            ))),
            build_release: Some(StringOrVec::Single(wrap(
                format!("cd {output_dir} && gleam build"),
                ctx.run_wrapper,
            ))),
            timeout_seconds: None,
        },
        Language::C => BuildCommandConfig {
            precondition: None,
            dependency_precondition: None,
            dependency_remediation: None,
            before: None,
            build: None,
            build_release: None,
            timeout_seconds: None,
        },
        Language::Jni => BuildCommandConfig {
            precondition: None,
            dependency_precondition: None,
            dependency_remediation: None,
            before: None,
            build: None,
            build_release: None,
            timeout_seconds: None,
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

    fn cfg(lang: Language, dir: &str, crate_name: &str) -> BuildCommandConfig {
        let tools = ToolsConfig::default();
        let ctx = LangContext::default(&tools);
        default_build_config(lang, dir, crate_name, &ctx)
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
    fn generated_build_quotes_configured_output_directory() {
        let malicious = "packages/go; touch /tmp/alef-build; #";
        let commands = cfg(Language::Go, malicious, "demo")
            .build
            .expect("go build command")
            .commands()
            .join(" ");
        assert!(commands.contains(&format!("cd {}", super::super::shell::quote_word(malicious))));
    }

    #[test]
    fn every_language_has_build_and_build_release() {
        for lang in all_languages() {
            let c = cfg(lang, "packages/test", "my-lib");
            assert!(c.build.is_some(), "{lang} should have a default build command");
            assert!(
                c.build_release.is_some(),
                "{lang} should have a default build_release command"
            );
        }
    }

    #[test]
    fn every_language_has_default_precondition() {
        for lang in all_languages() {
            let c = cfg(lang, "packages/test", "my-lib");
            let pre = c
                .precondition
                .unwrap_or_else(|| panic!("{lang} should have a precondition"));
            assert!(pre.starts_with("command -v "));
        }
    }

    /// Every dependency check must arrive with the command that satisfies it — the whole reason
    /// this outcome beats a bare failure is that it can tell the reader what to run. Enforced for
    /// user config in `validation::preconditions`; enforced for alef's own defaults here. ~keep
    #[test]
    fn every_dependency_precondition_ships_with_its_remediation() {
        for lang in all_languages() {
            let c = cfg(lang, "packages/test", "my-lib");
            assert_eq!(
                c.dependency_precondition.is_some(),
                c.dependency_remediation.is_some(),
                "{lang} must declare a dependency check and its remediation together"
            );
        }
    }

    /// The deliberate short list, pinned so it stays deliberate. Every language left out builds
    /// through a tool that resolves its own dependencies as part of the build (cargo, gradle,
    /// maven, dotnet, go, swiftpm, zig, gleam, pub) — giving those a dependency precondition
    /// would skip builds that work today, which is a worse defect than the one being fixed. ~keep
    #[test]
    fn only_tools_that_refuse_to_fetch_their_own_dependencies_declare_a_dependency_precondition() {
        let gated: Vec<Language> = all_languages()
            .into_iter()
            .filter(|lang| cfg(*lang, "packages/test", "my-lib").dependency_precondition.is_some())
            .collect();

        assert_eq!(gated, vec![Language::Python, Language::Elixir]);
    }

    #[test]
    fn python_dependency_precondition_matches_maturin_environment_resolution() {
        let c = cfg(Language::Python, "packages/python", "my-lib");
        let check = c.dependency_precondition.expect("python declares a dependency check");

        assert!(check.contains("VIRTUAL_ENV"), "{check}");
        assert!(check.contains("CONDA_PREFIX"), "{check}");
        assert!(check.contains(".venv"), "{check}");
        assert_eq!(c.dependency_remediation.as_deref(), Some("uv venv"));
    }

    /// Regression test for alef#368: napi-rs resolves the package name it bakes into the
    /// generated JS loader from whichever `package.json` it reads, defaulting to
    /// `<cwd>/package.json` rather than a path derived from `--manifest-path`/`-o`. This
    /// default command runs from the repo root, so without `--package-json-path` it silently
    /// read a workspace-root `package.json` in any consumer repo that had one. ~keep
    #[test]
    fn node_default_build_commands_point_napi_at_the_crate_local_package_json() {
        let c = cfg(Language::Node, "crates/my-lib-node", "my-lib");
        let build = c.build.expect("node has a default build command");
        let build_release = c.build_release.expect("node has a default build_release command");

        assert!(
            build
                .commands()
                .iter()
                .any(|cmd| cmd.contains("--package-json-path crates/my-lib-node/package.json")),
            "node build must be told explicitly which package.json names the binding crate: {build:?}"
        );
        assert!(
            build_release
                .commands()
                .iter()
                .any(|cmd| cmd.contains("--package-json-path crates/my-lib-node/package.json")),
            "node build_release must be told explicitly which package.json names the binding \
             crate: {build_release:?}"
        );
    }

    #[test]
    fn python_remediation_follows_the_configured_package_manager() {
        assert_eq!(python_environment_remediation("uv"), "uv venv");
        assert_eq!(python_environment_remediation("poetry"), "poetry install");
        assert_eq!(python_environment_remediation("pip"), "python3 -m venv .venv");
    }

    #[test]
    fn elixir_dependency_precondition_points_at_mix_deps_get() {
        let c = cfg(Language::Elixir, "packages/elixir", "my-lib");

        assert_eq!(
            c.dependency_precondition,
            Some(format!("[ -d {}/deps ]", quoted("packages/elixir")))
        );
        assert_eq!(
            c.dependency_remediation,
            Some(format!("cd {} && mix deps.get", quoted("packages/elixir")))
        );
    }

    /// Runs the emitted shell string against a real directory tree rather than asserting on its
    /// text: the check is a command, and a command that reads correctly but exits wrong would
    /// either skip every elixir build forever or examine nothing at all. The first case here is
    /// the pristine checkout that motivated the change — a scaffolded mix project with a mix.exs
    /// and no fetched dependencies — and it must fail. ~keep
    #[test]
    fn mix_dependency_check_fails_on_a_pristine_checkout_and_passes_once_deps_are_fetched() {
        let root = tempfile::tempdir().expect("tempdir");
        // The check is emitted with the *relative* `output_dir` a real config carries
        // (`packages/elixir`), and is run from the workspace root. Handing `sh` an absolute path
        // instead would feed it `C:\Users\...` on Windows, where every backslash is an `sh`
        // escape -- the test then answers "no deps" whatever is on disk, and its first assertion
        // passes for the wrong reason. ~keep
        const PACKAGE: &str = "packages/elixir";
        let package = root.path().join(PACKAGE);
        std::fs::create_dir_all(&package).expect("create package dir");
        let passes = || {
            std::process::Command::new("sh")
                .args(["-c", &mix_dependency_check(PACKAGE)])
                .current_dir(root.path())
                .status()
                .expect("check runs")
                .success()
        };
        std::fs::write(package.join("mix.exs"), "defmodule Sample.MixProject do\nend\n").expect("write mix.exs");

        assert!(
            !passes(),
            "a checked-out mix project with no deps/ has not run `mix deps.get`"
        );

        std::fs::create_dir(package.join("deps")).expect("create deps");
        assert!(passes(), "fetched deps must let the build through");
    }

    #[test]
    fn rust_uses_cargo_build_workspace() {
        let c = cfg(Language::Rust, "packages/rust", "my-lib");
        let build = c.build.unwrap().commands().join(" ");
        let release = c.build_release.unwrap().commands().join(" ");
        assert!(build.contains("cargo build --workspace"));
        assert!(release.contains("cargo build --release --workspace"));
    }

    #[test]
    fn python_uses_maturin_develop() {
        let c = cfg(Language::Python, "packages/python", "my-lib");
        let build = c.build.unwrap().commands().join(" ");
        let release = c.build_release.unwrap().commands().join(" ");
        assert!(build.contains("maturin develop"));
        assert!(build.contains("my-lib-py"));
        assert!(release.contains("--release"));
    }

    #[test]
    fn node_uses_napi_build() {
        let c = cfg(Language::Node, "packages/node", "my-lib");
        let build = c.build.unwrap().commands().join(" ");
        let release = c.build_release.unwrap().commands().join(" ");
        assert!(build.contains("npx --yes -p @napi-rs/cli@3.7.3 napi"));
        assert!(build.contains("build --manifest-path"));
        assert!(build.contains("my-lib-node"));
        assert!(release.contains("--release"));
    }

    #[test]
    fn wasm_uses_wasm_pack() {
        let c = cfg(Language::Wasm, "packages/wasm", "my-lib");
        let build = c.build.unwrap().commands().join(" ");
        let release = c.build_release.unwrap().commands().join(" ");
        assert!(build.contains("wasm-pack build"));
        assert!(build.contains("my-lib-wasm"));
        assert!(build.contains("--dev"));
        assert!(release.contains("--release"));
    }

    #[test]
    fn ffi_uses_its_manifest_without_requiring_workspace_membership() {
        let c = cfg(Language::Ffi, "packages/ffi", "my-lib");
        let build = c.build.unwrap().commands().join(" ");
        let release = c.build_release.unwrap().commands().join(" ");
        let manifest = format!("{}/Cargo.toml", quoted("packages/ffi"));
        assert_eq!(build, format!("cargo build --manifest-path {manifest}"));
        assert_eq!(release, format!("cargo build --release --manifest-path {manifest}"));
    }

    /// `--manifest-path 'packages/ffi'/Cargo.toml` quotes only the *prefix* of a path, which reads
    /// as a mistake and would be one in a tool that took the argument literally. It is not: `sh`
    /// concatenates the quoted word with the unquoted suffix into a single argument before cargo
    /// ever sees it. Text assertions cannot tell that apart from a genuinely malformed path, so
    /// this one asks a real shell what argv cargo receives. ~keep
    #[cfg(unix)]
    #[test]
    fn ffi_manifest_path_reaches_cargo_as_one_unquoted_path() {
        let command = cfg(Language::Ffi, "packages/ffi", "my-lib")
            .build
            .expect("ffi build command")
            .commands()
            .join(" ");
        let output = std::process::Command::new("sh")
            .args(["-c", &format!("printf '%s\\n' {command}")])
            .output()
            .expect("shell should start");
        let argv: Vec<String> = String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::to_string)
            .collect();

        assert_eq!(argv, ["cargo", "build", "--manifest-path", "packages/ffi/Cargo.toml"]);
    }

    #[test]
    fn ruby_uses_cargo_build_rb() {
        let c = cfg(Language::Ruby, "packages/ruby", "my-lib");
        let build = c.build.unwrap().commands().join(" ");
        assert!(build.contains("cargo build -p my-lib-rb"));
    }

    #[test]
    fn php_uses_cargo_build_php() {
        let c = cfg(Language::Php, "packages/php", "my-lib");
        let build = c.build.unwrap().commands().join(" ");
        assert!(build.contains("cargo build -p my-lib-php"));
    }

    #[test]
    fn r_uses_cargo_build_r() {
        let c = cfg(Language::R, "packages/r", "my-lib");
        let build = c.build.unwrap().commands().join(" ");
        assert!(build.contains("cargo build -p my-lib-r"));
    }

    #[test]
    fn java_uses_maven_package() {
        let c = cfg(Language::Java, "packages/java", "my-lib");
        let build = c.build.unwrap().commands().join(" ");
        assert!(build.contains("mvn"));
        assert!(build.contains("package"));
        assert!(build.contains("-DskipTests"));
    }

    #[test]
    fn csharp_uses_dotnet_build_configurations() {
        let c = cfg(Language::Csharp, "packages/csharp", "my-lib");
        let build = c.build.unwrap().commands().join(" ");
        let release = c.build_release.unwrap().commands().join(" ");
        assert!(build.contains("dotnet build"));
        assert!(build.contains("--configuration Debug"));
        assert!(release.contains("--configuration Release"));
    }

    #[test]
    fn elixir_uses_mix_compile() {
        let c = cfg(Language::Elixir, "packages/elixir", "my-lib");
        let build = c.build.unwrap().commands().join(" ");
        assert!(build.contains("mix compile"));
    }

    #[test]
    fn crate_name_substituted_in_commands() {
        let c = cfg(Language::Python, "packages/python", "custom-crate");
        let build = c.build.unwrap().commands().join(" ");
        assert!(build.contains("custom-crate-py"));
    }

    #[test]
    fn output_dir_substituted_in_go_commands() {
        let c = cfg(Language::Go, "my/custom/path", "my-lib");
        let build = c.build.unwrap().commands().join(" ");
        assert!(build.contains("my/custom/path"));
    }

    #[test]
    fn kotlin_uses_gradle_build() {
        let c = cfg(Language::Kotlin, "packages/kotlin", "my-lib");
        let build = c.build.unwrap().commands().join(" ");
        let release = c.build_release.unwrap().commands().join(" ");
        assert!(
            build.contains("gradle build"),
            "Kotlin build should use gradle build, got: {build}"
        );
        assert!(
            release.contains("gradle build"),
            "Kotlin release should use gradle build, got: {release}"
        );
        assert_eq!(c.precondition.as_deref(), Some("command -v gradle >/dev/null 2>&1"));
    }

    /// Pins the intentional `Kotlin` vs `KotlinAndroid` gradle *task* divergence:
    /// `assembleDebug`/`assembleRelease` (the narrower, variant-scoped AGP task) instead of
    /// the umbrella `build` task every other gradle-backed language here defaults to.
    ///
    /// This test's old name claimed to check that divergence against `build_command_for`'s
    /// `"gradle"` arm in `src/cli/pipeline/commands/build/build_command.rs` — it never did,
    /// and could not: that function is `pub(super)` to a different module tree, unreachable
    /// from here. Both this default and that arm now derive their task from
    /// `gradle_build_task(Language, bool)` above, so they no longer diverge on the task
    /// itself (they still differ on directory resolution: this one takes `output_dir`
    /// verbatim, that arm walks up to the nearest `settings.gradle`/`build.gradle` root).
    /// The actual cross-check that the two call sites agree lives where both functions are
    /// reachable — `kotlin_android_gradle_arm_matches_build_defaults_default` in
    /// `build_command_tests.rs`. Its prior absence, next to a same-module test whose name
    /// implied the comparison had already been made, is what let xberg-io/alef#259 ship:
    /// the CLI path fell into a tool-keyed `"gradle"` arm that could not distinguish
    /// `Kotlin` from `KotlinAndroid` and ran the umbrella `build` task for both. Changing
    /// the task `gradle_build_task` returns is a deliberate decision that must update both
    /// tests. ~keep
    #[test]
    fn kotlin_android_gradle_task_diverges_intentionally_from_kotlin() {
        let c = cfg(Language::KotlinAndroid, "packages/kotlin-android", "my-lib");
        let build = c.build.unwrap().commands().join(" ");
        let release = c.build_release.unwrap().commands().join(" ");
        let dir = quoted("packages/kotlin-android");
        assert_eq!(build, format!("cd {dir} && gradle assembleDebug"));
        assert_eq!(release, format!("cd {dir} && gradle assembleRelease"));
        assert_eq!(c.precondition.as_deref(), Some("command -v gradle >/dev/null 2>&1"));
    }

    #[test]
    fn swift_uses_swift_build_with_package_path() {
        let c = cfg(Language::Swift, "packages/swift", "my-lib");
        let build = c.build.unwrap().commands().join(" ");
        let release = c.build_release.unwrap().commands().join(" ");
        assert!(
            build.contains("swift build"),
            "Swift build should use swift build, got: {build}"
        );
        assert!(
            build.contains(&format!("--package-path {}", quoted("packages/swift"))),
            "Swift build should include package path, got: {build}"
        );
        assert!(
            release.contains("--configuration release"),
            "Swift release should use --configuration release, got: {release}"
        );
    }

    #[test]
    fn dart_uses_dart_pub_get() {
        let c = cfg(Language::Dart, "packages/dart", "my-lib");
        let build = c.build.unwrap().commands().join(" ");
        assert!(
            build.contains("dart pub get"),
            "Dart build should use dart pub get, got: {build}"
        );
        assert_eq!(c.precondition.as_deref(), Some("command -v dart >/dev/null 2>&1"));
    }

    #[test]
    fn gleam_uses_gleam_build() {
        let c = cfg(Language::Gleam, "packages/gleam", "my-lib");
        let build = c.build.unwrap().commands().join(" ");
        let release = c.build_release.unwrap().commands().join(" ");
        assert!(
            build.contains("gleam build"),
            "Gleam build should use gleam build, got: {build}"
        );
        assert!(
            release.contains("gleam build"),
            "Gleam release should use gleam build, got: {release}"
        );
        assert_eq!(c.precondition.as_deref(), Some("command -v gleam >/dev/null 2>&1"));
    }

    #[test]
    fn zig_uses_zig_build() {
        let c = cfg(Language::Zig, "packages/zig", "my-lib");
        let build = c.build.unwrap().commands().join(" ");
        let release = c.build_release.unwrap().commands().join(" ");
        assert!(
            build.contains("zig build"),
            "Zig build should use zig build, got: {build}"
        );
        assert!(
            release.contains("--release=fast"),
            "Zig release should use --release=fast, got: {release}"
        );
        assert_eq!(c.precondition.as_deref(), Some("command -v zig >/dev/null 2>&1"));
    }
}
