//! Emits the swift-bridge mirror enum wrapper and its `From` conversion.
//!
//! Only unit variants are exposed in the bridge enum. Data variants are
//! absorbed by a catch-all `Unknown` variant when present.

use crate::codegen::generators::type_paths::resolve_type_path;
use crate::core::ir::EnumDef;
use std::collections::HashMap;

pub(crate) fn emit_enum_wrapper(en: &EnumDef, source_crate: &str, type_paths: &HashMap<String, String>) -> String {
    let mut out = String::new();
    let source_path = resolve_type_path(&en.name, source_crate, type_paths);

    // Emit all variants (both unit and data-bearing) as unit-only variants in the bridge enum.
    // This preserves the variant tag for all source enum variants, allowing consumers to
    // distinguish them without collapsing data-bearing variants to Unknown.

    // Bridge enum variant names: use the raw Rust identifier from the IR.
    // Do NOT apply to_upper_camel_case() — heck transforms acronyms like "EasyOCR"
    // to "EasyOcr" and "RDFa" to "RdFa", creating names that don't match the source.
    // The bridge enum uses the same names as the source enum so the From impl
    // match arms are valid Rust identifiers on both sides.
    out.push_str(&crate::backends::swift::template_env::render(
        "enum_unit_header.jinja",
        minijinja::context! {
            name => &en.name,
        },
    ));
    for variant in &en.variants {
        out.push_str(&crate::backends::swift::template_env::render(
            "enum_unit_variant.jinja",
            minijinja::context! {
                variant_name => &variant.name,
            },
        ));
    }

    out.push_str("}\n\n");

    // From conversion: match all source variants and map to the corresponding bridge variant.
    out.push_str(&crate::backends::swift::template_env::render(
        "enum_from_impl_header.jinja",
        minijinja::context! {
            source_path => &source_path,
            name => &en.name,
        },
    ));
    out.push_str("        match val {\n");

    let has_cfg_variants = en.variants.iter().any(|v| v.cfg.is_some());

    for variant in &en.variants {
        let pattern = if variant.fields.is_empty() {
            variant.name.clone()
        } else if variant.is_tuple {
            format!("{}(..)", variant.name)
        } else {
            format!("{} {{ .. }}", variant.name)
        };

        // Mirror the dart enum_conversions emitter: variants gated by upstream `#[cfg(...)]`
        // (e.g. `Heif` under `#[cfg(feature = "heic")]`) must carry that same gate on the
        // From-impl arm so cross-compiling against feature sets that drop the variant (iOS
        // uses `android-target` which excludes `heic`) compiles cleanly.
        if let Some(condition) = variant.cfg.as_deref() {
            out.push_str("            #[cfg(");
            out.push_str(condition);
            out.push_str(")]\n");
        }

        out.push_str(&crate::backends::swift::template_env::render(
            "enum_from_variant.jinja",
            minijinja::context! {
                source_path => &source_path,
                variant_name => &variant.name,
                pattern => pattern,
            },
        ));
    }

    // Emit a catch-all wildcard arm whenever either of these conditions holds:
    //
    // 1. `excluded_variants` is non-empty: source variants live outside `variants`;
    //    the match arms only cover `variants`, making it non-exhaustive (E0004) under
    //    `--all-features`.
    //
    // 2. Any `variants` entry carries a `#[cfg(feature = "X")]` gate: when that feature
    //    is inactive the arm is compiled out, also making the match non-exhaustive.
    //
    // `#![allow(unreachable_patterns)]` at the crate root suppresses the redundant-arm
    // warning when all variants are in fact covered.
    if !en.excluded_variants.is_empty() || has_cfg_variants {
        out.push_str(&format!(
            "            _ => unreachable!(\"bridge enum variant of {} not exposed in binding\"),\n",
            en.name
        ));
    }

    out.push_str("        }\n");
    out.push_str("    }\n");
    out.push_str("}\n\n");

    // `to_string` impl — returns the serialized (serde) name of the variant so that
    // swift-bridge can expose it as a `toString() -> RustString` Swift method.
    // This lets e2e tests do `linkType().toString().toString()` to get "anchor" etc.
    // instead of relying on `String(describing:)` which yields the opaque class description.
    let mut variants = String::new();
    for variant in &en.variants {
        let serde_name = serde_variant_wire_name(variant, en.serde_rename_all.as_deref());
        variants.push_str(&crate::backends::swift::template_env::render(
            "rust_enum_to_string_variant.rs.jinja",
            minijinja::context! {
                variant_name => &variant.name,
                serde_name => &serde_name,
            },
        ));
    }

    out.push_str(&crate::backends::swift::template_env::render(
        "rust_enum_to_string_impl.rs.jinja",
        minijinja::context! {
            enum_name => &en.name,
            variants => variants,
        },
    ));

    out
}

