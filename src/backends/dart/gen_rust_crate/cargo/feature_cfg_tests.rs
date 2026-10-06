use super::*;
use crate::core::ir::{EnumDef, EnumVariant};

fn make_unit_variant(name: &str, cfg: Option<&str>) -> EnumVariant {
    EnumVariant {
        name: name.to_string(),
        cfg: cfg.map(str::to_string),
        ..Default::default()
    }
}

/// When the API has cfg-gated enum variants the emitted Cargo.toml must declare
/// a forwarding `[features]` block mapping each referenced feature to the core
/// dep. This is Option B — the binding crate re-exports the feature rather than
/// using a `[lints.rust]` check-cfg allow-list.
#[test]
fn cargo_toml_emits_forwarding_features_block_for_cfg_gated_variants() {
    use crate::core::config::ResolvedCrateConfig;
    use crate::core::ir::ApiSurface;

    let api = ApiSurface {
        enums: vec![EnumDef {
            name: "ImageOutputFormat".to_string(),
            variants: vec![
                make_unit_variant("Native", None),
                make_unit_variant("Heic", Some("feature = \"heic\"")),
                make_unit_variant("Svg", Some("feature = \"svg\"")),
            ],
            ..Default::default()
        }],
        ..Default::default()
    };
    let config = ResolvedCrateConfig {
        name: "sample-lib".to_string(),
        ..Default::default()
    };
    let file = emit_cargo_toml("packages/dart/rust", &api, &config, "sample_lib");
    assert!(
        file.content.contains(r#"heic = ["sample_lib/heic"]"#),
        "Cargo.toml must forward `heic` feature to core dep; got:\n{}",
        file.content
    );
    assert!(
        file.content.contains(r#"svg = ["sample_lib/svg"]"#),
        "Cargo.toml must forward `svg` feature to core dep; got:\n{}",
        file.content
    );
    assert!(
        file.content.contains("[features]"),
        "Cargo.toml must contain a [features] section; got:\n{}",
        file.content
    );
    // `#[cfg(feature = "X")]` arms compile without explicit activation.
    assert!(
        file.content.contains("default = ["),
        "Cargo.toml must contain a `default` feature list; got:\n{}",
        file.content
    );
    assert!(
        file.content.contains("\"heic\"") && file.content.contains("\"svg\""),
        "default feature list must include all cfg-forwarded features; got:\n{}",
        file.content
    );
    assert!(
        file.content.contains("'cfg(frb_expand)'"),
        "Cargo.toml must still include cfg(frb_expand); got:\n{}",
        file.content
    );
    assert!(
        !file.content.contains("values("),
        "Cargo.toml must not contain check-cfg values() — forwarding replaces allow-list; got:\n{}",
        file.content
    );
    toml::from_str::<toml::Value>(&file.content).expect("generated Cargo.toml must be valid TOML");
}

/// End-to-end regression for the two derivation sites that previously disagreed: the dart
/// crate's `lib.rs` (via `bridge_fn::emit_bridge_fn`, which renders `f.cfg` verbatim as
/// `#[cfg(...)]`) and its `Cargo.toml` `[features]` table (via `cargo::emit_cargo_toml`, which
/// declares whatever `shared_cfg::collect_cfg_features` returns). Runs the real
/// `gen_rust_crate::emit` entry point — not hand-built strings — on a surface with a plain
/// cfg-gated function (`native-http`, already covered before this fix) alongside a service
/// whose configurator is cfg-gated (`tower`, previously dropped because `collect_cfg_features`
/// never walked `ApiSurface::services`). ~keep
#[test]
fn emit_agrees_on_cfg_features_between_lib_rs_and_cargo_toml_including_services() {
    use crate::core::config::ResolvedCrateConfig;
    use crate::core::ir::{ApiSurface, FunctionDef, MethodDef, ServiceDef};

    let api = ApiSurface {
        functions: vec![FunctionDef {
            name: "native_ping".to_string(),
            rust_path: "sample_lib::native_ping".to_string(),
            cfg: Some(r#"feature = "native-http""#.to_string()),
            ..Default::default()
        }],
        services: vec![ServiceDef {
            name: "ClientConfig".to_string(),
            rust_path: "sample_lib::client::ClientConfig".to_string(),
            constructor: MethodDef {
                name: "new".to_string(),
                ..Default::default()
            },
            configurators: vec![MethodDef {
                name: "with_tower_layer".to_string(),
                cfg: Some(r#"feature = "tower""#.to_string()),
                ..Default::default()
            }],
            registrations: vec![],
            entrypoints: vec![],
            doc: String::new(),
            cfg: None,
        }],
        ..Default::default()
    };
    let config = ResolvedCrateConfig {
        name: "sample-lib".to_string(),
        ..Default::default()
    };

    let files = crate::backends::dart::gen_rust_crate::emit(&api, &config).expect("dart backend generates files");
    let lib_rs = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("src/lib.rs"))
        .expect("lib.rs is generated");
    let cargo_toml = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("Cargo.toml"))
        .expect("Cargo.toml is generated");

    // Control (deliverable: a negative assertion is worthless unless the fixture actually
    // emitted a cfg gate) — prove `native_ping` really is gated in the generated source before
    // trusting any conclusion drawn from the manifest.
    assert!(
        lib_rs.content.contains("#[cfg(feature = \"native-http\")]") && lib_rs.content.contains("fn native_ping"),
        "control: the fixture must actually emit a cfg-gated function in lib.rs, got:\n{}",
        lib_rs.content
    );

    let parsed: toml::Value = toml::from_str(&cargo_toml.content).expect("generated Cargo.toml must be valid TOML");
    let declared: Vec<&str> = parsed["features"]
        .as_table()
        .expect("[features] is a table")
        .keys()
        .map(String::as_str)
        .collect();

    assert!(
        declared.contains(&"native-http"),
        "control: the function-level gate must already forward, got: {declared:?}"
    );
    assert!(
        declared.contains(&"tower"),
        "the service configurator's cfg gate must also forward to [features], or cargo rejects \
             any `#[cfg(feature = \"tower\")]` a backend re-emits for it as unexpected_cfg; got: {declared:?}"
    );
}

/// The dart scaffold already emits its own `unexpected_cfgs` check-cfg
/// allowlist for `cfg(frb_expand)` into `[lints.rust]`. A configured
/// `[crates.cargo_lints]` table must compose with that single table -- not
/// open a second `[lints.rust]` header, which Cargo rejects as a duplicate
/// table -- and the builtin `cfg(frb_expand)` entry must survive a colliding
/// user key.
#[test]
fn cargo_toml_merges_configured_cargo_lints_with_builtin_unexpected_cfgs() {
    use crate::core::config::{CargoLintsConfig, ResolvedCrateConfig};
    use crate::core::ir::ApiSurface;

    let mut cargo_lints = CargoLintsConfig::default();
    cargo_lints
        .rust
        .insert("unexpected_cfgs".to_string(), toml::Value::String("warn".to_string()));
    cargo_lints
        .rust
        .insert("unused_must_use".to_string(), toml::Value::String("deny".to_string()));
    cargo_lints
        .clippy
        .insert("print_stdout".to_string(), toml::Value::String("deny".to_string()));
    let config = ResolvedCrateConfig {
        name: "sample-lib".to_string(),
        cargo_lints,
        ..Default::default()
    };
    let file = emit_cargo_toml("packages/dart/rust", &ApiSurface::default(), &config, "sample_lib");

    assert_eq!(
        file.content.matches("[lints.rust]").count(),
        1,
        "must not emit a second [lints.rust] table; got:\n{}",
        file.content
    );
    assert!(
        file.content
            .contains("unexpected_cfgs = { level = \"warn\", check-cfg = ['cfg(frb_expand)'] }"),
        "the builtin cfg(frb_expand) entry must survive the user's colliding key; got:\n{}",
        file.content
    );
    assert!(
        file.content.contains("unused_must_use = \"deny\""),
        "non-colliding configured rust lints must be spliced in; got:\n{}",
        file.content
    );
    assert!(
        file.content
            .contains("[lints.clippy]\ndbg_macro = \"deny\"\nprint_stderr = \"deny\"\nprint_stdout = \"deny\""),
        "configured clippy lints must merge with the builtin deny defaults; got:\n{}",
        file.content
    );
    toml::from_str::<toml::Value>(&file.content).expect("generated Cargo.toml must be valid TOML");
}

/// Absence of a configured `cargo_lints` table must reproduce the pre-existing
/// builtin `[lints.rust]` block exactly, followed by the builtin `[lints.clippy]`
/// deny block that alef now emits unconditionally for every generated binding crate.
#[test]
fn cargo_toml_emits_builtin_clippy_denies_when_cargo_lints_unset() {
    use crate::core::config::ResolvedCrateConfig;
    use crate::core::ir::ApiSurface;

    let config = ResolvedCrateConfig {
        name: "sample-lib".to_string(),
        ..Default::default()
    };
    let file = emit_cargo_toml("packages/dart/rust", &ApiSurface::default(), &config, "sample_lib");

    assert!(
        file.content.ends_with(
            "[lints.rust]\n# flutter_rust_bridge uses #[cfg(frb_expand)] internally during macro expansion.\n\
                 unexpected_cfgs = { level = \"warn\", check-cfg = ['cfg(frb_expand)'] }\n\n\
                 [lints.clippy]\ndbg_macro = \"deny\"\nprint_stderr = \"deny\"\nprint_stdout = \"deny\"\n"
        ),
        "got:\n{}",
        file.content
    );
}

/// When no item has a cfg attribute the `[features]` block must be omitted.
#[test]
fn cargo_toml_omits_features_block_when_no_cfg_attrs() {
    use crate::core::config::ResolvedCrateConfig;
    use crate::core::ir::ApiSurface;

    let api = ApiSurface {
        enums: vec![EnumDef {
            name: "SimpleEnum".to_string(),
            variants: vec![make_unit_variant("A", None), make_unit_variant("B", None)],
            ..Default::default()
        }],
        ..Default::default()
    };
    let config = ResolvedCrateConfig {
        name: "sample-lib".to_string(),
        ..Default::default()
    };
    let file = emit_cargo_toml("packages/dart/rust", &api, &config, "sample_lib");
    assert!(
        file.content
            .contains("unexpected_cfgs = { level = \"warn\", check-cfg = ['cfg(frb_expand)'] }"),
        "Cargo.toml must use single-entry form when no cfg attrs; got:\n{}",
        file.content
    );
    assert!(
        !file.content.contains("[features]"),
        "Cargo.toml must not contain [features] block when no cfg attrs; got:\n{}",
        file.content
    );
    toml::from_str::<toml::Value>(&file.content).expect("generated Cargo.toml must be valid TOML");
}

/// Features listed under `excluded_default_features` must still be declared
/// as opt-in forwarding entries, but must NOT appear in the `default = [...]`
/// array. This keeps `cargo build --features <name>` working on desktop
/// while preventing default builds (e.g. iOS / Android NDK cross-compiles)
/// from auto-activating features that pull in system libraries with
/// cross-compile-hostile `build.rs` scripts (e.g. `libheif-sys` via `heic`).
#[test]
fn cargo_toml_excludes_named_features_from_default_but_keeps_forwarding_entries() {
    use crate::core::config::ResolvedCrateConfig;
    use crate::core::config::languages::DartConfig;
    use crate::core::ir::ApiSurface;

    let api = ApiSurface {
        enums: vec![EnumDef {
            name: "ImageOutputFormat".to_string(),
            variants: vec![
                make_unit_variant("Native", None),
                make_unit_variant("Heic", Some("feature = \"heic\"")),
                make_unit_variant("Svg", Some("feature = \"svg\"")),
            ],
            ..Default::default()
        }],
        ..Default::default()
    };
    let config = ResolvedCrateConfig {
        name: "sample-lib".to_string(),
        dart: Some(DartConfig {
            excluded_default_features: vec!["heic".to_string()],
            ..Default::default()
        }),
        ..Default::default()
    };
    let file = emit_cargo_toml("packages/dart/rust", &api, &config, "sample_lib");

    assert!(
        file.content.contains(r#"heic = ["sample_lib/heic"]"#),
        "Cargo.toml must keep `heic` forwarding entry; got:\n{}",
        file.content
    );
    assert!(
        file.content.contains(r#"svg = ["sample_lib/svg"]"#),
        "Cargo.toml must keep `svg` forwarding entry; got:\n{}",
        file.content
    );
    let default_line = file
        .content
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
    toml::from_str::<toml::Value>(&file.content).expect("generated Cargo.toml must be valid TOML");
}

/// When the core crate defines an `android-target` aggregate, the dart bridge
/// crate's `[features]` block must emit a matching `android-target` that
/// forwards to the core dep and enables the cfg-forwarded passthrough features
/// that are members of the core aggregate (sorted, `full` and non-members
/// excluded). The forward uses the dart `core_dep_key` (rust-ident form).
#[test]
fn cargo_toml_emits_android_target_aggregate_when_core_defines_it() {
    use crate::core::config::ResolvedCrateConfig;
    use crate::core::ir::ApiSurface;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::TempDir;

    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::write(
        root.join("Cargo.toml"),
        "[workspace]\nresolver = \"2\"\nmembers = [\"crates/sample-core\"]\n",
    )
    .unwrap();
    fs::create_dir_all(root.join("crates/sample-core/src")).unwrap();
    fs::write(root.join("crates/sample-core/src/lib.rs"), "pub fn f() {}").unwrap();
    fs::write(
        root.join("crates/sample-core/Cargo.toml"),
        r#"[package]
name = "sample-core"
version = "0.1.0"

[features]
android-target = ["no-ort-target", "ocr"]
no-ort-target = ["pdf", "html"]
pdf = []
html = []
ocr = []
embeddings = []
"#,
    )
    .unwrap();

    let api = ApiSurface {
        enums: vec![EnumDef {
            name: "Format".to_string(),
            variants: vec![
                make_unit_variant("Pdf", Some("feature = \"pdf\"")),
                make_unit_variant("Html", Some("feature = \"html\"")),
                make_unit_variant("Ocr", Some("feature = \"ocr\"")),
                make_unit_variant("Embeddings", Some("feature = \"embeddings\"")),
            ],
            ..Default::default()
        }],
        ..Default::default()
    };
    let config = ResolvedCrateConfig {
        name: "sample-core".to_string(),
        workspace_root: Some(root.to_path_buf()),
        sources: vec![PathBuf::from("crates/sample-core/src/lib.rs")],
        ..Default::default()
    };
    let file = emit_cargo_toml("packages/dart/rust", &api, &config, "sample_core");
    assert!(
        file.content
            .contains(r#"android-target = ["sample_core/android-target", "html", "ocr", "pdf"]"#),
        "dart Cargo.toml must emit the android-target aggregate feature; got:\n{}",
        file.content
    );
    toml::from_str::<toml::Value>(&file.content).expect("generated Cargo.toml must be valid TOML");
}

/// cfg-gated types (not just variants) must also appear in the forwarding block.
#[test]
fn cargo_toml_forwarding_covers_type_level_cfg_attrs() {
    use crate::core::config::ResolvedCrateConfig;
    use crate::core::ir::{ApiSurface, TypeDef};

    let api = ApiSurface {
        types: vec![TypeDef {
            name: "PdfDoc".to_string(),
            rust_path: "mylib::PdfDoc".to_string(),
            cfg: Some(r#"feature = "pdf""#.to_string()),
            ..Default::default()
        }],
        ..Default::default()
    };
    let config = ResolvedCrateConfig {
        name: "sample-lib".to_string(),
        ..Default::default()
    };
    let file = emit_cargo_toml("packages/dart/rust", &api, &config, "sample_lib");
    assert!(
        file.content.contains(r#"pdf = ["sample_lib/pdf"]"#),
        "Cargo.toml must forward `pdf` feature from type-level cfg; got:\n{}",
        file.content
    );
    toml::from_str::<toml::Value>(&file.content).expect("generated Cargo.toml must be valid TOML");
}

/// Regression test: `cargo-sort` (and hence `poly lint`) orders
/// `[target.'cfg(...)'.dependencies]` tables alphabetically by the raw cfg
/// predicate string (plain byte-wise comparison), NOT with the default
/// `cfg(not(any(...)))` branch always first. With multiple overrides, an
/// `all(...)`-prefixed override (a downstream consumer's macOS-Intel target)
/// must sort *before* the `not(any(...))` default branch (`'a'` < `'n'`), while a
/// `target_os = ...` override sorts after it (`'n'` < `'t'`).
#[test]
fn cargo_toml_target_dep_overrides_sort_all_before_not() {
    use crate::core::config::ResolvedCrateConfig;
    use crate::core::config::languages::{DartConfig, DartTargetDepOverride};
    use crate::core::ir::ApiSurface;

    let api = ApiSurface::default();
    let config = ResolvedCrateConfig {
        name: "sample-lib".to_string(),
        dart: Some(DartConfig {
            target_dep_overrides: vec![
                DartTargetDepOverride {
                    cfg: "target_os = \"android\"".to_string(),
                    features: vec!["android-target".to_string()],
                    default_features: false,
                },
                DartTargetDepOverride {
                    cfg: "target_os = \"windows\"".to_string(),
                    features: vec!["windows-target".to_string()],
                    default_features: false,
                },
                DartTargetDepOverride {
                    cfg: "all(target_os = \"macos\", target_arch = \"x86_64\")".to_string(),
                    features: vec!["macos-intel-target".to_string()],
                    default_features: false,
                },
            ],
            ..Default::default()
        }),
        ..Default::default()
    };
    let file = emit_cargo_toml("packages/dart/rust", &api, &config, "sample_lib");
    let content = &file.content;

    let all_pos = content
        .find("[target.'cfg(all(target_os = \"macos\", target_arch = \"x86_64\"))'.dependencies]")
        .expect("expected the macOS-Intel `all(...)` override block");
    let not_pos = content
        .find("[target.'cfg(not(any(")
        .expect("expected the default `not(any(...))` block");
    let android_pos = content
        .find("[target.'cfg(target_os = \"android\")'.dependencies]")
        .expect("expected the android override block");

    assert!(
        all_pos < not_pos,
        "the `all(...)` override must sort BEFORE the `not(...)` default branch; got:\n{content}"
    );
    assert!(
        not_pos < android_pos,
        "the `not(...)` default branch must sort before `target_os = \"android\"`; got:\n{content}"
    );
    toml::from_str::<toml::Value>(content).expect("generated Cargo.toml must be valid TOML");
}

/// A consumer whose root `Cargo.toml` declares a `[workspace]` but no
/// `[workspace.package] version` (e.g. a plain `resolver`+`members` table) must
/// still get a literal `version = "…"` line — there is nothing to inherit from.
#[test]
fn cargo_toml_uses_literal_version_without_workspace_package_version() {
    use crate::core::config::ResolvedCrateConfig;
    use crate::core::ir::ApiSurface;
    use std::fs;
    use tempfile::TempDir;

    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    let root_cargo_toml = root.join("Cargo.toml");
    fs::write(
        &root_cargo_toml,
        "[workspace]\nresolver = \"2\"\nmembers = [\"crates/sample-core\"]\n",
    )
    .unwrap();

    let api = ApiSurface::default();
    let config = ResolvedCrateConfig {
        name: "sample-core".to_string(),
        workspace_root: Some(root.to_path_buf()),
        version_from: root_cargo_toml.to_string_lossy().into_owned(),
        ..Default::default()
    };
    let file = emit_cargo_toml("packages/dart/rust", &api, &config, "sample_core");
    assert!(
        file.content.contains("version = \"0.1.0\""),
        "must fall back to a literal version when [workspace.package] is absent; got:\n{}",
        file.content
    );
    assert!(
        !file.content.contains("version.workspace = true"),
        "must not claim workspace inheritance that doesn't exist; got:\n{}",
        file.content
    );
    toml::from_str::<toml::Value>(&file.content).expect("generated Cargo.toml must be valid TOML");
}

/// A consumer whose root `Cargo.toml` declares `[workspace.package] version` must
/// get `version.workspace = true` in the emitted dart rust crate's Cargo.toml, so
/// the crate tracks the workspace-wide version bump instead of drifting behind a
/// stamped-once literal.
#[test]
fn cargo_toml_inherits_workspace_version_when_workspace_package_declares_it() {
    use crate::core::config::ResolvedCrateConfig;
    use crate::core::ir::ApiSurface;
    use std::fs;
    use tempfile::TempDir;

    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    let root_cargo_toml = root.join("Cargo.toml");
    fs::write(
            &root_cargo_toml,
            "[workspace]\nresolver = \"2\"\nmembers = [\"crates/sample-core\"]\n\n[workspace.package]\nversion = \"9.9.9\"\n",
        )
        .unwrap();

    let api = ApiSurface::default();
    let config = ResolvedCrateConfig {
        name: "sample-core".to_string(),
        workspace_root: Some(root.to_path_buf()),
        version_from: root_cargo_toml.to_string_lossy().into_owned(),
        ..Default::default()
    };
    let file = emit_cargo_toml("packages/dart/rust", &api, &config, "sample_core");
    assert!(
        file.content.contains("version.workspace = true"),
        "must inherit the workspace version when [workspace.package] declares one; got:\n{}",
        file.content
    );
    assert!(
        !file.content.contains("version = \"9.9.9\""),
        "must not also emit a literal version line; got:\n{}",
        file.content
    );
    toml::from_str::<toml::Value>(&file.content).expect("generated Cargo.toml must be valid TOML");
}
