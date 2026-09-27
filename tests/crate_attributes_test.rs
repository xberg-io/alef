//! Tests for `[[crates]] crate_attributes` — per-crate custom inner attribute
//! injection (`#![...]`) into every generated Rust `lib.rs` for that crate, across
//! every Rust-emitting backend (ffi, jni, node, python, php, wasm, ruby, elixir, R,
//! swift, dart).
//!
//! Mirrors `tests/workspace_extra_clippy_allows_test.rs`, scoped one level down:
//! `extra_clippy_allows` is a workspace-wide default: `crate_attributes` is set
//! per `[[crates]]` entry.

use alef::backends::ffi::FfiBackend;
use alef::backends::jni::JniBackend;
use alef::backends::napi::NapiBackend;
use alef::core::backend::Backend;
use alef::core::config::{NewAlefConfig, ResolvedCrateConfig};
use alef::core::ir::ApiSurface;

// ---------------------------------------------------------------------------

/// Build a single resolved crate from an inline `alef.toml`. `languages` is the
/// raw (already-quoted) contents of the `[workspace] languages` array, e.g.
/// `r#""ffi""#`. `crate_body` is appended verbatim inside the `[[crates]]` entry
/// (and may include trailing `[crates.*]` sub-tables).
fn make_config(languages: &str, crate_body: &str) -> ResolvedCrateConfig {
    let toml_src = format!(
        r#"
[workspace]
languages = [{languages}]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
{crate_body}
"#
    );
    let cfg: NewAlefConfig = toml::from_str(&toml_src).expect("config parses");
    cfg.resolve().expect("config resolves").remove(0)
}

fn try_resolve(languages: &str, crate_body: &str) -> Result<ResolvedCrateConfig, alef::core::config::ResolveError> {
    let toml_src = format!(
        r#"
[workspace]
languages = [{languages}]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
{crate_body}
"#
    );
    let cfg: NewAlefConfig = toml::from_str(&toml_src).expect("config parses");
    cfg.resolve().map(|mut v| v.remove(0))
}

fn empty_api() -> ApiSurface {
    ApiSurface {
        unresolved_modules: Vec::new(),
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        ..ApiSurface::default()
    }
}

fn lib_rs_content(files: &[alef::core::backend::GeneratedFile]) -> &str {
    &files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .expect("lib.rs present")
        .content
}

// ---------------------------------------------------------------------------
// Config parsing / defaulting
// ---------------------------------------------------------------------------

#[test]
fn crate_config_parses_crate_attributes_in_configured_order() {
    let cfg = make_config(
        r#""ffi""#,
        r#"crate_attributes = ["recursion_limit = \"256\"", "feature(async_closure)"]"#,
    );
    assert_eq!(
        cfg.crate_attributes,
        vec![
            "recursion_limit = \"256\"".to_string(),
            "feature(async_closure)".to_string()
        ]
    );
}

