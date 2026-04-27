//! Emits the swift-bridge mirror enum wrapper and its `From` conversion.
//!
//! Only unit variants are exposed in the bridge enum. Data variants are
//! absorbed by a catch-all `Unknown` variant when present.

use alef_codegen::generators::type_paths::resolve_type_path;
use alef_core::ir::EnumDef;
use std::collections::HashMap;

pub(crate) fn emit_enum_wrapper(en: &EnumDef, source_crate: &str, type_paths: &HashMap<String, String>) -> String {
    let mut out = String::new();
    let source_path = resolve_type_path(&en.name, source_crate, type_paths);

    // Only emit unit variants. Variants with data fields (tuple or struct variants)
    // cannot be matched as unit patterns in the From impl below, so we expose only
    // the unit variants here. Data variants are bridged as JSON (String) by the
    // JSON-bridge fallback on containing types.
    let unit_variants: Vec<&alef_core::ir::EnumVariant> =
        en.variants.iter().filter(|v| v.fields.is_empty()).collect();

    // Bridge enum variant names: use the raw Rust identifier from the IR (which is
    // the actual identifier from the kreuzberg source, e.g. "EasyOCR", "RDFa").
    // Do NOT apply to_upper_camel_case() — heck transforms acronyms like "EasyOCR"
    // to "EasyOcr" and "RDFa" to "RdFa", creating names that don't match the source.
    // The bridge enum uses the same names as the kreuzberg source enum so the From impl
    // match arms are valid Rust identifiers on both sides.
    out.push_str(&format!("pub enum {} {{\n", en.name));
    for variant in &unit_variants {
        // Use the raw variant name from the IR — it is already a valid Rust identifier.
        out.push_str(&format!("    {},\n", variant.name));
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
    out.push_str(&format!("impl From<{source_path}> for {} {{\n", en.name));
    out.push_str(&format!("    fn from(val: {source_path}) -> Self {{\n"));
    out.push_str("        match val {\n");
    for variant in &unit_variants {
        // Use raw variant name on both sides — source variant name == bridge variant name.
        out.push_str(&format!(
            "            {source_path}::{} => Self::{},\n",
            variant.name, variant.name
        ));
    }
    if has_data_variants {
        out.push_str("            _ => Self::Unknown,\n");
    }
    out.push_str("        }\n");
    out.push_str("    }\n");
    out.push_str("}\n");

    out
}
