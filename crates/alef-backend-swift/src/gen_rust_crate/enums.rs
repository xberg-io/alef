//! Emits the swift-bridge mirror enum wrapper and its `From` conversion.
//!
//! Only unit variants are exposed in the bridge enum. Data variants are
//! absorbed by a catch-all `Unknown` variant when present.

use alef_codegen::generators::type_paths::resolve_type_path;
use alef_core::ir::EnumDef;
use heck::ToSnakeCase;
use std::collections::HashMap;

pub(crate) fn emit_enum_wrapper(en: &EnumDef, source_crate: &str, type_paths: &HashMap<String, String>) -> String {
    let mut out = String::new();
    let source_path = resolve_type_path(&en.name, source_crate, type_paths);

    // Only emit unit variants. Variants with data fields (tuple or struct variants)
    // cannot be matched as unit patterns in the From impl below, so we expose only
    // the unit variants here. Data variants are bridged as JSON (String) by the
    // JSON-bridge fallback on containing types.
    let unit_variants: Vec<&alef_core::ir::EnumVariant> = en.variants.iter().filter(|v| v.fields.is_empty()).collect();

    // Bridge enum variant names: use the raw Rust identifier from the IR (which is
    // the actual identifier from the kreuzberg source, e.g. "EasyOCR", "RDFa").
    // Do NOT apply to_upper_camel_case() — heck transforms acronyms like "EasyOCR"
    // to "EasyOcr" and "RDFa" to "RdFa", creating names that don't match the source.
    // The bridge enum uses the same names as the kreuzberg source enum so the From impl
    // match arms are valid Rust identifiers on both sides.
    out.push_str(&crate::template_env::render(
        "enum_unit_header.jinja",
        minijinja::context! {
            name => &en.name,
        },
    ));
    for variant in &unit_variants {
        // Use the raw variant name from the IR — it is already a valid Rust identifier.
        out.push_str(&crate::template_env::render(
            "enum_unit_variant.jinja",
            minijinja::context! {
                variant_name => &variant.name,
            },
        ));
    }
    // Add a catch-all variant to absorb any data variants from the source enum
    // that we don't model explicitly. This prevents exhaustiveness failures in the
    // From impl when the source enum has more variants than our bridge enum.
    let has_data_variants = en.variants.len() > unit_variants.len();
    if has_data_variants {
        out.push_str("    /// Data variants not directly bridgeable — represented as Unknown.\n");
        out.push_str("    Unknown,\n");
    }
    out.push_str("}\n\n");

    // From conversion: map unit variants; data variants fall through to Unknown.
    out.push_str(&crate::template_env::render(
        "enum_from_impl_header.jinja",
        minijinja::context! {
            source_path => &source_path,
            name => &en.name,
        },
    ));
    out.push_str("        match val {\n");
    for variant in &unit_variants {
        // Use raw variant name on both sides — source variant name == bridge variant name.
        out.push_str(&crate::template_env::render(
            "enum_from_variant.jinja",
            minijinja::context! {
                source_path => &source_path,
                variant_name => &variant.name,
            },
        ));
    }
    if has_data_variants {
        out.push_str(&crate::template_env::render(
            "enum_from_wildcard.jinja",
            minijinja::context! {},
        ));
    }
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
    for variant in &unit_variants {
        let serde_name = serde_variant_name(variant, en.serde_rename_all.as_deref());
        out.push_str(&format!(
            "            Self::{} => \"{}\".to_string(),\n",
            variant.name, serde_name
        ));
    }
    if has_data_variants {
        out.push_str("            Self::Unknown => \"unknown\".to_string(),\n");
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
fn serde_variant_name(variant: &alef_core::ir::EnumVariant, rename_all: Option<&str>) -> String {
    if let Some(rename) = &variant.serde_rename {
        return rename.clone();
    }
    match rename_all {
        Some("snake_case") => variant.name.to_snake_case(),
        Some("lowercase") => variant.name.to_lowercase(),
        Some("UPPERCASE") => variant.name.to_uppercase(),
        Some("camelCase") => {
            // to_lower_camel_case from heck
            use heck::ToLowerCamelCase;
            variant.name.to_lower_camel_case()
        }
        Some("PascalCase") | Some("UpperCamelCase") => {
            use heck::ToUpperCamelCase;
            variant.name.to_upper_camel_case()
        }
        Some("SCREAMING_SNAKE_CASE") => {
            use heck::ToShoutySnakeCase;
            variant.name.to_shouty_snake_case()
        }
        Some("kebab-case") => {
            use heck::ToKebabCase;
            variant.name.to_kebab_case()
        }
        Some("SCREAMING-KEBAB-CASE") => {
            use heck::ToShoutyKebabCase;
            variant.name.to_shouty_kebab_case()
        }
        _ => variant.name.clone(),
    }
}
