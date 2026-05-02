//! WASM enum code generation.

use alef_core::ir::EnumDef;

use super::functions::emit_rustdoc;

/// Generate a wasm-bindgen enum definition.
pub(super) fn gen_enum(enum_def: &EnumDef, prefix: &str) -> String {
    let js_name = format!("{prefix}{}", enum_def.name);
    let mut lines = vec![];
    let doc = emit_rustdoc(&enum_def.doc);
    if !doc.is_empty() {
        // emit_rustdoc includes trailing newlines; push each line individually
        for line in doc.lines() {
            lines.push(line.to_string());
        }
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
            serde_rename_all: None,
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
    }
}
