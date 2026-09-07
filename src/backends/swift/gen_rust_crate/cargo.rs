//! Emits `Cargo.toml` and `build.rs` for the swift-bridge crate.

use crate::codegen::cfg as shared_cfg;
use crate::core::ir::ApiSurface;

/// Formats a features array for TOML output.
/// Uses multi-line format when `features.len() >= 3` or the rendered line exceeds 100 chars.
fn format_features_array(features: &[String]) -> String {
    if features.is_empty() {
        return String::new();
    }

    let quoted = features.iter().map(|f| format!("\"{f}\"")).collect::<Vec<_>>();
    let single_line = quoted.join(", ");
    let single_line_full = format!(", features = [{single_line}]");

    if features.len() >= 3 || single_line_full.len() > 100 {
        let mut multi_line = String::from(", features = [\n");
        for feature in &quoted {
            multi_line.push_str("    ");
            multi_line.push_str(feature);
            multi_line.push_str(",\n");
        }
        multi_line.push(']');
        multi_line
    } else {
        single_line_full
    }
}

/// Emit the `Cargo.toml` content for the generated swift crate.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_cargo_toml(
    crate_name: &str,
    core_dep_key: &str,
    _core_crate_dir: &str,
    version: &str,
    swift_bridge_ver: &str,
    swift_bridge_build_ver: &str,
    core_path: &str,
    features: &[String],
    extra_deps: &str,
    license: &str,
    has_streaming_adapters: bool,
    target_overrides: &[crate::core::config::languages::SwiftTargetDepOverride],
    api: &ApiSurface,
    excluded_default_features: &[String],
    ffi_dep_key: &str,
    ffi_dep_path: &str,
    ffi_features: &[String],
    ffi_target_overrides: &[crate::core::config::languages::SwiftTargetDepOverride],
    cargo_lints: &crate::core::config::CargoLintsConfig,
) -> String {
    let source_crate_name = core_dep_key;
    let features_block = if features.is_empty() {
        String::new()
    } else {
        format_features_array(features)
    };
    let package_rename_block = if core_dep_key != crate_name {
        format!(", package = \"{crate_name}\"")
    } else {
        String::new()
    };
    let streaming_deps = if has_streaming_adapters {
        "futures-util = \"0.3\"\n"
    } else {
        ""
    };
    let extra_deps_block = if extra_deps.trim().is_empty() {
        String::new()
    } else {
        format!("{extra_deps}\n")
    };
    // gated on `cfg(not(any(<override cfgs>)))` and each override emits its own
    // `[target.'cfg(...)'.dependencies]` block (similar to the FFI and Dart
    let core_dep_for_block = crate::scaffold::render_core_dep(
        source_crate_name,
        core_path,
        &format!("{features_block}{package_rename_block}"),
        version,
    );
    // Both the core-dep and FFI-dep override loops below emit
    // `[target.'cfg(...)'.dependencies]` tables into the same `[dependencies]`
    // section of the final manifest, so cargo-sort's table-order rule
    // (alphabetical by the raw cfg predicate string, plain byte-wise
    // comparison) applies across both groups together — sorting each group
    // independently and concatenating them is not enough. Collect every
    // target-cfg entry (core + FFI) into one list and sort it once, in
    // `target_blocks_section` below.
    let mut target_dep_entries: Vec<(String, String)> = Vec::new();
    if !target_overrides.is_empty() {
        // Gate the default dep on cfg(not(any(<overrides>))) to keep one and only
        let neg_cfg = if target_overrides.len() == 1 {
            target_overrides[0].cfg.clone()
        } else {
            let any = target_overrides
                .iter()
                .map(|o| o.cfg.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            format!("any({any})")
        };
        target_dep_entries.push((format!("not({neg_cfg})"), core_dep_for_block.clone()));
        for entry in target_overrides {
            let feat_list = entry
                .features
                .iter()
                .map(|f| format!("\"{f}\""))
                .collect::<Vec<_>>()
                .join(", ");
            let feats_block = if feat_list.is_empty() {
                String::new()
            } else {
                format!(", features = [{feat_list}]")
            };
            let default_block = if entry.default_features {
                String::new()
            } else {
                ", default-features = false".to_string()
            };
            let entry_dep = crate::scaffold::render_core_dep(
                source_crate_name,
                core_path,
                &format!("{feats_block}{default_block}{package_rename_block}"),
                version,
            );
            target_dep_entries.push((entry.cfg.clone(), entry_dep));
        }
    }
    let mut dep_entries: Vec<String> = vec![
        "ahash = \"0.8\"".to_string(),
        "async-trait = \"0.1\"".to_string(),
        "libc = \"0.2\"".to_string(),
        "serde = { version = \"1\", features = [\"derive\"] }".to_string(),
        "serde_json = \"1\"".to_string(),
        format!("swift-bridge = \"{swift_bridge_ver}\""),
        "tokio = { version = \"1\", features = [\"rt\", \"rt-multi-thread\", \"macros\"] }".to_string(),
    ];
    if !core_dep_for_block.is_empty() && target_overrides.is_empty() {
        dep_entries.push(core_dep_for_block.clone());
    }
    // NOTE: see `ffi_keep_alive_shim.rs.jinja` and `ResolvedCrateConfig::ffi_crate_path_from_swift_rust`.
    // When `ffi_features` is non-empty, drop the FFI crate's default features and enable exactly
    // this set — lets the swift shim exclude cross-compile-hostile features (e.g. `heic` via a
    // `full-no-heic` set) that the primary core dep's feature handling does not reach.
    let ffi_suffix = if ffi_features.is_empty() {
        String::new()
    } else {
        format!(", default-features = false{}", format_features_array(ffi_features))
    };
    // When `ffi_target_overrides` is set the FFI dep moves out of the flat
    // `[dependencies]` table entirely and into `target_dep_entries` below —
    // mirrors how the core dep's `target_overrides` loop above excludes
    // `core_dep_for_block` from `dep_entries` once overrides apply.
    if ffi_target_overrides.is_empty() {
        dep_entries.push(crate::scaffold::render_core_dep(
            ffi_dep_key,
            ffi_dep_path,
            &ffi_suffix,
            version,
        ));
    }
    if !ffi_target_overrides.is_empty() {
        let neg_cfg = if ffi_target_overrides.len() == 1 {
            ffi_target_overrides[0].cfg.clone()
        } else {
            let any = ffi_target_overrides
                .iter()
                .map(|o| o.cfg.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            format!("any({any})")
        };
        let default_entry = crate::scaffold::render_core_dep(ffi_dep_key, ffi_dep_path, &ffi_suffix, version);
        target_dep_entries.push((format!("not({neg_cfg})"), default_entry));
        for entry in ffi_target_overrides {
            let feat_list = entry
                .features
                .iter()
                .map(|f| format!("\"{f}\""))
                .collect::<Vec<_>>()
                .join(", ");
            let feats_block = if feat_list.is_empty() {
                String::new()
            } else {
                format!(", features = [{feat_list}]")
            };
            // Order matches `ffi_suffix` above (`default-features` before
            // `features`), not the core-dep override loop's order, so a
            // single override entry reproduces the same dep-line shape as
            // the flat `ffi_features`-only case.
            let entry_suffix = if entry.default_features {
                feats_block
            } else {
                format!(", default-features = false{feats_block}")
            };
            let entry_dep = crate::scaffold::render_core_dep(ffi_dep_key, ffi_dep_path, &entry_suffix, version);
            target_dep_entries.push((entry.cfg.clone(), entry_dep));
        }
    }
    // The core-dep and FFI-dep loops above both contribute to
    // `target_dep_entries`; sort them together (not each group separately) so
    // the combined `[target.'cfg(...)'.dependencies]` table sequence matches
    // what `cargo-sort` expects regardless of which dependency a given block
    // is about.
    let target_blocks_section = crate::scaffold::join_sorted_target_dep_blocks(target_dep_entries);
    let target_blocks_section = if target_blocks_section.is_empty() {
        String::new()
    } else {
        format!("\n{target_blocks_section}")
    };
    if has_streaming_adapters {
        dep_entries.push("futures-util = \"0.3\"".to_string());
    }
    for line in extra_deps.lines() {
        let trimmed = line.trim_end();
        if !trimmed.is_empty() {
            dep_entries.push(trimmed.to_string());
        }
    }
    crate::scaffold::sort_dependency_lines(&mut dep_entries);
    let dep_block = dep_entries.join("\n");
    let _ = streaming_deps;
    let _ = extra_deps_block;

    // `#[cfg(feature = "X")]` arms emitted by the codegen produce
    let mut cfg_features = shared_cfg::collect_cfg_features(api);
    // A config-only `excluded_default_features` name (gates no `#[cfg(feature = ...)]`) must
    // still get a forwarding entry below -- alef-task #374, regression in
    // `cargo_excluded_features_tests.rs`. ~keep
    cfg_features.extend(excluded_default_features.iter().cloned());
    let features_table = if cfg_features.is_empty() {
        String::new()
    } else {
        // `[target.'cfg(...)'.dependencies]` block alone is insufficient
        let excluded: std::collections::HashSet<&str> = excluded_default_features.iter().map(String::as_str).collect();
        let mut lines: Vec<String> = Vec::with_capacity(cfg_features.len() + 1);
        let default_list: Vec<String> = cfg_features
            .iter()
            .filter(|name| !excluded.contains(name.as_str()))
            .map(|name| format!("\"{name}\""))
            .collect();
        lines.push(format!("default = [{}]", default_list.join(", ")));
        for name in &cfg_features {
            lines.push(format!(r#"{name} = ["{core_dep_key}/{name}"]"#));
        }
        format!("[features]\n{}\n\n", lines.join("\n"))
    };

    // The [lints.rust] block keeps cfg(frb_expand) in the allow-list (FRB-internal
    // cfg used during macro expansion). Cargo allows only one `[lints.rust]` table
    // per manifest, so a configured `[crates.cargo_lints.rust]` entry becomes an
    // extra sibling line under this same hand-written header rather than a second
    // one -- see `CargoLintsConfig::extra_rust_lines`. ~keep
    let mut lints_rust_lines =
        vec!["unexpected_cfgs = { level = \"warn\", check-cfg = ['cfg(frb_expand)'] }".to_string()];
    lints_rust_lines.extend(cargo_lints.extra_rust_lines(&["unexpected_cfgs"]));
    let clippy_block = cargo_lints.clippy_block();
    let clippy_section = if clippy_block.is_empty() {
        String::new()
    } else {
        format!("\n\n{clippy_block}")
    };
    let lints_block = format!("[lints.rust]\n{}{clippy_section}", lints_rust_lines.join("\n"));

    format!(
        r#"# Generated by alef. Do not edit by hand.
[package]
name = "{crate_name}-swift"
version = "{version}"
edition = "2024"
license = "{license}"

# `ahash`, `async-trait`, `libc`, `serde`, `serde_json`, and `tokio` are all
# conditionally referenced by alef-emitted code: `ahash` only when the
# umbrella crate exposes `AHashMap<Cow<str>, _>` parameters (the conditional
# `__*_ahash` shim rebuilds), `async-trait` and `tokio` only when the API
# surface includes async streaming adapters and runtime spawn, `libc` only
# when service API C callback functions are emitted, `serde` and
# `serde_json` only when JSON DTO conversions are emitted. They are listed
# unconditionally in `[dependencies]` so the manifest is stable across
# regens, and ignored here so cargo-machete does not flag downstream crates
# whose API surface does not trigger those paths as unused.
[package.metadata.cargo-machete]
ignored = ["ahash", "async-trait", "libc", "serde", "serde_json", "tokio"]

[lib]
crate-type = ["cdylib", "staticlib"]
# The `extern "Swift"` block emits linker references that are only resolvable
# when the crate is linked into a Swift target. `cargo test --workspace` on
# pure-Rust runners (e.g. windows-latest) would otherwise fail with
# undefined `__swift_bridge__$*$alef_visit_*` symbols.
test = false
doctest = false
bench = false

{features_table}[dependencies]
{dep_block}
{target_blocks_section}
[build-dependencies]
swift-bridge-build = "{swift_bridge_build_ver}"

{lints_block}
"#
    )
}

/// Emit the `build.rs` content for the generated swift crate.
pub(crate) fn emit_build_rs() -> String {
    format!(
        "{}\n{}",
        crate::core::hash::SELF_MARKING_HEADER_LINE,
        r#"use std::path::PathBuf;

fn main() {
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR unset"));
    let crate_name = std::env::var("CARGO_PKG_NAME").expect("CARGO_PKG_NAME unset");
    let bridges = vec!["src/lib.rs"];
    swift_bridge_build::parse_bridges(bridges).write_all_concatenated(out_dir, &crate_name);
    println!("cargo:rerun-if-changed=src/lib.rs");
}
"#
    )
}

#[cfg(test)]
mod tests {
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
}
