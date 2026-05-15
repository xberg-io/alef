//! WASM enum code generation.

use alef_core::ir::EnumDef;
use heck::{ToLowerCamelCase, ToShoutySnakeCase, ToSnakeCase};

use super::functions::emit_rustdoc;

/// Compute the serde wire name for an enum variant, respecting `serde_rename` and
/// `serde_rename_all` on the parent enum.
fn variant_serde_name(variant_name: &str, serde_rename: Option<&str>, serde_rename_all: Option<&str>) -> String {
    if let Some(explicit) = serde_rename {
        return explicit.to_string();
    }
    match serde_rename_all {
        Some("snake_case") => variant_name.to_snake_case(),
        Some("camelCase") => variant_name.to_lower_camel_case(),
        Some("SCREAMING_SNAKE_CASE") => variant_name.to_shouty_snake_case(),
        Some("lowercase") => variant_name.to_lowercase(),
        Some("UPPERCASE") => variant_name.to_uppercase(),
        // PascalCase (default serde), or unknown strategy → keep as-is
        _ => variant_name.to_string(),
    }
}

/// Generate a wasm-bindgen enum definition.
pub(super) fn gen_enum(enum_def: &EnumDef, prefix: &str) -> String {
    let js_name = format!("{prefix}{}", enum_def.name);
    let mut lines = vec![];
    let doc = emit_rustdoc(&enum_def.doc);
    if !doc.is_empty() {
        lines.push(doc);
    }
    lines.extend([
        "#[wasm_bindgen]".to_string(),
        "#[derive(Clone, Copy, PartialEq, Eq)]".to_string(),
        format!("pub enum {} {{", js_name),
    ]);

    for (idx, variant) in enum_def.variants.iter().enumerate() {
        lines.push(format!("    {} = {},", variant.name, idx));
    }

    lines.push("}".to_string());

    // Default impl — prefer the variant marked `is_default`, fall back to first
    let default_variant = enum_def
        .variants
        .iter()
        .find(|v| v.is_default)
        .or_else(|| enum_def.variants.first());
    if let Some(dv) = default_variant {
        lines.push(String::new());
        lines.push("#[allow(clippy::derivable_impls)]".to_string());
        lines.push(format!("impl Default for {} {{", js_name));
        lines.push(format!("    fn default() -> Self {{ Self::{} }}", dv.name));
        lines.push("}".to_string());
    }

    // `to_api_str()` — returns the serde wire string for each variant.
    // Used by struct getters that return `Option<String>` for optional enum fields,
    // ensuring the WASM boundary returns the same snake_case strings as other bindings.
    if !enum_def.variants.is_empty() {
        lines.push(String::new());
        lines.push(format!("impl {} {{", js_name));
        lines.push(
            "    /// Returns the serde wire string for this variant (e.g. `\"stop\"`, `\"tool_calls\"`).".to_string(),
        );
        lines.push("    pub fn to_api_str(self) -> &'static str {".to_string());
        lines.push("        match self {".to_string());
        for variant in &enum_def.variants {
            let wire = variant_serde_name(
                &variant.name,
                variant.serde_rename.as_deref(),
                enum_def.serde_rename_all.as_deref(),
            );
            lines.push(format!("            Self::{} => \"{}\",", variant.name, wire));
        }
        lines.push("        }".to_string());
        lines.push("    }".to_string());
        lines.push("}".to_string());
    }

    lines.join("\n")
}
#[cfg(test)]
mod tests {
    use super::gen_enum;
    use alef_core::ir::{EnumDef, EnumVariant};

    fn make_enum(name: &str, variants: &[&str]) -> EnumDef {
        EnumDef {
            name: name.to_string(),
            rust_path: format!("test::{name}"),
            original_rust_path: String::new(),
            variants: variants
                .iter()
                .map(|v| EnumVariant {
                    name: v.to_string(),
                    fields: vec![],
                    is_tuple: false,
                    doc: String::new(),
                    is_default: false,
                    serde_rename: None,
                })
                .collect(),
            doc: String::new(),
            cfg: None,
            is_copy: true,
            has_serde: false,
            serde_tag: None,
            serde_untagged: false,
            serde_rename_all: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }
    }

    #[test]
    fn gen_enum_produces_wasm_bindgen_attribute() {
        let e = make_enum("Color", &["Red", "Green", "Blue"]);
        let result = gen_enum(&e, "Wasm");
        assert!(result.contains("#[wasm_bindgen]"));
        assert!(result.contains("pub enum WasmColor"));
        assert!(result.contains("Red = 0,"));
        assert!(result.contains("Green = 1,"));
        assert!(result.contains("Blue = 2,"));
    }

    #[test]
    fn gen_enum_empty_variants_no_panic() {
        let e = make_enum("Empty", &[]);
        let result = gen_enum(&e, "");
        assert!(result.contains("pub enum Empty"));
        // No to_api_str() for empty enums
        assert!(!result.contains("to_api_str"));
    }

    #[test]
    fn gen_enum_to_api_str_snake_case() {
        let mut e = make_enum("FinishReason", &["Stop", "ToolCalls", "Length", "ContentFilter"]);
        e.serde_rename_all = Some("snake_case".to_string());
        let result = gen_enum(&e, "Wasm");
        assert!(result.contains("pub fn to_api_str(self) -> &'static str"));
        assert!(result.contains("Self::Stop => \"stop\""));
        assert!(result.contains("Self::ToolCalls => \"tool_calls\""));
        assert!(result.contains("Self::Length => \"length\""));
        assert!(result.contains("Self::ContentFilter => \"content_filter\""));
    }

    #[test]
    fn gen_enum_to_api_str_explicit_rename_overrides_rename_all() {
        let mut e = make_enum("Role", &["User", "Assistant"]);
        e.serde_rename_all = Some("snake_case".to_string());
        // Give "User" an explicit rename
        e.variants[0].serde_rename = Some("human".to_string());
        let result = gen_enum(&e, "Wasm");
        assert!(result.contains("Self::User => \"human\""));
        assert!(result.contains("Self::Assistant => \"assistant\""));
    }

    #[test]
    fn gen_enum_to_api_str_no_rename_all_uses_variant_name() {
        let e = make_enum("Status", &["Active", "Inactive"]);
        let result = gen_enum(&e, "");
        assert!(result.contains("Self::Active => \"Active\""));
        assert!(result.contains("Self::Inactive => \"Inactive\""));
    }
}
