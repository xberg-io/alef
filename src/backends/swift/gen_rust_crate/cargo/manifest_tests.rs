use super::*;
use crate::core::ir::{ApiSurface, EnumDef, EnumVariant};

fn make_unit_variant(name: &str, cfg: Option<&str>) -> EnumVariant {
    EnumVariant {
        serde_untagged: false,
        name: name.to_string(),
        fields: vec![],
        doc: String::new(),
        is_default: false,
        serde_rename: None,
        is_tuple: false,
        sensitive: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        originally_had_data_fields: false,
        cfg: cfg.map(|s| s.to_string()),
        version: Default::default(),
    }
}

/// When the API has cfg-gated enum variants the emitted Cargo.toml must declare
/// a forwarding `[features]` block mapping each referenced feature to the core dep.
#[test]
fn cargo_toml_emits_forwarding_features_block_for_cfg_gated_variants() {
    let api = ApiSurface {
        enums: vec![EnumDef {
            name: "ImageOutputFormat".to_string(),
            variants: vec![
                make_unit_variant("Heif", Some("feature = \"heic\"")),
                make_unit_variant("Svg", Some("feature = \"svg\"")),
                make_unit_variant("Jpeg", None),
            ],
            methods: vec![],
            excluded_variants: vec![],
            ..Default::default()
        }],
        ..Default::default()
    };

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
        &Default::default(),
    );

    assert!(
        content.contains(r#"heic = ["sample_lib/heic"]"#),
        "Cargo.toml must forward `heic` feature to core dep; got:\n{}",
        content
    );
    assert!(
        content.contains(r#"svg = ["sample_lib/svg"]"#),
        "Cargo.toml must forward `svg` feature to core dep; got:\n{}",
        content
    );
    assert!(
        content.contains("[features]"),
        "Cargo.toml must contain a [features] section; got:\n{}",
        content
    );
    assert!(
        content.contains("'cfg(frb_expand)'"),
        "Cargo.toml must still include cfg(frb_expand); got:\n{}",
        content
    );
    assert!(
        !content.contains("values("),
        "Cargo.toml must not contain check-cfg values() — forwarding replaces allow-list; got:\n{}",
        content
    );
    toml::from_str::<toml::Value>(&content).expect("generated Cargo.toml must be valid TOML");
}

/// The swift scaffold already emits its own `unexpected_cfgs` check-cfg
/// allowlist for `cfg(frb_expand)` into `[lints.rust]`. A configured
/// `cargo_lints` table must compose with that single table -- not open a
/// second `[lints.rust]` header, which Cargo rejects as a duplicate table --
/// and the builtin `cfg(frb_expand)` entry must survive a colliding user key.
#[test]
fn cargo_toml_merges_configured_cargo_lints_with_builtin_unexpected_cfgs() {
    let mut cargo_lints = crate::core::config::CargoLintsConfig::default();
    cargo_lints
        .rust
        .insert("unexpected_cfgs".to_string(), toml::Value::String("warn".to_string()));
    cargo_lints
        .rust
        .insert("unused_must_use".to_string(), toml::Value::String("deny".to_string()));
    cargo_lints
        .clippy
        .insert("print_stdout".to_string(), toml::Value::String("deny".to_string()));

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
        &ApiSurface::default(),
        &[],
        "sample-lib-ffi",
        "../../../crates/sample-lib-ffi",
        &[],
        &[],
        &cargo_lints,
    );

    assert_eq!(
        content.matches("[lints.rust]").count(),
        1,
        "must not emit a second [lints.rust] table; got:\n{content}"
    );
    assert!(
        content.contains("unexpected_cfgs = { level = \"warn\", check-cfg = ['cfg(frb_expand)'] }"),
        "the builtin cfg(frb_expand) entry must survive the user's colliding key; got:\n{content}"
    );
    assert!(
        content.contains("unused_must_use = \"deny\""),
        "non-colliding configured rust lints must be spliced in; got:\n{content}"
    );
    assert!(
        content.contains("[lints.clippy]\ndbg_macro = \"deny\"\nprint_stderr = \"deny\"\nprint_stdout = \"deny\""),
        "configured clippy lints must merge with the builtin deny defaults; got:\n{content}"
    );
    toml::from_str::<toml::Value>(&content).expect("generated Cargo.toml must be valid TOML");
}

