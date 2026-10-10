//! Regression for alef-task #374: an `excluded_default_features` name that gates no item in the
//! extracted API surface (e.g. a Cargo-only feature that only affects a dependency's `build.rs`
//! linking, such as `libheif-sys` via `heic`) is never discovered by
//! `shared_cfg::collect_cfg_features`, which walks `#[cfg(feature = "X")]` attributes on IR
//! nodes. The `[features]` table was built exclusively from that discovery set, so a
//! config-only name never got its promised opt-in forwarding entry (`<name> = ["<core>/<name>"]`)
//! at all -- breaking `cargo build -p <crate>-swift --features <name>` on desktop, exactly the
//! escape hatch `excluded_default_features` documents as always available.
//! `manifest_with_features_and_merged_target_overrides_is_cargo_sort_clean` in
//! `cargo_sort_order_tests.rs` does not catch this: it excludes `heic` from an enum variant
//! `#[cfg(feature = "heic")]`, so `heic` IS discoverable there and only exercises the
//! already-working half.

use super::cargo::emit_cargo_toml;
use crate::core::ir::ApiSurface;

#[test]
fn cargo_toml_forwards_excluded_feature_not_referenced_by_any_cfg_attribute() {
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
        &["heic".to_string()],
        "sample-lib-ffi",
        "../../../crates/sample-lib-ffi",
        &[],
        &[],
        "",
        &Default::default(),
    );

    assert!(
        content.contains("[features]"),
        "a config-only excluded_default_features name must still produce a [features] table:\n{content}"
    );
    assert!(
        content.contains(r#"heic = ["sample_lib/heic"]"#),
        "a config-only excluded_default_features name (not referenced by any #[cfg(feature = ...)] \
         in the API surface) must still get a forwarding entry so `cargo build --features heic` \
         keeps working:\n{content}"
    );
    let default_line = content
        .lines()
        .find(|l| l.starts_with("default = ["))
        .expect("default = [...] line must be emitted");
    assert!(
        !default_line.contains("\"heic\""),
        "default = [...] must NOT contain excluded `heic`; got: {default_line}"
    );
    toml::from_str::<toml::Value>(&content).expect("generated Cargo.toml must be valid TOML");
}
