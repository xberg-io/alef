use super::*;
use crate::core::backend::{BuildConfig, BuildDependency};

/// A directory as it is spelled *inside an emitted shell command* — a quoted word, not a bare
/// path. Expectations derive it from `quote_word` rather than restating one quoting spelling, so
/// a change to the escaping policy cannot silently repoint a command at a different directory:
/// the escaping itself is proved separately, and once, by
/// `core::config::shell::tests::quote_word_preserves_literal_shell_value`, which runs a hostile
/// value through a real shell. ~keep
fn quoted(dir: &str) -> String {
    crate::core::config::shell::quote_word(dir)
}

#[test]
fn csharp_build_command_uses_verbosity_flag_not_query_mode() {
    let alef_cfg: crate::core::config::NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["csharp"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
"#,
    )
    .unwrap();
    let config = alef_cfg.resolve().unwrap().remove(0);
    let build_config = BuildConfig {
        tool: "dotnet",
        crate_suffix: "",
        build_dep: BuildDependency::Ffi,
        post_build: Vec::new(),
    };

    let command = build_command_for(Language::Csharp, &build_config, &config, false);

    assert!(
        command.contains("--verbosity quiet"),
        "C# build must use explicit quiet verbosity: {command}"
    );
    assert!(
        !command.contains(" -q"),
        "C# build must not use dotnet query mode shorthand: {command}"
    );
}

/// Regression test: `napi build`'s own `--dts` output defaults to the crate's
/// `package.json` `"types"` field, which is `index.d.ts` — the exact file alef's node
/// backend writes its own hand-derived type declarations (unions, doc comments, the
/// `alef:hash:` provenance line) to. Every `napi build` invocation this arm emits — the
/// default node build step every consumer without a `[build_commands.node]` override
/// runs — used to leave `--dts` unset, so a routine `alef build` (or the scaffolded
/// `pnpm run build`, which shares this same command shape) silently clobbered alef's
/// canonical `index.d.ts` with napi-rs's own auto-derived one, discarding the
/// provenance header `alef verify` relies on to detect staleness. ~keep
#[test]
fn napi_build_command_never_lets_napi_rs_overwrite_alefs_index_d_ts() {
    let alef_cfg: crate::core::config::NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["node"]

[[crates]]
name = "sample-lib"
sources = ["src/lib.rs"]
"#,
    )
    .unwrap();
    let config = alef_cfg.resolve().unwrap().remove(0);
    let build_config = BuildConfig {
        tool: "napi",
        crate_suffix: "-node",
        build_dep: BuildDependency::None,
        post_build: Vec::new(),
    };

    let command = build_command_for(Language::Node, &build_config, &config, false);

    assert!(
        command.contains(&format!(
            "--dts {}",
            crate::core::template_versions::npm::NAPI_AUTO_DTS_FILENAME
        )),
        "napi build must redirect its own auto-derived .d.ts away from alef's \
         index.d.ts: {command}"
    );
    assert!(
        !command.contains("--dts index.d.ts"),
        "napi build must never be told to write its own type declarations over alef's \
         canonical index.d.ts: {command}"
    );
}

/// Regression test for alef#368: napi-rs resolves the package name it bakes into the
/// generated JS loader (and, with `--platform`, every target's optional-dependency package
/// name) from whichever `package.json` it reads, which defaults to `<cwd>/package.json` --
/// not a path derived from `--manifest-path`/`-o`. alef always invokes napi from the repo
/// root, so a consumer repo that also has a workspace-root `package.json` (a common
/// monorepo layout) got that package's name baked into the loader instead of the binding
/// crate's own -- silently generating optional-dependency requires for packages that do not
/// exist. `--package-json-path` must always point at the binding crate's own manifest. ~keep
#[test]
fn napi_build_command_points_napi_at_the_crate_local_package_json() {
    let alef_cfg: crate::core::config::NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["node"]

[[crates]]
name = "sample-lib"
sources = ["src/lib.rs"]
"#,
    )
    .unwrap();
    let config = alef_cfg.resolve().unwrap().remove(0);
    let build_config = BuildConfig {
        tool: "napi",
        crate_suffix: "-node",
        build_dep: BuildDependency::None,
        post_build: Vec::new(),
    };

    let command = build_command_for(Language::Node, &build_config, &config, false);

    assert!(
        command.contains(&format!(
            "--package-json-path {}/package.json",
            quoted("crates/sample-lib-node")
        )),
        "napi build must be told explicitly which package.json names the binding crate, \
         rather than letting it default to the repo root's: {command}"
    );
}

