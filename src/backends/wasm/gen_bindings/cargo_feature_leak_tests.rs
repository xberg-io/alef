//! Regression coverage for the generated wasm crate's `[features] default = [...]` switching a
//! core-crate feature back on that its own core dependency line had just switched off.
//!
//! `gen_cargo_toml` writes two things that must agree about the same core crate:
//!
//! 1. the dependency line, `default-features = false, features = [...]` whenever
//!    `features_for_language(Language::Wasm)` is non-empty, and
//! 2. the `[features]` table, whose `default = [...]` row is the intersection of the
//!    cfg-referenced feature names with `codegen::cfg::enabled_features_for_language`.
//!
//! `enabled_features_for_language` unions in the core crate's own declared `default = [...]` when
//! `core_default_features_active` says the dependency edge keeps them -- and that predicate used
//! to answer `true` for every language but R, wasm included. So a core crate declaring
//! `default = ["native-http"]` produced `default = ["native-http", ...]` in the wasm manifest,
//! whose forwarding row `native-http = ["<core>/native-http"]` re-enabled the exact feature the
//! dependency line had turned off. In a downstream consumer that feature pulls in reqwest and
//! tokio's native I/O stack, and the wasm32 build died on `This wasm target is unsupported by
//! mio. If using Tokio, disable the net feature.` ~keep
//!
//! Split into its own file so `tests.rs` (984 lines) stays under the `file-modularization` cap.

use super::cargo::gen_cargo_toml;
use crate::core::config::{Language, NewAlefConfig, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, TypeDef};

/// The shape that broke, reduced to its two features: a native-only one the core crate turns on by
/// default (pulling tokio's `net`, hence mio), and the wasm-only replacement the binding asks for.
const CORE_FEATURES_BODY: &str = r#"default = ["native-http"]
native-http = ["dep:tokio", "tokio/net"]
wasm-http = ["dep:gloo-timers"]
"#;

fn write_core_manifest(dir: &std::path::Path) {
    let core_dir = dir.join("crates").join("test-lib");
    std::fs::create_dir_all(&core_dir).expect("create core crate dir");
    std::fs::write(
        core_dir.join("Cargo.toml"),
        format!("[package]\nname = \"test-lib\"\n\n[features]\n{CORE_FEATURES_BODY}"),
    )
    .expect("write core Cargo.toml");
}

/// `wasm_table` is the body of `[crates.wasm]` -- empty for "configure no features at all".
fn config_with_core_manifest(dir: &std::path::Path, wasm_table: &str) -> ResolvedCrateConfig {
    write_core_manifest(dir);
    let toml_src = format!(
        r#"
[workspace]
languages = ["wasm"]
[[crates]]
name = "test-lib"
sources = ["crates/test-lib/src/lib.rs"]
workspace_root = "{root}"
[crates.wasm]
{wasm_table}
"#,
        // TOML strings treat `\` as an escape, so a Windows temp path must be forward-slashed
        // before it is interpolated; Cargo and alef both accept `/` separators on every host. ~keep
        root = dir.display().to_string().replace('\\', "/"),
    );
    let cfg: NewAlefConfig = toml::from_str(&toml_src).expect("valid alef config");
    cfg.resolve().expect("resolve").remove(0)
}

