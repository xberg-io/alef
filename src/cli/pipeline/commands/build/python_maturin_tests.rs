//! What the Python build command alef constructs actually says.
//!
//! Before this file, nothing in the repo asserted on it at any layer: `build_command_tests.rs`
//! covered the C#, node, Kotlin, wasm, Swift, zig and gleam arms, and `build_defaults`'s own
//! `python_uses_maturin_develop` only checked that the string contained `maturin develop`. Both
//! defects this file pins — a configured `[workspace.tools] python_package_manager` ignored, and
//! the generated crate's `extension-module` feature never activated — were therefore invisible to
//! every gate alef had.
//!
//! Two producers must agree, so both are exercised here: `build_command_for`'s `"maturin"` arm,
//! which every real `alef build` reaches (0.82.0 removed `[build_commands.python]` from
//! `alef.toml`, so there is no override path left), and `build_defaults::default_build_config`'s
//! Python arm, which is only ever compared against it directly in tests like these. ~keep

use super::*;
use crate::core::backend::{BuildConfig, BuildDependency};
use crate::core::config::python_build::PYO3_EXTENSION_MODULE_FEATURE;
use crate::core::config::{LangContext, ToolsConfig, build_defaults};

/// A directory as it is spelled *inside the emitted shell command* — a quoted word, not a bare
/// path. Expectations derive it from `quote_word` rather than restating one quoting spelling, so
/// a change to the escaping policy cannot silently repoint a command at a different directory:
/// the escaping itself is proved separately, and once, by
/// `core::config::shell::tests::quote_word_preserves_literal_shell_value`, which runs a hostile
/// value through a real shell. ~keep
fn quoted(dir: &str) -> String {
    crate::core::config::shell::quote_word(dir)
}

fn maturin_build_config() -> BuildConfig {
    BuildConfig {
        tool: "maturin",
        crate_suffix: "-py",
        build_dep: BuildDependency::None,
        post_build: Vec::new(),
    }
}

/// A resolved config for a python crate whose binding-crate output path is `output_path`.
///
/// The path is passed through verbatim, so tests point it at a temporary directory to give the
/// feature probe a real manifest to read without touching the process-wide current directory.
///
/// `output_path` is set directly on the resolved config rather than through `[crates.output]`
/// TOML: path-safety validation now rejects any absolute `[crates.output]` value at `resolve()`
/// time (it would let a hostile config value write generated files outside the project root),
/// but these tests need a real absolute tempdir for the feature probe to read a real manifest
/// from. Setting the resolved fields directly reproduces exactly what `resolve_output_paths`
/// would have written for a (now-disallowed) absolute override. ~keep
fn python_config(output_path: &str, package_manager: Option<&str>) -> crate::core::config::ResolvedCrateConfig {
    let tools_section = package_manager
        .map(|pm| format!("\n[workspace.tools]\npython_package_manager = \"{pm}\"\n"))
        .unwrap_or_default();
    let alef_cfg: crate::core::config::NewAlefConfig = toml::from_str(&format!(
        r#"
[workspace]
languages = ["python"]
{tools_section}
[[crates]]
name = "sample-lib"
sources = ["src/lib.rs"]
"#
    ))
    .unwrap();
    let mut config = alef_cfg.resolve().unwrap().remove(0);
    let output_path = std::path::PathBuf::from(output_path);
    config.explicit_output.python = Some(output_path.clone());
    config.output_paths.insert("python".to_string(), output_path);
    config
}

/// Write a binding-crate manifest that declares the extension-module feature, exactly as
/// `scaffold::languages::python` emits it, and return the crate directory it lives in.
fn scaffolded_py_crate(dir: &std::path::Path) -> String {
    let crate_dir = dir.join("crates").join("sample-lib-py");
    std::fs::create_dir_all(&crate_dir).unwrap();
    std::fs::write(
        crate_dir.join("Cargo.toml"),
        format!(
            "[package]\nname = \"sample-lib-py\"\nversion = \"0.1.0\"\n\n[lib]\ncrate-type = [\"cdylib\"]\n\n\
             [features]\n{}\n",
            crate::core::config::python_build::extension_module_feature_line()
        ),
    )
    .unwrap();
    crate_dir.to_string_lossy().into_owned()
}

/// Defect 1. The consumer's `[workspace.tools] python_package_manager` selects the environment
/// their `maturin` is pinned in; resolving a bare `maturin` off `PATH` instead builds with
/// whatever version happens to be installed globally.
#[test]
fn python_build_command_runs_maturin_through_the_configured_package_manager() {
    let dir = tempfile::tempdir().unwrap();
    let crate_dir = scaffolded_py_crate(dir.path());
    let config = python_config(&crate_dir, Some("uv"));

    let command = build_command_for(Language::Python, &maturin_build_config(), &config, false);

    assert!(
        command.starts_with("uv run --frozen --only-dev maturin develop"),
        "a configured python_package_manager must run maturin from its locked environment, \
         not off PATH: {command}"
    );
}

/// Control for defect 1: with no package manager configured the command must be exactly what it
/// is today. `ToolsConfig::python_pm` defaults to `uv`, and honouring that default here would
/// hand every consumer who never asked for uv an unrunnable command.
#[test]
fn python_build_command_is_unchanged_when_no_package_manager_is_configured() {
    let dir = tempfile::tempdir().unwrap();
    let crate_dir = dir.path().join("crates").join("sample-lib-py");
    std::fs::create_dir_all(&crate_dir).unwrap();
    let crate_dir = crate_dir.to_string_lossy().into_owned();
    let config = python_config(&crate_dir, None);

    let command = build_command_for(Language::Python, &maturin_build_config(), &config, false);

    assert_eq!(
        command,
        format!("maturin develop --manifest-path {}/Cargo.toml", quoted(&crate_dir))
    );
}