/// Companion to the test above: `[crates.output] node` is unconfigured here, which is the
/// common case (most consumers never set it). Before this fix, that left `crate_dir` empty
/// in this arm alone -- every other backend arm in this function falls back to
/// `config.package_dir(lang)` when its own `output_path_for` lookup is empty, but this one
/// didn't, so the emitted command was `--manifest-path /Cargo.toml -o  --dts ...`, pointing
/// at the repo root instead of the generated crate. ~keep
#[test]
fn napi_build_command_falls_back_to_the_default_crate_dir_when_output_is_unconfigured() {
    let alef_cfg: crate::core::config::NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["node"]

[[crates]]
name = "sample-lib"
sources = ["src/lib.rs"]
"#,
    )
    .unwrap();
    let config = alef_cfg.resolve().unwrap().remove(0);
    let build_config = BuildConfig {
        tool: "napi",
        crate_suffix: "-node",
        build_dep: BuildDependency::None,
        post_build: Vec::new(),
    };

    let command = build_command_for(Language::Node, &build_config, &config, false);
    let node_dir = quoted("crates/sample-lib-node");

    assert!(
        command.contains(&format!("--manifest-path {node_dir}/Cargo.toml")),
        "an unconfigured [crates.output] node must still resolve to the default crate \
         directory, not an empty path: {command}"
    );
    assert!(
        command.contains(&format!("-o {node_dir} ")),
        "an unconfigured [crates.output] node must still pass the default crate directory \
         as the output dir, not an empty one: {command}"
    );
}

/// Same regression as [`napi_build_command_points_napi_at_the_crate_local_package_json`],
/// but with `[crates.output] node` set explicitly -- the shape of the consumer config that
/// shipped alef#368: the crate directory itself resolved correctly (`--manifest-path`/`-o`
/// already pointed at it), so only `--package-json-path` was missing.
#[test]
fn napi_build_command_honors_an_explicit_output_path_for_package_json_too() {
    let alef_cfg: crate::core::config::NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["node"]

[[crates]]
name = "sample-lib"
sources = ["src/lib.rs"]

[crates.output]
node = "crates/sample-lib-node/src"
"#,
    )
    .unwrap();
    let config = alef_cfg.resolve().unwrap().remove(0);
    let build_config = BuildConfig {
        tool: "napi",
        crate_suffix: "-node",
        build_dep: BuildDependency::None,
        post_build: Vec::new(),
    };

    let command = build_command_for(Language::Node, &build_config, &config, false);

    assert!(
        command.contains(&format!(
            "--package-json-path {}/package.json",
            quoted("crates/sample-lib-node")
        )),
        "an explicit [crates.output] node must still resolve --package-json-path to the \
         binding crate's own manifest: {command}"
    );
}

#[test]
fn kotlin_gradle_build_command_runs_in_generated_package() {
    let alef_cfg: crate::core::config::NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["kotlin"]

[[crates]]
name = "sample-lib"
sources = ["src/lib.rs"]
"#,
    )
    .unwrap();
    let config = alef_cfg.resolve().unwrap().remove(0);
    let build_config = BuildConfig {
        tool: "gradle",
        crate_suffix: "",
        build_dep: BuildDependency::Ffi,
        post_build: Vec::new(),
    };

    let dir = quoted("packages/kotlin");
    assert_eq!(
        build_command_for(Language::Kotlin, &build_config, &config, false),
        format!("cd {dir} && gradle build")
    );
    assert_eq!(
        build_command_for(Language::Kotlin, &build_config, &config, true),
        format!("cd {dir} && gradle build -Prelease")
    );
}