#[test]
fn crate_attributes_defaults_to_empty() {
    let cfg = make_config(r#""ffi""#, "");
    assert!(cfg.crate_attributes.is_empty());
}

// ---------------------------------------------------------------------------
// format_crate_attributes
// ---------------------------------------------------------------------------

#[test]
fn format_crate_attributes_returns_empty_vec_when_unset() {
    let result = alef::codegen::shared::format_crate_attributes(&[]);
    assert!(result.is_empty(), "empty input must format to no attributes");
}

#[test]
fn format_crate_attributes_preserves_configured_order() {
    let attrs = vec![
        "recursion_limit = \"256\"".to_string(),
        "feature(async_closure)".to_string(),
    ];
    let result = alef::codegen::shared::format_crate_attributes(&attrs);
    assert_eq!(
        result,
        vec![
            "recursion_limit = \"256\"".to_string(),
            "feature(async_closure)".to_string()
        ],
        "entries must not be merged or reordered, unlike format_extra_clippy_allows"
    );
}

// ---------------------------------------------------------------------------
// ffi backend (previously unwired — no extras hook of any kind)
// ---------------------------------------------------------------------------

#[test]
fn ffi_backend_emits_crate_attribute_before_any_item() {
    let cfg = make_config(r#""ffi""#, r#"crate_attributes = ["recursion_limit = \"256\""]"#);
    let api = empty_api();
    let files = FfiBackend.generate_bindings(&api, &cfg).expect("ffi generates");
    let content = lib_rs_content(&files);

    assert!(
        content.contains("#![recursion_limit = \"256\"]\n"),
        "expected the exact inner attribute line, got:\n{content}"
    );

    let attr_pos = content
        .find("#![recursion_limit = \"256\"]")
        .expect("attribute present");
    let first_use_pos = content.find("use ").expect("a use import exists");
    assert!(
        attr_pos < first_use_pos,
        "inner attribute must precede every use/item (rustc E0753 otherwise)"
    );
}

#[test]
fn ffi_backend_emits_multiple_crate_attributes_in_order() {
    let cfg = make_config(
        r#""ffi""#,
        r#"crate_attributes = ["recursion_limit = \"256\"", "feature(async_closure)"]"#,
    );
    let api = empty_api();
    let files = FfiBackend.generate_bindings(&api, &cfg).expect("ffi generates");
    let content = lib_rs_content(&files);

    let recursion_pos = content
        .find("#![recursion_limit = \"256\"]")
        .expect("recursion_limit attribute present");
    let feature_pos = content
        .find("#![feature(async_closure)]")
        .expect("feature attribute present");
    assert!(
        recursion_pos < feature_pos,
        "attributes must appear in configured order"
    );
}

#[test]
fn ffi_backend_no_crate_attribute_when_unset() {
    let cfg = make_config(r#""ffi""#, "");
    let api = empty_api();
    let files = FfiBackend.generate_bindings(&api, &cfg).expect("ffi generates");
    let content = lib_rs_content(&files);

    assert!(
        !content.contains("recursion_limit"),
        "no-config baseline must be byte-identical to output with no crate_attributes set"
    );
}

// ---------------------------------------------------------------------------
// jni backend (previously unwired — Jinja-template splice path)
// ---------------------------------------------------------------------------

#[test]
fn jni_backend_emits_crate_attribute() {
    let cfg = make_config(
        r#""kotlin_android", "jni""#,
        "crate_attributes = [\"recursion_limit = \\\"256\\\"\"]\n\n[crates.kotlin_android]\n",
    );
    let api = empty_api();
    let files = JniBackend.generate_bindings(&api, &cfg).expect("jni generates");
    let content = lib_rs_content(&files);

    assert!(
        content.contains("#![recursion_limit = \"256\"]\n"),
        "expected the exact inner attribute line, got:\n{content}"
    );
    let attr_pos = content
        .find("#![recursion_limit = \"256\"]")
        .expect("attribute present");
    let use_pos = content.find("use ").expect("a use import exists");
    assert!(attr_pos < use_pos, "inner attribute must precede every use/item");
}

#[test]
fn jni_backend_no_crate_attribute_when_unset() {
    let cfg = make_config(r#""kotlin_android", "jni""#, "\n[crates.kotlin_android]\n");
    let api = empty_api();
    let files = JniBackend.generate_bindings(&api, &cfg).expect("jni generates");
    let content = lib_rs_content(&files);

    assert!(
        !content.contains("recursion_limit"),
        "no-config baseline must be byte-identical to output with no crate_attributes set"
    );
}

// ---------------------------------------------------------------------------
// Composability with extra_clippy_allows (workspace-level, pre-existing knob)
// ---------------------------------------------------------------------------

#[test]
fn napi_backend_composes_crate_attributes_with_extra_clippy_allows() {
    let toml_src = r#"
[workspace]
languages = ["node"]
extra_clippy_allows = ["single_match"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
crate_attributes = ["recursion_limit = \"256\""]
"#;
    let cfg: NewAlefConfig = toml::from_str(toml_src).expect("config parses");
    let resolved = cfg.resolve().expect("config resolves").remove(0);
    let api = empty_api();
    let files = NapiBackend.generate_bindings(&api, &resolved).expect("napi generates");
    let content = lib_rs_content(&files);

    assert!(
        content.contains("#![allow(clippy::single_match)]\n"),
        "extra_clippy_allows attribute must still be emitted, got:\n{content}"
    );
    assert!(
        content.contains("#![recursion_limit = \"256\"]\n"),
        "crate_attributes attribute must be emitted, got:\n{content}"
    );

    let clippy_pos = content.find("#![allow(clippy::single_match)]").unwrap();
    let recursion_pos = content.find("#![recursion_limit").unwrap();
    assert!(
        clippy_pos < recursion_pos,
        "extra_clippy_allows must be spliced before crate_attributes, neither clobbering the other"
    );
}

// ---------------------------------------------------------------------------
// Malformed entries are rejected loudly at config-resolve time
// ---------------------------------------------------------------------------

#[test]
fn malformed_crate_attribute_full_syntax_wrapper_is_rejected() {
    // Not a raw string: the TOML entry embeds `"#` (from `#![...]`), which would
    // prematurely close an `r#"..."#` raw-string literal.
    let crate_body = "crate_attributes = [\"#![recursion_limit = \\\"256\\\"]\"]";
    let err = try_resolve(r#""ffi""#, crate_body).expect_err("full #![...] syntax must be rejected");
    assert!(
        err.to_string().contains("must not include the `#![...]` wrapper"),
        "unexpected error message: {err}"
    );
}

#[test]
fn malformed_crate_attribute_empty_string_is_rejected() {
    let err = try_resolve(r#""ffi""#, r#"crate_attributes = [""]"#).expect_err("empty entry must be rejected");
    assert!(
        err.to_string().contains("empty or whitespace-only"),
        "unexpected error message: {err}"
    );
}

#[test]
fn malformed_crate_attribute_invalid_path_is_rejected() {
    let err = try_resolve(r#""ffi""#, r#"crate_attributes = ["not a real lint; evil"]"#)
        .expect_err("garbage attribute body must be rejected");
    assert!(
        err.to_string().contains("is malformed"),
        "unexpected error message: {err}"
    );
}

#[test]
fn malformed_crate_attribute_multiline_is_rejected() {
    let err = try_resolve(
        r#""ffi""#,
        "crate_attributes = [\"recursion_limit = \\\"256\\\"\\nfeature(x)\"]",
    )
    .expect_err("multi-line entry must be rejected");
    assert!(
        err.to_string().contains("must not contain a newline"),
        "unexpected error message: {err}"
    );
}

#[test]
fn well_formed_crate_attribute_with_nested_call_syntax_is_accepted() {
    let cfg = make_config(r#""ffi""#, r#"crate_attributes = ["feature(async_closure)"]"#);
    assert_eq!(cfg.crate_attributes, vec!["feature(async_closure)".to_string()]);
}