/// Absence of a configured `cargo_lints` table must reproduce the pre-existing
/// builtin `[lints.rust]` block exactly, followed by the builtin `[lints.clippy]`
/// deny block that alef now emits unconditionally for every generated binding crate.
#[test]
fn cargo_toml_emits_builtin_clippy_denies_when_cargo_lints_unset() {
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
        &ApiSurface::default(),
        &[],
        "sample-lib-ffi",
        "../../../crates/sample-lib-ffi",
        &[],
        &[],
        &Default::default(),
    );

    assert!(
        content.ends_with(
            "[lints.rust]\nunexpected_cfgs = { level = \"warn\", check-cfg = ['cfg(frb_expand)'] }\n\n\
                 [lints.clippy]\ndbg_macro = \"deny\"\nprint_stderr = \"deny\"\nprint_stdout = \"deny\"\n"
        ),
        "got:\n{content}"
    );
}

/// When no item has a cfg attribute the `[features]` block must be omitted.
#[test]
fn cargo_toml_omits_features_block_when_no_cfg_attrs() {
    let api = ApiSurface {
        enums: vec![EnumDef {
            name: "SimpleEnum".to_string(),
            variants: vec![make_unit_variant("A", None), make_unit_variant("B", None)],
            methods: vec![],
            excluded_variants: vec![],
            ..Default::default()
        }],
        ..Default::default()
    };

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
        &Default::default(),
    );

    assert!(
        content.contains("'cfg(frb_expand)'"),
        "Cargo.toml must include cfg(frb_expand); got:\n{}",
        content
    );
    assert!(
        !content.contains("[features]"),
        "Cargo.toml must not contain [features] block when no cfg attrs; got:\n{}",
        content
    );
    assert!(
        !content.contains("values("),
        "Cargo.toml must not contain feature values when no cfg attrs; got:\n{}",
        content
    );
    toml::from_str::<toml::Value>(&content).expect("generated Cargo.toml must be valid TOML");
}