/// Regression test for the real consumer failure this arm was rewritten for: an
/// explicit `[crates.output] kotlin_android` pointing at a deep, gradle-marker-free
/// namespace source directory (`.../src/main/kotlin/io/<ns>/<pkg>/android/`) used to be
/// `cd`-ed into verbatim, and gradle rejected it ("Project directory '...' is not part
/// of the build defined by settings file '...'"). The fix must walk up to the nearest
/// `settings.gradle.kts` and build from there instead. ~keep
#[test]
fn gradle_build_walks_up_to_settings_gradle_root_from_deep_namespace_dir() {
    let root = tempfile::tempdir().expect("failed to create temp dir");
    let project_root = root.path().join("packages/kotlin-android");
    let deep_source_dir = project_root.join("src/main/kotlin/io/alpha/mylib/android");
    std::fs::create_dir_all(&deep_source_dir).expect("failed to create deep namespace dir");
    std::fs::write(
        project_root.join("settings.gradle.kts"),
        "rootProject.name = \"alpha\"\n",
    )
    .expect("failed to write settings.gradle.kts fixture");
    std::fs::write(project_root.join("build.gradle.kts"), "// android library build\n")
        .expect("failed to write build.gradle.kts fixture");

    // Prove the fixture actually built the shape under test before asserting on it:
    // both markers must exist at the project root and be absent from the deep dir.
    assert!(project_root.join("settings.gradle.kts").is_file());
    assert!(project_root.join("build.gradle.kts").is_file());
    assert!(!deep_source_dir.join("settings.gradle.kts").exists());
    assert!(!deep_source_dir.join("build.gradle.kts").exists());

    let alef_cfg: crate::core::config::NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["kotlin_android"]

[[crates]]
name = "sample-lib"
sources = ["src/lib.rs"]

[crates.kotlin_android]
package = "dev.alpha"
"#,
    )
    .unwrap();
    let mut config = alef_cfg.resolve().unwrap().remove(0);
    // `[crates.output] kotlin_android` is set directly on the resolved config rather than
    // through TOML: path-safety validation now rejects any absolute `[crates.output]` value at
    // `resolve()` time (it would let a hostile config value write generated files outside the
    // project root), but this test needs a real absolute tempdir with real gradle marker files
    // on disk to exercise the walk-up logic against. ~keep
    let explicit = std::path::PathBuf::from(format!("{}/", deep_source_dir.display()));
    config.explicit_output.kotlin_android = Some(explicit.clone());
    config.output_paths.insert("kotlin_android".to_string(), explicit);
    let build_config = BuildConfig {
        tool: "gradle",
        crate_suffix: "",
        build_dep: BuildDependency::Ffi,
        post_build: Vec::new(),
    };

    let expected_root = quoted(&project_root.display().to_string());
    assert_eq!(
        build_command_for(Language::KotlinAndroid, &build_config, &config, false),
        format!("cd {expected_root} && gradle assembleDebug")
    );
    assert_eq!(
        build_command_for(Language::KotlinAndroid, &build_config, &config, true),
        format!("cd {expected_root} && gradle assembleRelease")
    );
}

/// Boundary case: no `settings.gradle*`/`build.gradle*` exists anywhere up the ancestor
/// chain, so the walk-up must fall back to the original source directory unchanged
/// (like the mix/mvn/dotnet arms above it) instead of walking to the filesystem root or
/// panicking. ~keep
#[test]
fn gradle_build_falls_back_to_source_dir_when_no_marker_found() {
    let root = tempfile::tempdir().expect("failed to create temp dir");
    let deep_source_dir = root
        .path()
        .join("packages/kotlin-android/src/main/kotlin/io/alpha/mylib/android");
    std::fs::create_dir_all(&deep_source_dir).expect("failed to create deep namespace dir");

    // Prove the fixture is genuinely marker-free before asserting on the fallback.
    assert!(deep_source_dir.is_dir());
    for ancestor in deep_source_dir.ancestors() {
        for marker in [
            "settings.gradle.kts",
            "settings.gradle",
            "build.gradle.kts",
            "build.gradle",
        ] {
            assert!(
                !ancestor.join(marker).exists(),
                "fixture must not contain {marker} under {}",
                ancestor.display()
            );
        }
        if ancestor == root.path() {
            break;
        }
    }

    let alef_cfg: crate::core::config::NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["kotlin_android"]

[[crates]]
name = "sample-lib"
sources = ["src/lib.rs"]

[crates.kotlin_android]
package = "dev.alpha"
"#,
    )
    .unwrap();
    let mut config = alef_cfg.resolve().unwrap().remove(0);
    // `[crates.output] kotlin_android` is set directly on the resolved config rather than
    // through TOML: path-safety validation now rejects any absolute `[crates.output]` value at
    // `resolve()` time (it would let a hostile config value write generated files outside the
    // project root), but this test needs a real absolute tempdir on disk to exercise the
    // marker-free fallback against. ~keep
    let explicit = std::path::PathBuf::from(format!("{}/", deep_source_dir.display()));
    config.explicit_output.kotlin_android = Some(explicit.clone());
    config.output_paths.insert("kotlin_android".to_string(), explicit);
    let build_config = BuildConfig {
        tool: "gradle",
        crate_suffix: "",
        build_dep: BuildDependency::Ffi,
        post_build: Vec::new(),
    };

    // The configured output path carries a trailing separator and the fallback returns it
    // verbatim, so the expectation must too -- `cd <dir>/` is what a real run emits. ~keep
    let expected = quoted(&format!("{}/", deep_source_dir.display()));
    assert_eq!(
        build_command_for(Language::KotlinAndroid, &build_config, &config, false),
        format!("cd {expected} && gradle assembleDebug")
    );
}

