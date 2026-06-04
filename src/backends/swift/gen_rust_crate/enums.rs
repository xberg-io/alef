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

    // Bridge enum variant names: use the raw Rust identifier from the IR (which is
    // the actual identifier from the sample_core source, e.g. "EasyOCR", "RDFa").
    // Do NOT apply to_upper_camel_case() — heck transforms acronyms like "EasyOCR"
    // to "EasyOcr" and "RDFa" to "RdFa", creating names that don't match the source.
    // The bridge enum uses the same names as the sample_core source enum so the From impl
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

    for variant in &en.variants {
        let pattern = if variant.fields.is_empty() {
            variant.name.clone()
        } else if variant.is_tuple {
            format!("{}(..)", variant.name)
        } else {
            format!("{} {{ .. }}", variant.name)
        };

        out.push_str(&crate::backends::swift::template_env::render(
            "enum_from_variant.jinja",
            minijinja::context! {
                source_path => &source_path,
                variant_name => &variant.name,
                pattern => pattern,
            },
        ));
    }

    // Emit unreachable!() arms for binding-excluded variants (e.g. feature-gated variants).
    // The source enum may have variants not in the bridge (binding_excluded or feature-gated),
    // so we need a catch-all to make the match exhaustive. Feature-gated variants that don't
    // appear in the IR are still valid values at runtime, but the bridge doesn't expose them,
    // so we use unreachable!() to indicate this is a logic error in the guard conditions.
    // Always emit a wildcard arm to handle variants excluded from the binding generation
    // (even if not explicitly marked as binding_excluded in the IR, they may exist at runtime
    // when different feature combinations are used).
    out.push_str("            _ => unreachable!(\"bridge enum variant not exposed in binding\"),\n");

    out.push_str("        }\n");
    out.push_str("    }\n");
    out.push_str("}\n\n");

    // `to_string` impl — returns the serialized (serde) name of the variant so that
    // swift-bridge can expose it as a `toString() -> RustString` Swift method.
    // This lets e2e tests do `linkType().toString().toString()` to get "anchor" etc.
    // instead of relying on `String(describing:)` which yields the opaque class description.
    out.push_str(&format!("impl {} {{\n", en.name));
    out.push_str("    pub fn to_string(&self) -> String {\n");
    out.push_str("        match self {\n");

    for variant in &en.variants {
        let serde_name = serde_variant_wire_name(variant, en.serde_rename_all.as_deref());
        out.push_str(&format!(
            "            Self::{} => \"{}\".to_string(),\n",
            variant.name, serde_name
        ));
    }

    out.push_str("        }\n");
    out.push_str("    }\n");
    out.push_str("}\n");

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