/// One type gated on both feature names, so `collect_cfg_features` discovers `native-http` and
/// `wasm-http` and the manifest grows a forwarding row for each.
fn api_gated_on_both_features() -> ApiSurface {
    ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Client".to_string(),
            rust_path: "test_lib::Client".to_string(),
            cfg: Some(r#"any(feature = "native-http", feature = "wasm-http")"#.to_string()),
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn features_block(cargo_toml: &str) -> String {
    let start = cargo_toml
        .find("[features]\n")
        .unwrap_or_else(|| panic!("no [features] table in:\n{cargo_toml}"));
    let body = &cargo_toml[start..];
    let end = body
        .find("\n\n")
        .unwrap_or_else(|| panic!("unterminated [features] table in:\n{cargo_toml}"));
    body[..end].to_string()
}

fn dependency_line(cargo_toml: &str, name: &str) -> String {
    let prefix = format!("{name} = ");
    cargo_toml
        .lines()
        .find(|line| line.starts_with(&prefix))
        .unwrap_or_else(|| panic!("no `{name}` dependency line in:\n{cargo_toml}"))
        .to_string()
}

/// THE LEAK. Reverting `core_default_features_active`'s `Language::Wasm` arm makes this fail with
/// `default = ["native-http", "wasm-http"]` -- the byte-for-byte shape a downstream crate shipped, and the
/// one that dragged mio into a wasm32 build.
#[test]
fn wasm_features_default_excludes_a_core_default_the_dependency_line_disabled() {
    let dir = tempfile::tempdir().expect("tempdir");
    let config = config_with_core_manifest(dir.path(), r#"features = ["wasm-http"]"#);
    let cargo_toml = gen_cargo_toml(&api_gated_on_both_features(), &config);

    assert_eq!(
        features_block(&cargo_toml),
        "[features]\ndefault = [\"wasm-http\"]\n\
         native-http = [\"test-lib/native-http\"]\n\
         wasm-http = [\"test-lib/wasm-http\"]",
        "`native-http` must stay declared-but-off: the core dep line already turned it off, and \
         defaulting it back on re-enables the native tokio stack on wasm32:\n{cargo_toml}"
    );
    toml::from_str::<toml::Value>(&cargo_toml).expect("generated wasm Cargo.toml must be valid TOML");
}

/// The other half of the same contract, pinned separately so a regression tells you WHICH of the
/// two disagreeing emissions moved. This line is already correct today; it is the `[features]`
/// table that contradicted it, so this must not change when the fix is reverted -- which is
/// exactly what makes the pair diagnostic.
#[test]
fn wasm_core_dependency_line_disables_default_features_and_names_only_the_wasm_set() {
    let dir = tempfile::tempdir().expect("tempdir");
    let config = config_with_core_manifest(dir.path(), r#"features = ["wasm-http"]"#);
    let cargo_toml = gen_cargo_toml(&api_gated_on_both_features(), &config);

    let core_path = config.core_crate_dep_path_for_language(&super::wasm_output_layout(&config).root, Language::Wasm);
    assert_eq!(
        dependency_line(&cargo_toml, "test-lib"),
        format!(r#"test-lib = {{ path = "{core_path}", default-features = false, features = ["wasm-http"] }}"#),
        "the core dep edge must suppress defaults and request exactly the configured wasm set:\n{cargo_toml}"
    );
}

/// DO-NOT-OVER-NARROW CONTROL. With no wasm feature list configured, `gen_cargo_toml` emits a
/// plain dependency line with no `default-features = false`, so the core crate's `default` really
/// IS active and the `[features]` table must keep saying so. A "fix" that suppressed core
/// defaults for wasm unconditionally would fail here with `default = ["wasm-http"]` while the
/// dependency line it must agree with still leaves `native-http` on -- the same disagreement,
/// mirrored.
#[test]
fn wasm_features_default_keeps_core_defaults_when_no_wasm_feature_list_is_configured() {
    let dir = tempfile::tempdir().expect("tempdir");
    let config = config_with_core_manifest(dir.path(), "");
    let cargo_toml = gen_cargo_toml(&api_gated_on_both_features(), &config);

    let core_path = config.core_crate_dep_path_for_language(&super::wasm_output_layout(&config).root, Language::Wasm);
    assert_eq!(
        dependency_line(&cargo_toml, "test-lib"),
        format!(r#"test-lib = {{ path = "{core_path}" }}"#),
        "sanity: with nothing configured the dep line must leave the core defaults alone:\n{cargo_toml}"
    );
    assert_eq!(
        features_block(&cargo_toml),
        "[features]\ndefault = [\"native-http\"]\n\
         native-http = [\"test-lib/native-http\"]\n\
         wasm-http = [\"test-lib/wasm-http\"]",
        "a core default that IS active on this dep edge must still be defaulted on:\n{cargo_toml}"
    );
}

// The native halves of this contract live where their emitters do, because `scaffold_python_cargo`
// is not reachable from here: `scaffold::languages::python::scaffold_python_cargo` manifest control
// in `scaffold::languages::python`, and the `enabled_features_for_language` control in
// `codegen::cfg::tests::enabled_features`. ~keep