/// The missing check whose absence let xberg-io/alef#259 ship: `build_command_for`'s
/// `"gradle"` arm and `build_defaults::default_build_config`'s `KotlinAndroid` arm must
/// resolve to the *same* command, not two independent derivations that happen to agree only
/// when a consumer declares a `[workspace.build_commands.kotlin_android]` overlay. Before
/// `gradle_build_task` existed, this failed: the gradle arm matched on the `"gradle"` tool
/// string shared by `Kotlin` and `KotlinAndroid`, so it ran the umbrella `gradle build` for
/// `KotlinAndroid` too, while `build_defaults` (matched on `Language`) already knew to run
/// `assembleDebug`/`assembleRelease`. With no directory markers on disk, `build_command_for`'s
/// walk-up falls back to `source_dir` unchanged, so both sides resolve the identical
/// `packages/kotlin-android` directory here and a byte-for-byte comparison is meaningful. ~keep
#[test]
fn kotlin_android_gradle_arm_matches_build_defaults_default() {
    let alef_cfg: crate::core::config::NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["kotlin_android"]

[[crates]]
name = "sample-lib"
sources = ["src/lib.rs"]

[crates.kotlin_android]
package = "dev.alpha"
"#,
    )
    .unwrap();
    let config = alef_cfg.resolve().unwrap().remove(0);
    let build_config = BuildConfig {
        tool: "gradle",
        crate_suffix: "",
        build_dep: BuildDependency::Ffi,
        post_build: Vec::new(),
    };

    let cli_build = build_command_for(Language::KotlinAndroid, &build_config, &config, false);
    let cli_release = build_command_for(Language::KotlinAndroid, &build_config, &config, true);

    let defaults = config.build_command_config_for_language(Language::KotlinAndroid);
    let default_build = defaults
        .build
        .expect("KotlinAndroid has a default build command")
        .commands()
        .join(" ");
    let default_release = defaults
        .build_release
        .expect("KotlinAndroid has a default build_release command")
        .commands()
        .join(" ");

    assert_eq!(
        cli_build, default_build,
        "build_command_for's gradle arm must resolve the same KotlinAndroid build command as \
         build_defaults, not the umbrella `gradle build` shared with Kotlin"
    );
    assert_eq!(
        cli_release, default_release,
        "build_command_for's gradle arm must resolve the same KotlinAndroid release command as \
         build_defaults, not the umbrella `gradle build -Prelease` shared with Kotlin"
    );
    // Equality alone is satisfied by both sides being wrong in the same way. The commands reach
    // `sh -c` with a consumer-configured directory in them, so pin the property that made them
    // diverge in the first place: the directory is a *quoted word*, on both sides. ~keep
    assert_eq!(
        cli_build,
        format!("cd {} && gradle assembleDebug", quoted("packages/kotlin-android")),
        "both call sites must shell-quote the directory they cd into"
    );
}

#[test]
fn unknown_build_tool_fails_instead_of_reporting_success() {
    let config = ResolvedCrateConfig::default();
    let build_config = BuildConfig {
        tool: "unsupported",
        crate_suffix: "",
        build_dep: BuildDependency::None,
        post_build: Vec::new(),
    };

    let command = build_command_for(Language::Kotlin, &build_config, &config, false);
    assert!(
        command.ends_with("&& false"),
        "unknown build tool must still exit non-zero: {command}"
    );
    assert!(
        command.contains("no default build command for tool \"unsupported\""),
        "unknown build tool failure must name the missing tool instead of a bare `false`: {command}"
    );
}

