//! Tests for `[workspace] extra_clippy_allows` — the configurable clippy allow-list
//! extension in generated Rust binding files.

use alef::backends::napi::NapiBackend;
use alef::backends::pyo3::Pyo3Backend;
use alef::core::backend::Backend;
use alef::core::config::{NewAlefConfig, ResolvedCrateConfig};
use alef::core::ir::ApiSurface;

// ---------------------------------------------------------------------------
// Config helpers
// ---------------------------------------------------------------------------

fn make_config_with_extras(extras: &[&str]) -> ResolvedCrateConfig {
    let extra_list: Vec<String> = extras.iter().map(|s| format!("\"{s}\"")).collect();
    let extra_toml = if extras.is_empty() {
        String::new()
    } else {
        format!("extra_clippy_allows = [{}]", extra_list.join(", "))
    };
    let toml_src = format!(
        r#"
[workspace]
languages = ["python", "node"]
{extra_toml}

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.python]
module_name = "_test_lib"
"#
    );
    let cfg: NewAlefConfig = toml::from_str(&toml_src).expect("config parses");
    cfg.resolve().expect("config resolves").remove(0)
}

fn empty_api() -> ApiSurface {
    ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        ..ApiSurface::default()
    }
}

// ---------------------------------------------------------------------------
// Parsing / config tests
// ---------------------------------------------------------------------------

#[test]
fn workspace_parses_extra_clippy_allows() {
    let cfg = make_config_with_extras(&["single_match", "clippy::collapsible_match"]);
    assert_eq!(cfg.extra_clippy_allows.len(), 2);
    assert_eq!(cfg.extra_clippy_allows[0], "single_match");
    assert_eq!(cfg.extra_clippy_allows[1], "clippy::collapsible_match");
}

#[test]
fn workspace_extra_clippy_allows_defaults_to_empty() {
    let cfg = make_config_with_extras(&[]);
    assert!(cfg.extra_clippy_allows.is_empty());
}

// ---------------------------------------------------------------------------
// Helper unit tests
// ---------------------------------------------------------------------------

#[test]
fn format_extra_clippy_allows_returns_none_when_empty() {
    let result = alef::codegen::shared::format_extra_clippy_allows(&[]);
    assert!(result.is_none(), "empty extras should return None");
}

#[test]
fn format_extra_clippy_allows_normalises_bare_names() {
    let extras = vec!["single_match".to_string(), "collapsible_match".to_string()];
    let result = alef::codegen::shared::format_extra_clippy_allows(&extras).unwrap();
    assert_eq!(result, "allow(clippy::single_match, clippy::collapsible_match)");
}

#[test]
fn format_extra_clippy_allows_accepts_prefixed_names() {
    let extras = vec!["clippy::single_match".to_string()];
    let result = alef::codegen::shared::format_extra_clippy_allows(&extras).unwrap();
    assert_eq!(result, "allow(clippy::single_match)");
}

#[test]
fn format_extra_clippy_allows_deduplicates() {
    let extras = vec!["single_match".to_string(), "clippy::single_match".to_string()];
    let result = alef::codegen::shared::format_extra_clippy_allows(&extras).unwrap();
    // "single_match" normalises to "clippy::single_match", which is a dup → 1 entry
    assert_eq!(result, "allow(clippy::single_match)");
}

// ---------------------------------------------------------------------------
// Integration: extras appear in pyo3 generated output
// ---------------------------------------------------------------------------

#[test]
fn pyo3_backend_emits_extra_allows() {
    let config = make_config_with_extras(&["single_match", "collapsible_match"]);
    let api = empty_api();
    let files = Pyo3Backend.generate_bindings(&api, &config).expect("pyo3 generates");
    let lib_rs = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .expect("lib.rs present");
    assert!(
        lib_rs.content.contains("clippy::single_match"),
        "extra allow 'single_match' must appear in pyo3 lib.rs"
    );
    assert!(
        lib_rs.content.contains("clippy::collapsible_match"),
        "extra allow 'collapsible_match' must appear in pyo3 lib.rs"
    );
}

#[test]
fn pyo3_backend_no_extra_attr_when_empty() {
    let config_no_extras = make_config_with_extras(&[]);
    let config_with_extras = make_config_with_extras(&["single_match"]);
    let api = empty_api();

    let files_no = Pyo3Backend
        .generate_bindings(&api, &config_no_extras)
        .expect("pyo3 generates");
    let files_yes = Pyo3Backend
        .generate_bindings(&api, &config_with_extras)
        .expect("pyo3 generates");

    let lib_no = files_no
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .unwrap();
    let lib_yes = files_yes
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .unwrap();

    // No extras → no `clippy::single_match` in output
    assert!(
        !lib_no.content.contains("clippy::single_match"),
        "output without extras must not contain single_match"
    );
    // With extras → the lint appears
    assert!(
        lib_yes.content.contains("clippy::single_match"),
        "output with extras must contain single_match"
    );
    // Without extras → byte-identical to baseline (no extra allow line at all)
    assert!(
        !lib_no.content.contains("allow(clippy::single_match"),
        "no-config baseline must not have extra allow line"
    );
}

#[test]
fn napi_backend_emits_extra_allows() {
    let config = make_config_with_extras(&["single_match"]);
    let api = empty_api();
    let files = NapiBackend.generate_bindings(&api, &config).expect("napi generates");
    let lib_rs = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .expect("lib.rs present");
    assert!(
        lib_rs.content.contains("clippy::single_match"),
        "extra allow must appear in napi lib.rs"
    );
}