/// Compute the serde-serialized name for a unit enum variant.
///
/// Priority order:
/// 1. Explicit `#[serde(rename = "...")]` on the variant.
/// 2. `rename_all` transformation applied to the Rust identifier.
/// 3. Raw Rust identifier (no transformation).
fn serde_variant_wire_name(variant: &crate::core::ir::EnumVariant, rename_all: Option<&str>) -> String {
    crate::codegen::naming::wire_variant_value(&variant.name, variant.serde_rename.as_deref(), rename_all)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{EnumDef, EnumVariant};

    fn make_unit_variant(name: &str, cfg: Option<&str>) -> EnumVariant {
        EnumVariant {
            name: name.to_string(),
            cfg: cfg.map(str::to_string),
            ..Default::default()
        }
    }

    /// When any variant in the primary list carries a `#[cfg(...)]` gate the
    /// From-impl match must emit a `_ => unreachable!()` catch-all arm so it
    /// remains exhaustive when that feature is inactive (E0004 guard).
    #[test]
    fn cfg_gated_variant_emits_catch_all_in_from_impl() {
        let en = EnumDef {
            name: "ImageOutputFormat".to_string(),
            variants: vec![
                make_unit_variant("Jpeg", None),
                make_unit_variant("Heif", Some(r#"feature = "heic""#)),
            ],
            methods: vec![],
            excluded_variants: vec![],
            ..Default::default()
        };
        let type_paths = std::collections::HashMap::new();
        let out = emit_enum_wrapper(&en, "mylib", &type_paths);
        assert!(
            out.contains("_ => unreachable!"),
            "expected catch-all `_ => unreachable!` arm when cfg-gated variant present, got:\n{out}"
        );
        assert!(
            out.contains("ImageOutputFormat"),
            "catch-all message must include the enum name, got:\n{out}"
        );
    }

    /// When no variant is cfg-gated and `excluded_variants` is empty, no catch-all
    /// should be emitted (the match is statically exhaustive without it).
    #[test]
    fn no_cfg_or_excluded_variants_does_not_emit_catch_all() {
        let en = EnumDef {
            name: "SimpleEnum".to_string(),
            variants: vec![make_unit_variant("A", None), make_unit_variant("B", None)],
            methods: vec![],
            excluded_variants: vec![],
            ..Default::default()
        };
        let type_paths = std::collections::HashMap::new();
        let out = emit_enum_wrapper(&en, "mylib", &type_paths);
        assert!(
            !out.contains("_ => unreachable!"),
            "unexpected catch-all arm in From impl for fully-covered enum:\n{out}"
        );
    }

    /// `excluded_variants` alone (no inline cfg gates) must still trigger the catch-all.
    #[test]
    fn excluded_variants_alone_emits_catch_all() {
        let en = EnumDef {
            name: "ImageOutputFormat".to_string(),
            variants: vec![make_unit_variant("Jpeg", None)],
            methods: vec![],
            excluded_variants: vec![make_unit_variant("ExcludedVariant", None)],
            ..Default::default()
        };
        let type_paths = std::collections::HashMap::new();
        let out = emit_enum_wrapper(&en, "mylib", &type_paths);
        assert!(
            out.contains("_ => unreachable!"),
            "expected catch-all arm when excluded_variants is non-empty, got:\n{out}"
        );
    }
}