/// The `mvn`/`mix`/`cargo` arms all resolve a build directory by walking up from a
/// consumer-configured `[crates.output]` path looking for a marker file (`pom.xml`, `mix.exs`,
/// `Cargo.toml`), then splice the result into `cd {dir} && …` -- the same shape the
/// `gradle`/`dotnet`/`go` arms were fixed for. These were not covered by any literal-string test
/// before this file (a text assertion could not tell a correct path from an injected one
/// anyway), so each of the three gets a real-`sh` injection-witness test instead. ~keep
#[cfg(unix)]
#[test]
fn mvn_build_command_keeps_a_hostile_output_dir_out_of_shell_text() {
    let root = tempfile::tempdir().expect("tempdir");
    let hostile = "packages/java$(touch witness)#";
    let package = root.path().join(hostile);
    std::fs::create_dir_all(&package).expect("create hostile package dir");
    std::fs::write(package.join("pom.xml"), "<project/>\n").expect("write pom.xml");
    let witness = root.path().join("witness");

    let alef_cfg: crate::core::config::NewAlefConfig = toml::from_str(&format!(
        r#"
[workspace]
languages = ["java"]

[[crates]]
name = "sample-lib"
sources = ["src/lib.rs"]

[crates.output]
java = {hostile:?}
"#
    ))
    .unwrap();
    let config = alef_cfg.resolve().unwrap().remove(0);
    let build_config = BuildConfig {
        tool: "mvn",
        crate_suffix: "-java",
        build_dep: BuildDependency::None,
        post_build: Vec::new(),
    };

    let command = build_command_for(Language::Java, &build_config, &config, false);
    std::process::Command::new("sh")
        .args(["-c", &command])
        .current_dir(root.path())
        .output()
        .expect("shell should start");

    assert!(
        !witness.exists(),
        "command substitution in the output path must not run: {command}"
    );
}

#[cfg(unix)]
#[test]
fn mix_build_command_keeps_a_hostile_output_dir_out_of_shell_text() {
    let root = tempfile::tempdir().expect("tempdir");
    let hostile = "packages/elixir$(touch witness)#";
    let package = root.path().join(hostile);
    std::fs::create_dir_all(&package).expect("create hostile package dir");
    std::fs::write(package.join("mix.exs"), "").expect("write mix.exs");
    let witness = root.path().join("witness");

    let alef_cfg: crate::core::config::NewAlefConfig = toml::from_str(&format!(
        r#"
[workspace]
languages = ["elixir"]

[[crates]]
name = "sample-lib"
sources = ["src/lib.rs"]

[crates.output]
elixir = {hostile:?}
"#
    ))
    .unwrap();
    let config = alef_cfg.resolve().unwrap().remove(0);
    let build_config = BuildConfig {
        tool: "mix",
        crate_suffix: "-elixir",
        build_dep: BuildDependency::Rustler,
        post_build: Vec::new(),
    };

    let command = build_command_for(Language::Elixir, &build_config, &config, false);
    std::process::Command::new("sh")
        .args(["-c", &command])
        .current_dir(root.path())
        .output()
        .expect("shell should start");

    assert!(
        !witness.exists(),
        "command substitution in the output path must not run: {command}"
    );
}

#[cfg(unix)]
#[test]
fn cargo_ffi_native_dir_build_keeps_a_hostile_crate_dir_out_of_shell_text() {
    let root = tempfile::tempdir().expect("tempdir");
    let hostile = "crates/sample-lib-ffi$(touch witness)#";
    let crate_dir = root.path().join(hostile);
    std::fs::create_dir_all(crate_dir.join("native")).expect("create native dir");
    std::fs::write(crate_dir.join("native").join("Cargo.toml"), "[package]\nname = \"n\"\n")
        .expect("write native Cargo.toml");
    let witness = root.path().join("witness");

    let alef_cfg: crate::core::config::NewAlefConfig = toml::from_str(&format!(
        r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "sample-lib"
sources = ["src/lib.rs"]

[crates.output]
ffi = {hostile:?}
"#
    ))
    .unwrap();
    let config = alef_cfg.resolve().unwrap().remove(0);
    let build_config = BuildConfig {
        tool: "cargo",
        crate_suffix: "-ffi",
        build_dep: BuildDependency::None,
        post_build: Vec::new(),
    };

    let command = build_command_for(Language::Ffi, &build_config, &config, false);
    std::process::Command::new("sh")
        .args(["-c", &command])
        .current_dir(root.path())
        .output()
        .expect("shell should start");

    assert!(
        !witness.exists(),
        "command substitution in the crate dir must not run: {command}"
    );
}

