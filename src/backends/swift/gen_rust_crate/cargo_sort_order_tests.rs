//! `cargo sort --check` TABLE-order conformance for the Swift bridge crate's `Cargo.toml`.
//!
//! `cargo.rs`'s own test module already drives [`emit_cargo_toml`]'s DEPENDENCY-KEY order
//! through `cargo_sort_order::assert_dependency_keys_sorted` (see
//! `cargo_toml_merges_and_sorts_core_and_ffi_target_blocks_together`), but nothing asserted the
//! table HEADER order (`[package]` < `[lib]` < `[features]` < `[dependencies]` <
//! `[build-dependencies]` < `[lints.*]`) against the real emitter output via
//! `cargo_sort_order::assert_canonical_table_order`. Split into its own file rather than grown
//! onto `cargo.rs`, which is already at the file-modularization line cap. ~keep

use super::cargo::emit_cargo_toml;
use crate::core::config::languages::SwiftTargetDepOverride;
use crate::core::ir::{ApiSurface, EnumDef, EnumVariant};
use crate::test_support::cargo_sort_order::{assert_canonical_table_order, assert_dependency_keys_sorted};

/// A plain manifest with only the always-emitted tables.
#[test]
fn plain_manifest_is_cargo_sort_clean() {
    let api = ApiSurface::default();
    let content = emit_cargo_toml(
        "sample-lib",
        "sample_lib",
        "sample-lib",
        "0.1.0",
        "0.1.0",
        "0.1.0",
        "../..",
        &[],
        "",
        "MIT",
        false,
        &[],
        &api,
        &[],
        "sample-lib-ffi",
        "../../../crates/sample-lib-ffi",
        &[],
        &[],
        "",
        &Default::default(),
    );
    assert_canonical_table_order("swift Cargo.toml", &content);
    assert!(
        assert_dependency_keys_sorted("swift Cargo.toml", &content) > 0,
        "expected at least one dependency key to have been compared:\n{content}"
    );
}

/// A manifest carrying every optional table this emitter can produce: `[features]` (cfg-gated
/// enum variant), merged core+FFI `[target.'cfg(...)'.dependencies]` overrides, and the trailing
/// `[build-dependencies]` / `[lints.rust]` tables.
#[test]
fn manifest_with_features_and_merged_target_overrides_is_cargo_sort_clean() {
    let api = ApiSurface {
        enums: vec![EnumDef {
            name: "ImageOutputFormat".to_string(),
            variants: vec![EnumVariant {
                name: "Heic".to_string(),
                cfg: Some("feature = \"heic\"".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    };
    let core_overrides = vec![SwiftTargetDepOverride {
        cfg: "target_os = \"android\"".to_string(),
        features: vec!["android-target".to_string()],
        default_features: false,
    }];
    let ffi_overrides = vec![SwiftTargetDepOverride {
        cfg: r#"any(target_os = "ios", target_os = "android")"#.to_string(),
        features: vec!["android-target".to_string()],
        default_features: false,
    }];
    let content = emit_cargo_toml(
        "acme",
        "acme",
        "acme",
        "1.1.0",
        "0.1.59",
        "0.1.59",
        "../../..",
        &[],
        "",
        "MIT",
        false,
        &core_overrides,
        &api,
        &[],
        "acme-ffi",
        "../../../crates/acme-ffi",
        &["full-no-heic".to_string()],
        &ffi_overrides,
        "",
        &Default::default(),
    );
    assert_canonical_table_order("swift Cargo.toml (features + merged target overrides)", &content);
    assert!(
        assert_dependency_keys_sorted("swift Cargo.toml (features + merged target overrides)", &content) > 0,
        "expected at least one dependency key to have been compared:\n{content}"
    );
    toml::from_str::<toml::Value>(&content).expect("generated Cargo.toml must be valid TOML");
}