/// cfg-gated types (not just variants) must also appear in the forwarding block.
#[test]
fn cargo_toml_forwarding_covers_type_level_cfg_attrs() {
    use crate::core::ir::TypeDef;

    let api = ApiSurface {
        types: vec![TypeDef {
            name: "PdfDoc".to_string(),
            rust_path: "mylib::PdfDoc".to_string(),
            cfg: Some(r#"feature = "pdf""#.to_string()),
            ..Default::default()
        }],
        ..Default::default()
    };

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
        &Default::default(),
    );

    assert!(
        content.contains(r#"pdf = ["sample_lib/pdf"]"#),
        "Cargo.toml must forward `pdf` feature from type-level cfg; got:\n{}",
        content
    );
    toml::from_str::<toml::Value>(&content).expect("generated Cargo.toml must be valid TOML");
}

/// Features listed under `excluded_default_features` must still be declared
/// as opt-in forwarding entries, but must NOT appear in the `default = [...]`
/// array. This keeps `cargo build --features <name>` working on desktop
/// while preventing default builds (e.g. iOS / Android NDK cross-compiles)
/// from auto-activating features that pull in system libraries with
/// cross-compile-hostile `build.rs` scripts (e.g. `libheif-sys` via `heic`).
#[test]
fn cargo_toml_excludes_named_features_from_default_but_keeps_forwarding_entries() {
    let api = ApiSurface {
        enums: vec![EnumDef {
            name: "ImageOutputFormat".to_string(),
            variants: vec![
                make_unit_variant("Heif", Some("feature = \"heic\"")),
                make_unit_variant("Svg", Some("feature = \"svg\"")),
            ],
            methods: vec![],
            excluded_variants: vec![],
            ..Default::default()
        }],
        ..Default::default()
    };

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
        &Default::default(),
    );

    assert!(
        content.contains(r#"heic = ["sample_lib/heic"]"#),
        "Cargo.toml must keep `heic` forwarding entry; got:\n{}",
        content
    );
    assert!(
        content.contains(r#"svg = ["sample_lib/svg"]"#),
        "Cargo.toml must keep `svg` forwarding entry; got:\n{}",
        content
    );
    let default_line = content
        .lines()
        .find(|l| l.starts_with("default = ["))
        .expect("default = [...] line must be emitted");
    assert!(
        !default_line.contains("\"heic\""),
        "default = [...] must NOT contain excluded `heic`; got: {default_line}"
    );
    assert!(
        default_line.contains("\"svg\""),
        "default = [...] must still contain non-excluded `svg`; got: {default_line}"
    );
    toml::from_str::<toml::Value>(&content).expect("generated Cargo.toml must be valid TOML");
}

/// The generated swift crate must depend on the FFI crate directly (in
/// addition to the core crate). Regression test: without this dependency,
/// nothing in the swift crate's Rust dependency graph reaches the FFI
/// crate's `#[no_mangle] extern "C"` exports, so a Rust `staticlib` build
/// drops them entirely, leaving the shipped `.a` without the FFI symbols
/// the generated Swift service API code calls via `@_silgen_name`.
#[test]
fn cargo_toml_depends_on_ffi_crate() {
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
        &Default::default(),
    );

    assert!(
        content.contains(r#"sample-lib-ffi = { version = "0.1.0", path = "../../../crates/sample-lib-ffi" }"#),
        "Cargo.toml must depend on the FFI crate by path; got:\n{}",
        content
    );
    toml::from_str::<toml::Value>(&content).expect("generated Cargo.toml must be valid TOML");
}

/// When `ffi_features` is non-empty, the injected FFI crate dependency must
/// drop default features and enable exactly the listed set. This lets the
/// swift shim exclude cross-compile-hostile features (e.g. `heic` via a
/// `full-no-heic` set) on the secondary FFI dependency, which the primary
/// core dep's `features` / `excluded_default_features` do not reach.
#[test]
fn cargo_toml_ffi_crate_honors_ffi_features() {
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
        &["full-no-heic".to_string(), "pdf".to_string(), "ocr".to_string()],
        &[],
        &Default::default(),
    );

    let ffi_line = content
        .lines()
        .find(|l| l.starts_with("sample-lib-ffi = "))
        .expect("FFI dependency line must be emitted");
    assert!(
        ffi_line.contains("default-features = false"),
        "FFI dep must disable default features when ffi_features is set; got: {ffi_line}"
    );
    for feat in ["full-no-heic", "pdf", "ocr"] {
        assert!(
            content.contains(&format!("\"{feat}\"")),
            "FFI dep features must include `{feat}`; got:\n{content}"
        );
    }
    toml::from_str::<toml::Value>(&content).expect("generated Cargo.toml must be valid TOML");
}

/// Regression test for issue #370: without `ffi_target_dep_overrides`, the
/// injected FFI dep has no way to split per target the way the core dep
/// can via `target_dep_overrides`. This reproduces a downstream consumer's
/// exact hand-patched `packages/swift/rust/Cargo.toml` split — a flat
/// `full-no-heic` default gated off iOS/Android, and `android-target` on
/// iOS/Android — so that downstream patch can be deleted.
#[test]
fn cargo_toml_ffi_target_overrides_reproduce_downstream_ios_android_split() {
    use crate::core::config::languages::SwiftTargetDepOverride;

    let api = ApiSurface::default();
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
        &[],
        &api,
        &[],
        "acme-ffi",
        "../../../crates/acme-ffi",
        &["full-no-heic".to_string()],
        &ffi_overrides,
        &Default::default(),
    );

    assert!(
            content.contains(
                "[target.'cfg(not(any(target_os = \"ios\", target_os = \"android\")))'.dependencies]\n\
acme-ffi = { version = \"1.1.0\", path = \"../../../crates/acme-ffi\", default-features = false, features = [\"full-no-heic\"] }"
            ),
            "must emit the default (non-iOS/Android) target block exactly; got:\n{content}"
        );
    assert!(
            content.contains(
                "[target.'cfg(any(target_os = \"ios\", target_os = \"android\"))'.dependencies]\n\
acme-ffi = { version = \"1.1.0\", path = \"../../../crates/acme-ffi\", default-features = false, features = [\"android-target\"] }"
            ),
            "must emit the iOS/Android target block exactly; got:\n{content}"
        );
    // Both target-gated dep lines start with "acme-ffi = ", so distinguish
    // "no flat-table duplicate" by an exact count rather than a substring
    // match, which would also match the (expected) lines inside the two
    // target blocks asserted above. ~keep
    let ffi_dep_line_count = content.lines().filter(|l| l.starts_with("acme-ffi = ")).count();
    assert_eq!(
        ffi_dep_line_count, 2,
        "exactly the two target-gated acme-ffi lines must be emitted, with no bare \
             flat-table duplicate; got {ffi_dep_line_count} in:\n{content}"
    );
    assert!(
        !content.contains(r#"acme-ffi = { version = "1.1.0", path = "../../../crates/acme-ffi" }"#),
        "the flat `[dependencies]` table must not carry a bare, feature-less acme-ffi entry \
             once target overrides apply; got:\n{content}"
    );
    toml::from_str::<toml::Value>(&content).expect("generated Cargo.toml must be valid TOML");
}

/// Backward compatibility: a config that sets only the flat `ffi_features`
/// and no `ffi_target_dep_overrides` must emit exactly what it emits
/// today — a single ungated `[dependencies]` line, no
/// `[target.'cfg(...)'.dependencies]` blocks for the FFI dep. Existing
/// users must not have to migrate.
#[test]
fn cargo_toml_ffi_target_overrides_empty_is_unchanged_from_flat_ffi_features() {
    let api = ApiSurface::default();

    let with_empty_overrides = emit_cargo_toml(
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
        &["full-no-heic".to_string()],
        &[],
        &Default::default(),
    );

    assert!(
            with_empty_overrides.contains(
                r#"sample-lib-ffi = { version = "0.1.0", path = "../../../crates/sample-lib-ffi", default-features = false, features = ["full-no-heic"] }"#
            ),
            "no-override case must keep the flat single-line FFI dep; got:\n{with_empty_overrides}"
        );
    assert!(
        !with_empty_overrides.contains("[target.'cfg("),
        "no-override case must not emit any target-gated blocks; got:\n{with_empty_overrides}"
    );
    toml::from_str::<toml::Value>(&with_empty_overrides).expect("generated Cargo.toml must be valid TOML");
}

/// Regression test: `cargo-sort` (and hence `poly lint`) orders every
/// `[target.'cfg(...)'.dependencies]` table in the manifest alphabetically
/// by the raw cfg predicate string — across ALL dependencies sharing the
/// `[dependencies]` section, not per-dependency. The core dep's
/// `target_overrides` and the FFI dep's `ffi_target_overrides` both emit
/// such tables into the same manifest, so sorting each group independently
/// (and simply concatenating them) is not enough: this reproduces a
/// downstream consumer's real config (a core `all(...)` macOS-Intel
/// override plus an FFI `any(ios, android)` override) and asserts the
/// fully merged, globally sorted order.
#[test]
fn cargo_toml_merges_and_sorts_core_and_ffi_target_blocks_together() {
    use crate::core::config::languages::SwiftTargetDepOverride;

    let api = ApiSurface::default();
    let core_overrides = vec![
        SwiftTargetDepOverride {
            cfg: "target_os = \"android\"".to_string(),
            features: vec!["android-target".to_string()],
            default_features: false,
        },
        SwiftTargetDepOverride {
            cfg: "target_os = \"windows\"".to_string(),
            features: vec!["windows-target".to_string()],
            default_features: false,
        },
        SwiftTargetDepOverride {
            cfg: "all(target_os = \"macos\", target_arch = \"x86_64\")".to_string(),
            features: vec!["macos-intel-target".to_string()],
            default_features: false,
        },
    ];
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
        &Default::default(),
    );

    // Expected global order (plain byte-wise comparison of the raw cfg
    // predicate string): `all(` < `any(` < `not(` < `target_os`. ~keep
    let all_pos = content
        .find("[target.'cfg(all(target_os = \"macos\", target_arch = \"x86_64\"))'.dependencies]")
        .expect("expected the macOS-Intel `all(...)` override block");
    let any_pos = content
        .find("[target.'cfg(any(target_os = \"ios\", target_os = \"android\"))'.dependencies]")
        .expect("expected the FFI `any(ios, android)` override block");
    let not_ffi_pos = content
        .find("[target.'cfg(not(any(target_os = \"ios\", target_os = \"android\")))'.dependencies]")
        .expect("expected the FFI default `not(any(ios, android))` block");
    let not_core_pos = content
        .find("[target.'cfg(not(any(target_os = \"android\", target_os = \"windows\", all(")
        .expect("expected the core default `not(any(...))` block");
    let android_pos = content
        .find("[target.'cfg(target_os = \"android\")'.dependencies]")
        .expect("expected the core android override block");

    assert!(
        all_pos < any_pos,
        "`all(...)` must sort before `any(...)`; got:\n{content}"
    );
    assert!(
        any_pos < not_core_pos,
        "`any(...)` must sort before `not(any(android, ...))`; got:\n{content}"
    );
    assert!(
        not_core_pos < not_ffi_pos,
        "the core `not(any(android, windows, all(...)))` must sort before the FFI \
             `not(any(ios, android))` (byte-wise: \"android\" < \"ios\" right after the \
             shared `not(any(target_os = \"` prefix); got:\n{content}"
    );
    assert!(
        not_ffi_pos < android_pos,
        "`not(...)` must sort before `target_os = \"android\"`; got:\n{content}"
    );

    // `[build-dependencies]` must precede `[lints.rust]`.
    let build_deps_pos = content
        .find("[build-dependencies]")
        .expect("expected a [build-dependencies] section");
    let lints_pos = content.find("[lints.rust]").expect("expected a [lints.rust] section");
    assert!(
        build_deps_pos < lints_pos,
        "[build-dependencies] must precede [lints.rust]; got:\n{content}"
    );

    toml::from_str::<toml::Value>(&content).expect("generated Cargo.toml must be valid TOML");
    assert!(
        crate::test_support::cargo_sort_order::assert_dependency_keys_sorted("swift Cargo.toml", &content) > 0,
        "the swift manifest must carry dependency keys for the key-order check to examine"
    );
}