fn wasm_config(extra: &str) -> ResolvedCrateConfig {
    let alef_cfg: crate::core::config::NewAlefConfig = toml::from_str(&format!(
        r#"
[workspace]
languages = ["wasm"]

[[crates]]
name = "sample-lib"
sources = ["src/lib.rs"]
{extra}
"#
    ))
    .unwrap();
    alef_cfg.resolve().unwrap().remove(0)
}

fn wasm_build_config() -> BuildConfig {
    BuildConfig {
        tool: "wasm-pack",
        crate_suffix: "-wasm",
        build_dep: BuildDependency::None,
        post_build: Vec::new(),
    }
}

/// Regression test: `alef e2e` (local mode) resolves the wasm package through
/// `pkg/nodejs` (`ResolvedCrateConfig::wasm_crate_path`), and the scaffolded crate
/// manifest resolves `main`/`types` to `pkg/nodejs/…` and `module` to `pkg/web/…`.
/// `alef build` used to run a single `--target web` with wasm-pack's *default* out-dir,
/// producing a bare `pkg/` that none of those consumers read while never producing
/// `pkg/nodejs` at all — so a fresh checkout needed a hand-rolled extra build step.
/// The emitted command must match the scaffolded `build:wasm:<target>` scripts exactly,
/// since `publish::package::wasm` re-runs those same scripts via `pnpm run build:all`
/// and the two must not disagree about where a target lands. ~keep
#[test]
fn wasm_build_command_builds_every_default_target_into_its_own_pkg_subdir() {
    let config = wasm_config("");

    let command = build_command_for(Language::Wasm, &wasm_build_config(), &config, false);

    assert_eq!(
        command,
        format!(
            "cd {} && \
             wasm-pack build --dev --target web --out-dir pkg/web && \
             wasm-pack build --dev --target bundler --out-dir pkg/bundler && \
             wasm-pack build --dev --target nodejs --out-dir pkg/nodejs && \
             wasm-pack build --dev --target deno --out-dir pkg/deno",
            quoted("crates/sample-lib-wasm")
        )
    );
}

/// The bare `pkg/` that the pre-fix command produced is exactly what nothing consumes;
/// asserting its absence is what distinguishes this fix from "also happens to build web".
#[test]
fn wasm_build_command_never_leaves_a_target_in_the_bare_pkg_dir() {
    let command = build_command_for(Language::Wasm, &wasm_build_config(), &wasm_config(""), true);

    assert!(
        !command.contains("--target web --out-dir pkg "),
        "no target may use wasm-pack's default bare `pkg/` out-dir: {command}"
    );
    for target in ["web", "bundler", "nodejs", "deno"] {
        assert!(
            command.contains(&format!("--target {target} --out-dir pkg/{target}")),
            "release build must still pair every target with its own out-dir: {command}"
        );
    }
    assert!(command.contains("--release"), "{command}");
    assert!(!command.contains("--dev"), "{command}");
}

/// Failure path: a crate that narrows `[crates.wasm] targets` to keep its published
/// package small must get exactly that set — not a hardcoded web+nodejs pair. This is
/// the case a hardcoded command silently gets wrong, and it also proves the emitted
/// set is genuinely read from config rather than a constant that happens to match the
/// default. Note `pkg/nodejs` is absent here, so `alef e2e` cannot resolve the package
/// for such a crate — a config-consistency problem that belongs to config validation,
/// not something the build command should paper over by force-building nodejs. ~keep
#[test]
fn wasm_build_command_honours_a_narrowed_target_set() {
    let config = wasm_config("\n[crates.wasm]\ntargets = [\"web\"]\n");

    let command = build_command_for(Language::Wasm, &wasm_build_config(), &config, false);

    assert_eq!(
        command,
        format!(
            "cd {} && wasm-pack build --dev --target web --out-dir pkg/web",
            quoted("crates/sample-lib-wasm")
        )
    );
    assert!(!command.contains("nodejs"), "{command}");
}

