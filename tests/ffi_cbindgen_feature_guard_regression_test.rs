//! Regression coverage for the missing-`#if`-guard defect: alef emits
//! `#[cfg(feature = "...")]` on FFI exports and a matching `[defines]` entry in
//! the generated `cbindgen.toml`, but a stray pair of escaped quotes in the
//! `[defines]` *value* (`"feature = \"tokenizer\""` instead of
//! `"feature = tokenizer"`) meant cbindgen's own key matcher never recognized
//! the mapping, so gated exports were declared unconditionally in the header.
//!
//! This is deliberately an *integration* test that runs the real `cbindgen`
//! crate (the same one generated FFI crates depend on) against alef's own
//! generated `lib.rs` + `cbindgen.toml`, rather than asserting on the TOML
//! text alone: a symbol-name-only check (does the function appear in the
//! header at all) passes whether or not it is wrapped in the `#if` guard, so
//! it would not have caught this defect. See `assert_declared_inside_guard`
//! and `assert_declared_unconditionally` below — both assert on guard
//! placement, not mere presence.

use alef::backends::ffi::FfiBackend;
use alef::core::backend::Backend;
use alef::core::config::ResolvedCrateConfig;
use alef::core::config::new_config::NewAlefConfig;
use alef::core::ir::{ApiSurface, FunctionDef};

fn resolved_one(toml: &str) -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(toml).unwrap();
    cfg.resolve().unwrap().remove(0)
}

/// Assert `needle` (an exported symbol name) is declared strictly inside a
/// `#if defined(macro_name)` ... `#endif` span in `header`.
fn assert_declared_inside_guard(header: &str, macro_name: &str, needle: &str) {
    let if_marker = format!("#if defined({macro_name})");
    let if_pos = header
        .find(&if_marker)
        .unwrap_or_else(|| panic!("expected `{if_marker}` guard in generated header:\n{header}"));
    let endif_pos = header[if_pos..]
        .find("#endif")
        .map(|offset| if_pos + offset)
        .unwrap_or_else(|| panic!("expected a closing #endif for `{if_marker}` in generated header:\n{header}"));
    let needle_pos = header
        .find(needle)
        .unwrap_or_else(|| panic!("expected `{needle}` to be declared in generated header:\n{header}"));
    assert!(
        needle_pos > if_pos && needle_pos < endif_pos,
        "expected `{needle}` to be declared between `{if_marker}` and its `#endif`, \
         but it was declared outside that guard. header:\n{header}"
    );
}

/// Assert `needle` is declared outside of every `#if defined(...)` ... `#endif`
/// span in `header` (i.e. unconditionally exported).
fn assert_declared_unconditionally(header: &str, needle: &str) {
    let needle_pos = header
        .find(needle)
        .unwrap_or_else(|| panic!("expected `{needle}` to be declared in generated header:\n{header}"));

    let mut search_from = 0;
    while let Some(relative_if) = header[search_from..].find("#if defined(") {
        let if_pos = search_from + relative_if;
        let Some(relative_endif) = header[if_pos..].find("#endif") else {
            break;
        };
        let endif_pos = if_pos + relative_endif;
        assert!(
            !(needle_pos > if_pos && needle_pos < endif_pos),
            "expected `{needle}` to be declared unconditionally, but it fell inside \
             a `#if defined(...)` guard. header:\n{header}"
        );
        search_from = endif_pos + "#endif".len();
    }
}

#[test]
fn cbindgen_wraps_feature_gated_export_in_matching_if_defined_guard() {
    let config = resolved_one(
        r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "sample-lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "sample"
"#,
    );

    let mut api = ApiSurface {
        unresolved_modules: Vec::new(),
        crate_name: "sample-lib".to_string(),
        version: "1.0.0".to_string(),
        ..ApiSurface::default()
    };
    api.functions.push(FunctionDef {
        name: "render".to_string(),
        rust_path: "sample_lib::render".to_string(),
        cfg: Some(r#"feature = "document-render""#.to_string()),
        ..FunctionDef::default()
    });
    api.functions.push(FunctionDef {
        name: "ping".to_string(),
        rust_path: "sample_lib::ping".to_string(),
        ..FunctionDef::default()
    });

    let files = FfiBackend
        .generate_bindings(&api, &config)
        .expect("FFI backend must generate bindings");
    let lib_rs = &files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap().content;
    let cbindgen_toml = &files
        .iter()
        .find(|f| f.path.ends_with("cbindgen.toml"))
        .unwrap()
        .content;

    // The exact defect: this key must carry the bare, unquoted feature name.
    // cbindgen's `DefineKey::load` (bindgen/ir/cfg.rs) splits the `[defines]`
    // key on `=` and only trims whitespace — it does not strip quotes — so a
    // quoted value here never matches the unquoted `cfg_value` cbindgen reads
    // from a parsed `#[cfg(feature = "...")]` attribute via `LitStr::value()`.
    assert!(
        cbindgen_toml.contains(r#""feature = document-render" = "SAMPLE_FEATURE_DOCUMENT_RENDER""#),
        "cbindgen.toml [defines] key must be the unquoted `feature = <name>` form cbindgen matches, got:\n{cbindgen_toml}"
    );
    assert!(
        !cbindgen_toml.contains(r#"\""#),
        "cbindgen.toml [defines] must not contain escaped quotes in the feature-cfg mapping, got:\n{cbindgen_toml}"
    );

    let dir = tempfile::tempdir().expect("temp dir for cbindgen run");
    std::fs::write(dir.path().join("lib.rs"), lib_rs).expect("write generated lib.rs");
    std::fs::write(dir.path().join("cbindgen.toml"), cbindgen_toml).expect("write generated cbindgen.toml");

    let cbindgen_config = cbindgen::Config::from_file(dir.path().join("cbindgen.toml"))
        .expect("alef-generated cbindgen.toml must itself be a valid cbindgen config");

    let bindings = cbindgen::Builder::new()
        .with_src(dir.path().join("lib.rs"))
        .with_config(cbindgen_config)
        .generate()
        .expect("cbindgen must be able to parse alef's generated FFI source");

    let header_path = dir.path().join(config.ffi_header_name());
    assert!(
        bindings.write_to_file(&header_path),
        "cbindgen must write a new header file"
    );
    let header = std::fs::read_to_string(&header_path).expect("cbindgen must write the header");

    assert_declared_inside_guard(&header, "SAMPLE_FEATURE_DOCUMENT_RENDER", "sample_render");
    assert_declared_unconditionally(&header, "sample_ping");
}
