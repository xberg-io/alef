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
    let target_override_blocks = if target_overrides.is_empty() {
        String::new()
    } else {
        let mut blocks = String::new();
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
        blocks.push_str(&format!(
            "\n[target.'cfg(not({neg_cfg}))'.dependencies]\n{core_dep_for_block}\n"
        ));
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
            blocks.push_str(&format!("\n[target.'cfg({})'.dependencies]\n{entry_dep}\n", entry.cfg));
        }
        blocks
    };
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
    if has_streaming_adapters {
        dep_entries.push("futures-util = \"0.3\"".to_string());
    }
    for line in extra_deps.lines() {
        let trimmed = line.trim_end();
        if !trimmed.is_empty() {
            dep_entries.push(trimmed.to_string());
        }
    }
    dep_entries.sort();
    let dep_block = dep_entries.join("\n");
    let _ = streaming_deps;
    let _ = extra_deps_block;

    // `#[cfg(feature = "X")]` arms emitted by the codegen produce
    let cfg_features = shared_cfg::collect_cfg_features(api);
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
    let lints_block = "[lints.rust]\nunexpected_cfgs = { level = \"warn\", check-cfg = ['cfg(frb_expand)'] }";

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
{target_override_blocks}
{lints_block}

[build-dependencies]
swift-bridge-build = "{swift_bridge_build_ver}"
"#
    )
}

/// Emit the `build.rs` content for the generated swift crate.
pub(crate) fn emit_build_rs() -> String {
    r#"// Generated by alef. Do not edit by hand.
use std::path::PathBuf;

fn main() {
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR unset"));
    let crate_name = std::env::var("CARGO_PKG_NAME").expect("CARGO_PKG_NAME unset");
    let bridges = vec!["src/lib.rs"];
    swift_bridge_build::parse_bridges(bridges).write_all_concatenated(out_dir, &crate_name);
    println!("cargo:rerun-if-changed=src/lib.rs");
}
"#
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{ApiSurface, EnumDef, EnumVariant};

    fn make_unit_variant(name: &str, cfg: Option<&str>) -> EnumVariant {
        EnumVariant {
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
}