/// An empty target list must fail loudly and name the setting, matching the unknown-tool
/// arm, rather than emitting a dangling `cd <dir> && ` the shell rejects as a syntax error
/// that names neither alef nor the offending config key.
#[test]
fn wasm_build_command_fails_loudly_on_an_empty_target_set() {
    let config = wasm_config("\n[crates.wasm]\ntargets = []\n");

    let command = build_command_for(Language::Wasm, &wasm_build_config(), &config, false);

    assert!(command.ends_with("&& false"), "must exit non-zero: {command}");
    assert!(command.contains("[crates.wasm] targets is empty"), "{command}");
    assert!(
        !command.contains("wasm-pack build"),
        "must not emit a truncated wasm-pack invocation: {command}"
    );
}

#[test]
fn swift_build_command_uses_swift_build_with_package_path() {
    let alef_cfg: crate::core::config::NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["swift"]

[[crates]]
name = "sample-lib"
sources = ["src/lib.rs"]
"#,
    )
    .unwrap();
    let config = alef_cfg.resolve().unwrap().remove(0);
    let build_config = BuildConfig {
        tool: "swift",
        crate_suffix: "-swift",
        build_dep: BuildDependency::None,
        post_build: Vec::new(),
    };

    let swift_dir = quoted("packages/swift");
    assert_eq!(
        build_command_for(Language::Swift, &build_config, &config, false),
        format!("swift build --package-path {swift_dir}")
    );
    assert_eq!(
        build_command_for(Language::Swift, &build_config, &config, true),
        format!("swift build --package-path {swift_dir} --configuration release")
    );
}

#[test]
fn zig_build_command_uses_zig_build() {
    let alef_cfg: crate::core::config::NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["zig"]

[[crates]]
name = "sample-lib"
sources = ["src/lib.rs"]
"#,
    )
    .unwrap();
    let config = alef_cfg.resolve().unwrap().remove(0);
    let build_config = BuildConfig {
        tool: "zig",
        crate_suffix: "",
        build_dep: BuildDependency::Ffi,
        post_build: Vec::new(),
    };

    let zig_dir = quoted("packages/zig");
    assert_eq!(
        build_command_for(Language::Zig, &build_config, &config, false),
        format!("cd {zig_dir} && zig build")
    );
    assert_eq!(
        build_command_for(Language::Zig, &build_config, &config, true),
        format!("cd {zig_dir} && zig build --release=fast")
    );
}

#[test]
fn gleam_build_command_uses_gleam_build() {
    let alef_cfg: crate::core::config::NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["gleam"]

[[crates]]
name = "sample-lib"
sources = ["src/lib.rs"]
"#,
    )
    .unwrap();
    let config = alef_cfg.resolve().unwrap().remove(0);
    let build_config = BuildConfig {
        tool: "gleam",
        crate_suffix: "",
        build_dep: BuildDependency::Rustler,
        post_build: Vec::new(),
    };

    let gleam_dir = quoted("packages/gleam");
    assert_eq!(
        build_command_for(Language::Gleam, &build_config, &config, false),
        format!("cd {gleam_dir} && gleam build")
    );
    assert_eq!(
        build_command_for(Language::Gleam, &build_config, &config, true),
        format!("cd {gleam_dir} && gleam build")
    );
}

// Regression test for a real crash found while investigating the "false"
// command-substitution incident: `registry::get_backend` panics for `C`
// (it has no binding backend — it's an e2e/consumer-only target), and the
// classification loop used to call it unconditionally for every
// non-Rust language. A `[workspace] languages` list that includes "c"
// (a documented, valid e2e target) would crash the whole build instead
// of skipping C gracefully like any other backend-less language. ~keep
#[test]
fn c_language_is_skipped_gracefully_instead_of_panicking() {
    let config = ResolvedCrateConfig::default();

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        build(&config, &[Language::C], false, false)
    }));

    match result {
        Ok(build_result) => assert!(
            build_result.is_ok(),
            "C has no binding backend and must be skipped cleanly: {build_result:?}"
        ),
        Err(_) => panic!("building an unsupported binding target must not panic"),
    }
}