fn manifest_for(api: &ApiSurface) -> String {
    manifest_with_streaming(api, false)
}

fn manifest_with_streaming(api: &ApiSurface, has_streaming_adapters: bool) -> String {
    emit_cargo_toml(
        "sample-lib",
        "sample_lib",
        "sample-lib",
        "0.1.0",
        "0.1.59",
        "0.1.59",
        "../..",
        &[],
        "",
        "MIT",
        has_streaming_adapters,
        &[],
        api,
        &[],
        "sample-lib-ffi",
        "../../../crates/sample-lib-ffi",
        &[],
        &[],
        &Default::default(),
    )
}

/// An `async fn` in an `extern "Rust"` block only compiles when swift-bridge's `async`
/// feature (its runtime and `async_support` module) is on; a crate without async surface
/// keeps the plain dependency line.
#[test]
fn swift_bridge_async_feature_follows_the_async_surface() {
    use crate::core::ir::{FunctionDef, MethodDef, TypeDef};

    let sync_only = manifest_for(&ApiSurface::default());
    assert!(
        sync_only.contains("swift-bridge = \"0.1.59\"\n"),
        "a sync-only API must keep the plain swift-bridge dependency, got:\n{sync_only}"
    );

    let async_function = ApiSurface {
        functions: vec![FunctionDef {
            name: "fetch".to_string(),
            is_async: true,
            ..Default::default()
        }],
        ..Default::default()
    };
    let async_method = ApiSurface {
        types: vec![TypeDef {
            name: "Client".to_string(),
            is_opaque: true,
            methods: vec![MethodDef {
                name: "fetch".to_string(),
                is_async: true,
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    };
    let async_trait_method = ApiSurface {
        types: vec![TypeDef {
            name: "Plugin".to_string(),
            is_trait: true,
            methods: vec![MethodDef {
                name: "process".to_string(),
                is_async: true,
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    };
    let trait_manifest = manifest_for(&async_trait_method);
    assert!(
        trait_manifest.contains("swift-bridge = \"0.1.59\"\n"),
        "an async trait-bridge method is not declared `async fn`, so it needs no feature, got:\n{trait_manifest}"
    );
    for (label, api) in [("function", async_function), ("method", async_method)] {
        let content = manifest_for(&api);
        assert!(
            content.contains("swift-bridge = { version = \"0.1.59\", features = [\"async\"] }"),
            "an async {label} must enable swift-bridge's `async` feature, got:\n{content}"
        );
        toml::from_str::<toml::Value>(&content).expect("generated Cargo.toml must be valid TOML");
    }
}

/// The stream-start and `next` bridge functions are `async fn`, so a streaming adapter needs
/// the feature even when no IR function or method is itself async. ~keep
#[test]
fn swift_bridge_async_feature_is_enabled_for_a_streaming_adapter() {
    let content = manifest_with_streaming(&ApiSurface::default(), true);
    assert!(
        content.contains("swift-bridge = { version = \"0.1.59\", features = [\"async\"] }"),
        "a streaming adapter must enable swift-bridge's `async` feature, got:\n{content}"
    );
    toml::from_str::<toml::Value>(&content).expect("generated Cargo.toml must be valid TOML");
}
