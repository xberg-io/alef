//! Coverage for [`cfg_default_and_forwarding_lines`].
//!
//! Split out of the parent test module so both files stay under the
//! `file-modularization` cap; the shared feature-forwarding formula is a self-contained
//! concern shared by every Rust-emitting binding scaffolder (ruby, elixir, node, php, python).

use crate::codegen::cfg::{
    cfg_default_and_forwarding_lines, native_wrapper_default_features, native_wrapper_default_features_for_config,
};
use crate::core::config::{ComponentConfig, ComponentProvidesConfig, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, TypeDef};
use std::collections::{BTreeSet, HashSet};

fn set(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|n| (*n).to_string()).collect()
}

/// The formula every caller relies on: a `default = [...]` line naming every feature, then one
/// `<feature> = ["<core>/<feature>"]` forwarding row per feature, in the set's sorted order.
#[test]
fn builds_default_line_then_one_forwarding_row_per_feature() {
    let features = set(&["chunking-tokenizers", "ner"]);
    let excluded = HashSet::new();

    let lines = cfg_default_and_forwarding_lines(&features, "core-lib", &excluded);

    assert_eq!(
        lines,
        vec![
            r#"default = ["chunking-tokenizers", "ner"]"#.to_string(),
            r#"chunking-tokenizers = ["core-lib/chunking-tokenizers"]"#.to_string(),
            r#"ner = ["core-lib/ner"]"#.to_string(),
        ],
        "got: {lines:?}"
    );
}

/// A name in `excluded_default_features` still gets a forwarding row (so `cargo build
/// --features <name>` keeps working, matching `RubyConfig::excluded_default_features`'s
/// documented escape hatch) but is dropped from the `default = [...]` array.
#[test]
fn excluded_default_feature_is_forwarded_but_not_defaulted() {
    let features = set(&["chunking-tokenizers", "heic"]);
    let excluded: HashSet<&str> = ["heic"].into_iter().collect();

    let lines = cfg_default_and_forwarding_lines(&features, "core-lib", &excluded);

    assert_eq!(
        lines[0], r#"default = ["chunking-tokenizers"]"#,
        "excluded name must not appear in default: {lines:?}"
    );
    assert!(
        lines.contains(&r#"heic = ["core-lib/heic"]"#.to_string()),
        "excluded name must still get a forwarding row: {lines:?}"
    );
}

/// An empty feature set still produces a `default = []` line and no forwarding rows -- Elixir's
/// scaffold has always emitted its `[features]` table unconditionally, so this must not
/// special-case emptiness away; callers that want "no table at all" for an empty set (ruby,
/// node, python) check `features.is_empty()` themselves before calling.
#[test]
fn empty_feature_set_still_emits_a_default_line_with_no_forwarding_rows() {
    let features = BTreeSet::new();
    let excluded = HashSet::new();

    let lines = cfg_default_and_forwarding_lines(&features, "core-lib", &excluded);

    assert_eq!(lines, vec!["default = []".to_string()], "got: {lines:?}");
}

/// A configured downloadable component's Cargo features gate its *implementation* types, which
/// the host binding never wraps -- the implementation is compiled into the downloaded producer
/// cdylib. They must therefore never reach a binding's `[features]` table: a binding that
/// overrides the core crate (wasm/mobile, via `core_crate_override`) would forward
/// `<override>/<component-feature>` for a feature that crate never declares, and the whole
/// workspace stops resolving. Reproduces the spikard wasm binding, where `codegen-openapi` leaked
/// through this path.
#[test]
fn component_implementation_features_are_not_forwarded_to_bindings() {
    let api = ApiSurface {
        crate_name: "hostlib".to_string(),
        types: vec![TypeDef {
            name: "OpenApiCompiler".to_string(),
            rust_path: "hostlib::OpenApiCompiler".to_string(),
            cfg: Some(r#"feature = "codegen-openapi""#.to_string()),
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut config = ResolvedCrateConfig {
        name: "hostlib".to_string(),
        wrapper_default_features: vec!["di".to_string()],
        ..Default::default()
    };
    config.components.push(ComponentConfig {
        name: "openapi".to_string(),
        provides: vec![ComponentProvidesConfig {
            contract: "spec".to_string(),
            implementation: "hostlib::OpenApiCompiler".to_string(),
        }],
        features: vec!["codegen-openapi".to_string()],
        default_features: false,
        targets: Some(vec!["aarch64-apple-darwin".to_string()]),
        bundled_on: Vec::new(),
    });

    let unfiltered = native_wrapper_default_features(&api, &config.wrapper_default_features);
    assert!(
        unfiltered.contains("codegen-openapi"),
        "control: raw discovery must still see the component feature, got: {unfiltered:?}"
    );

    let forwarded = native_wrapper_default_features_for_config(&api, &config);
    assert!(
        !forwarded.contains("codegen-openapi"),
        "the component feature must be dropped from binding forwarding, got: {forwarded:?}"
    );
    assert!(
        forwarded.contains("di"),
        "an ordinary configured feature must still be forwarded, got: {forwarded:?}"
    );
}