/// Regression test: wasm-pack derives `pkg/nodejs/package.json`'s `"name"` from the wasm
/// crate's `Cargo.toml`, not from `config.wasm_package_name()` — so every `file:` dependency
/// and `require()`/`import` specifier the wasm e2e codegen emits (which use
/// `wasm_package_name()`) would name a package the directory does not declare unless
/// something patches it after the build. This proves the patch is surgical: only the
/// `"name"` field changes, every other field (including unrelated `"name"`-shaped strings
/// elsewhere in the file) is untouched.
#[test]
fn rewrite_wasm_package_json_name_only_touches_the_name_field() {
    let dir = tempfile::tempdir().expect("failed to create temp dir");
    let manifest_path = dir.path().join("package.json");
    std::fs::write(
        &manifest_path,
        r#"{
  "name": "sample-core-wasm",
  "version": "0.1.0",
  "description": "not a name field: \"name\" appears here too",
  "main": "sample_core_wasm.js"
}
"#,
    )
    .expect("failed to write fixture package.json");

    rewrite_wasm_package_json_name(&manifest_path, "@xberg-io/sample-crate-wasm").expect("rewrite must succeed");

    let rewritten = std::fs::read_to_string(&manifest_path).expect("failed to read rewritten package.json");
    assert!(
        rewritten.contains(r#""name": "@xberg-io/sample-crate-wasm""#),
        "{rewritten}"
    );
    assert!(!rewritten.contains("sample-core-wasm"), "{rewritten}");
    assert!(
        rewritten.contains(r#""main": "sample_core_wasm.js""#),
        "unrelated fields must survive untouched: {rewritten}"
    );
    assert!(
        rewritten.contains(r#"not a name field: \"name\" appears here too"#),
        "a `\"name\"` substring inside another field's value must not be touched: {rewritten}"
    );
}

#[test]
fn rewrite_wasm_package_json_name_is_idempotent() {
    let dir = tempfile::tempdir().expect("failed to create temp dir");
    let manifest_path = dir.path().join("package.json");
    std::fs::write(&manifest_path, r#"{"name": "already-correct"}"#).expect("failed to write fixture");

    rewrite_wasm_package_json_name(&manifest_path, "already-correct").expect("rewrite must succeed");

    let rewritten = std::fs::read_to_string(&manifest_path).expect("failed to read package.json");
    assert_eq!(rewritten, r#"{"name": "already-correct"}"#);
}

/// End-to-end regression through the public `run_post_build` entry point, proving the
/// `PostBuildStep::RewriteWasmPackageName` variant is actually wired up (not just the
/// private helper it calls) and resolves `package_json_path` relative to `base_dir`
/// directly — not `base_dir.join(crate_dir)`, unlike every other `PostBuildStep` — since
/// the crate directory (`config.package_dir(Language::Wasm)`'s default-formula fallback,
/// used when `[crates.output] wasm` isn't set) must already be baked into the path by
/// the caller that constructs this step.
#[test]
fn run_post_build_rewrites_wasm_package_json_name_relative_to_base_dir() {
    let base_dir = tempfile::tempdir().expect("failed to create temp dir");
    let pkg_dir = base_dir.path().join("crates/sample-lib-wasm/pkg/nodejs");
    std::fs::create_dir_all(&pkg_dir).expect("failed to create pkg/nodejs");
    std::fs::write(pkg_dir.join("package.json"), r#"{"name": "sample-lib-wasm-crate"}"#)
        .expect("failed to write fixture package.json");

    let alef_cfg: crate::core::config::NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["wasm"]

[[crates]]
name = "sample-lib"
sources = ["src/lib.rs"]
"#,
    )
    .unwrap();
    let config = alef_cfg.resolve().unwrap().remove(0);
    let build_config = BuildConfig {
        tool: "wasm-pack",
        crate_suffix: "-wasm",
        build_dep: BuildDependency::None,
        post_build: vec![crate::core::backend::PostBuildStep::RewriteWasmPackageName {
            package_json_path: std::path::PathBuf::from("crates/sample-lib-wasm/pkg/nodejs/package.json"),
            package_name: config.wasm_package_name(),
        }],
    };

    run_post_build(
        Language::Wasm,
        &build_config,
        &config,
        base_dir.path(),
        StagingProfile::PreferOnDisk,
    )
    .expect("post-build must succeed");

    let rewritten =
        std::fs::read_to_string(pkg_dir.join("package.json")).expect("failed to read rewritten package.json");
    assert!(
        rewritten.contains(&format!(r#""name": "{}""#, config.wasm_package_name())),
        "{rewritten}"
    );
}