/// Defect 2. The generated crate declares `extension-module` outside its default features and the
/// generated `pyproject.toml` requests the pyo3 features it turns on -- but maturin never reads
/// that `pyproject.toml` for this invocation, so the build has to pass the feature itself.
#[test]
fn python_build_command_carries_the_feature_the_generated_manifest_declares() {
    let dir = tempfile::tempdir().unwrap();
    let crate_dir = scaffolded_py_crate(dir.path());
    let config = python_config(&crate_dir, None);

    let command = build_command_for(Language::Python, &maturin_build_config(), &config, false);
    let release = build_command_for(Language::Python, &maturin_build_config(), &config, true);

    let expected_flag = format!("--features {PYO3_EXTENSION_MODULE_FEATURE}");
    assert!(
        command.contains(&expected_flag),
        "the build must activate the feature the generated manifest defines: {command}"
    );
    assert!(
        release.contains(&expected_flag),
        "the release build shares the arm and must carry the same feature: {release}"
    );
    assert!(release.contains("--release"), "{release}");
}

/// Control for defect 2: `cargo` errors outright on a `--features` naming a feature the package
/// does not define, so the flag must be driven by the manifest rather than assumed.
#[test]
fn python_build_command_omits_features_for_a_crate_that_declares_none() {
    let dir = tempfile::tempdir().unwrap();
    let crate_dir = dir.path().join("crates").join("hand-written-py");
    std::fs::create_dir_all(&crate_dir).unwrap();
    std::fs::write(
        crate_dir.join("Cargo.toml"),
        "[package]\nname = \"hand-written-py\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    let config = python_config(&crate_dir.to_string_lossy(), None);

    let command = build_command_for(Language::Python, &maturin_build_config(), &config, false);

    assert!(
        !command.contains("--features"),
        "a crate with no extension-module feature must not be handed one: {command}"
    );
}

/// The feature probe reads a manifest, so it is only as good as the path it reads. With
/// `[crates.output] python` unset -- the common case -- that path used to be a dangling
/// `/Cargo.toml` rooted at the filesystem root, which would have made every `--features`
/// decision above examine nothing and silently answer "no feature". ~keep
#[test]
fn python_build_command_resolves_the_binding_crate_without_an_explicit_output_path() {
    let alef_cfg: crate::core::config::NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["python"]

[[crates]]
name = "sample-lib"
sources = ["src/lib.rs"]
"#,
    )
    .unwrap();
    let config = alef_cfg.resolve().unwrap().remove(0);

    let command = build_command_for(Language::Python, &maturin_build_config(), &config, false);

    assert!(
        command.contains(&format!(
            "--manifest-path {}/Cargo.toml",
            quoted("crates/sample-lib-py")
        )),
        "the maturin arm must name the binding crate scaffold writes, not a root-relative \
         `/Cargo.toml`: {command}"
    );
}

/// The other producer of this command: `build_defaults::default_build_config`'s own Python arm,
/// exercised directly rather than through `build_command_for`. It must reach the same two
/// answers.
#[test]
fn python_default_build_commands_honour_the_package_manager_and_the_feature() {
    let tools = ToolsConfig {
        python_package_manager: Some("uv".to_string()),
        ..Default::default()
    };
    let ctx = LangContext::default(&tools);

    let config = build_defaults::default_build_config(Language::Python, "packages/python", "sample-lib", &ctx);
    let build = config
        .build
        .expect("python has a default build command")
        .commands()
        .join(" ");
    let release = config
        .build_release
        .expect("python has a default build_release command")
        .commands()
        .join(" ");

    for command in [&build, &release] {
        assert!(
            command.starts_with("uv run --frozen --only-dev maturin develop"),
            "the default python build must run through the configured package manager: {command}"
        );
        assert!(
            command.contains(&format!("--features {PYO3_EXTENSION_MODULE_FEATURE}")),
            "the default python build must activate the extension-module feature: {command}"
        );
    }
    assert!(release.contains("--release"), "{release}");
    assert_eq!(
        config.precondition.as_deref(),
        Some("command -v uv >/dev/null 2>&1"),
        "readiness must check the tool that actually runs the build -- maturin lives in the \
         locked environment and need not be on PATH at all"
    );
}

/// Control for the defaults path: an unset key leaves the invocation on `PATH` and leaves the
/// readiness check on maturin itself.
#[test]
fn python_default_build_commands_stay_on_path_without_a_configured_package_manager() {
    let tools = ToolsConfig::default();
    let ctx = LangContext::default(&tools);

    let config = build_defaults::default_build_config(Language::Python, "packages/python", "sample-lib", &ctx);
    let build = config
        .build
        .expect("python has a default build command")
        .commands()
        .join(" ");

    let expected = format!(
        "maturin develop --manifest-path crates/sample-lib-py/Cargo.toml \
         --features {PYO3_EXTENSION_MODULE_FEATURE}"
    );
    assert_eq!(build, expected);
    assert_eq!(
        config.precondition.as_deref(),
        Some("command -v maturin >/dev/null 2>&1")
    );
}
