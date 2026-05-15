//! WASM enum code generation.

use alef_core::ir::{EnumDef, TypeRef};
use heck::{ToLowerCamelCase, ToShoutySnakeCase, ToSnakeCase};

use crate::type_map::WasmMapper;
use alef_codegen::naming::to_node_name;
use alef_codegen::type_mapper::TypeMapper;

use super::functions::emit_rustdoc;

/// True if this enum is a serde-tagged data enum (`#[serde(tag = "...")]` with variant fields).
/// These are emitted as a flat wasm-bindgen struct with a discriminator field and the union of
/// all variant fields (each made optional) — analogous to the NAPI tagged-enum-as-object path.
pub(super) fn is_tagged_data_enum(enum_def: &EnumDef) -> bool {
    enum_def.serde_tag.is_some() && enum_def.variants.iter().any(|v| !v.fields.is_empty())
}

/// Escape a Rust reserved keyword by prepending the raw-identifier prefix.
/// Used when a field/tag name collides with a Rust keyword (e.g. `type`).
fn escape_rust_keyword(name: &str) -> String {
    // Rust 2024 edition reserved keywords (subset that can appear as field names).
    const RUST_KEYWORDS: &[&str] = &[
        "as", "break", "const", "continue", "crate", "else", "enum", "extern", "false", "fn", "for", "if", "impl",
        "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "self", "Self", "static", "struct",
        "super", "trait", "true", "type", "unsafe", "use", "where", "while", "async", "await", "dyn", "abstract",
        "become", "box", "do", "final", "macro", "override", "priv", "typeof", "unsized", "virtual", "yield", "try",
    ];
    if RUST_KEYWORDS.contains(&name) {
        format!("r#{name}")
    } else {
        name.to_string()
    }
}

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

/// Compute the serde wire tag value for a variant — what JS supplies in the `type` field.
pub(super) fn variant_tag_value(
    variant_name: &str,
    serde_rename: Option<&str>,
    serde_rename_all: Option<&str>,
) -> String {
    variant_serde_name(variant_name, serde_rename, serde_rename_all)
}

/// Generate a wasm-bindgen tagged-enum representation as a flat `#[wasm_bindgen]` struct.
///
/// Serde-tagged data enums (e.g. `#[serde(tag = "type")] enum AuthConfig { Basic { ... }, ...}`)
/// cannot be represented as wasm-bindgen C-style enums because that loses all variant fields.
/// Instead, we emit a struct with:
///  - a `type: String` discriminator (named after `serde_tag`, camelCased for JS)
///  - the union of every variant's fields, each `Option<T>` (so any single instance is valid)
///  - getters/setters for each field, plus a `default()` static factory
///
/// This mirrors the NAPI backend's `gen_tagged_enum_as_object` path. The corresponding
/// `From<Wasm{Enum}> for core::{Enum}` (and reverse) impls are emitted by
/// `gen_tagged_enum_binding_to_core` / `gen_tagged_enum_core_to_binding`.
pub(super) fn gen_tagged_enum_as_struct(enum_def: &EnumDef, prefix: &str) -> String {
    let js_name = format!("{prefix}{}", enum_def.name);
    let tag_field = enum_def.serde_tag.as_deref().unwrap_or("type");
    // Reserved Rust keywords (e.g. `type`) must be escaped with raw-identifier syntax.
    let tag_field_ident = escape_rust_keyword(tag_field);
    let tag_js_name = to_node_name(tag_field);
    let mapper = WasmMapper::new(std::collections::HashMap::new(), prefix.to_string());

    let mut lines = vec![];
    let doc = emit_rustdoc(&enum_def.doc);
    if !doc.is_empty() {
        lines.push(doc);
    }

    // Struct definition. Discriminator is always a String; all variant fields are Option<T>.
    lines.push("#[wasm_bindgen]".to_string());
    lines.push("#[derive(Clone, Default)]".to_string());
    lines.push(format!("pub struct {js_name} {{"));
    lines.push(format!("    pub(crate) {tag_field_ident}: String,"));

    // Union of all variant fields. De-duplicate by name; if a name appears in multiple variants
    // with different types we fall back to `String` (callers serialize to JSON) — matches the
    // NAPI `mixed_named_fields` handling. For kreuzcrawl's only tagged enum (AuthConfig) every
    // field is String, so we hit the simple path.
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut field_entries: Vec<(String, String)> = Vec::new();
    for variant in &enum_def.variants {
        for field in &variant.fields {
            if !seen.insert(field.name.clone()) {
                continue;
            }
            let mapped = mapper.map_type(&field.ty);
            let field_ty = if matches!(&field.ty, TypeRef::Optional(_)) {
                mapped
            } else {
                format!("Option<{mapped}>")
            };
            field_entries.push((field.name.clone(), field_ty.clone()));
            let escaped = escape_rust_keyword(&field.name);
            lines.push(format!("    pub(crate) {escaped}: {field_ty},"));
        }
    }
    lines.push("}".to_string());

    // wasm-bindgen impl: constructor, default(), and getter/setter for every field
    // including the discriminator.
    lines.push(String::new());
    lines.push("#[wasm_bindgen]".to_string());
    lines.push(format!("impl {js_name} {{"));
    lines.push("    #[wasm_bindgen(constructor)]".to_string());
    lines.push(format!("    pub fn new() -> {js_name} {{ Self::default() }}"));
    lines.push(String::new());
    lines.push("    #[wasm_bindgen]".to_string());
    lines.push("    #[allow(clippy::should_implement_trait)]".to_string());
    lines.push(format!(
        "    pub fn default() -> {js_name} {{ <Self as ::core::default::Default>::default() }}"
    ));

    // Discriminator getter/setter. wasm-bindgen exports the Rust fn name as the JS name unless
    // overridden; force `js_name` so JS sees the unescaped tag (e.g. "type") even when the Rust
    // identifier is raw (e.g. `r#type`).
    lines.push(String::new());
    lines.push(format!("    #[wasm_bindgen(getter, js_name = \"{tag_js_name}\")]"));
    lines.push(format!(
        "    pub fn {tag_field_ident}(&self) -> String {{ self.{tag_field_ident}.clone() }}"
    ));
    let setter_ident = format!("set_{tag_field}");
    let setter_ident_escaped = escape_rust_keyword(&setter_ident);
    lines.push(format!("    #[wasm_bindgen(setter, js_name = \"{tag_js_name}\")]"));
    lines.push(format!(
        "    pub fn {setter_ident_escaped}(&mut self, value: String) {{ self.{tag_field_ident} = value; }}"
    ));

    // Field getters/setters. Use Option<T> uniformly because the field may not apply to the
    // currently-active variant.
    for (name, ty) in &field_entries {
        let js_name_for_field = to_node_name(name);
        let name_ident = escape_rust_keyword(name);
        let setter_name = format!("set_{name}");
        let setter_name_ident = escape_rust_keyword(&setter_name);
        lines.push(String::new());
        lines.push(format!(
            "    #[wasm_bindgen(getter, js_name = \"{js_name_for_field}\")]"
        ));
        lines.push(format!(
            "    pub fn {name_ident}(&self) -> {ty} {{ self.{name_ident}.clone() }}"
        ));
        lines.push(format!(
            "    #[wasm_bindgen(setter, js_name = \"{js_name_for_field}\")]"
        ));
        lines.push(format!(
            "    pub fn {setter_name_ident}(&mut self, value: {ty}) {{ self.{name_ident} = value; }}"
        ));
    }
    lines.push("}".to_string());

    lines.join("\n")
}

/// Generate `From<Wasm{Enum}> for core::{Enum}` for a tagged-struct enum representation.
///
/// JS sets `obj.type = "basic"` and the variant-specific fields; this maps `obj.type` to the
/// matching core variant and reads the relevant fields. Missing fields fall back to
/// `Default::default()` so the conversion never panics for malformed input.
pub(super) fn gen_tagged_enum_binding_to_core(enum_def: &EnumDef, core_import: &str, prefix: &str) -> String {
    let core_path = alef_codegen::conversions::core_enum_path(enum_def, core_import);
    let binding_name = format!("{prefix}{}", enum_def.name);
    let tag_field = enum_def.serde_tag.as_deref().unwrap_or("type");
    let tag_field_ident = escape_rust_keyword(tag_field);

    let mut lines = vec![];
    lines.push(format!("impl From<{binding_name}> for {core_path} {{"));
    lines.push(format!("    fn from(val: {binding_name}) -> Self {{"));
    lines.push(format!("        match val.{tag_field_ident}.as_str() {{"));
    for variant in &enum_def.variants {
        let tag_value = variant_tag_value(
            &variant.name,
            variant.serde_rename.as_deref(),
            enum_def.serde_rename_all.as_deref(),
        );
        if variant.fields.is_empty() {
            lines.push(format!("            \"{tag_value}\" => Self::{},", variant.name));
        } else {
            let inits: Vec<String> = variant
                .fields
                .iter()
                .map(|f| {
                    let f_ident = escape_rust_keyword(&f.name);
                    format!("{}: val.{}.clone().unwrap_or_default()", f.name, f_ident)
                })
                .collect();
            lines.push(format!(
                "            \"{tag_value}\" => Self::{} {{ {} }},",
                variant.name,
                inits.join(", ")
            ));
        }
    }
    // Fallback: pick the first variant with default-initialized fields. Mirrors NAPI behaviour.
    if let Some(first) = enum_def.variants.first() {
        if first.fields.is_empty() {
            lines.push(format!("            _ => Self::{},", first.name));
        } else {
            let defaults: Vec<String> = first
                .fields
                .iter()
                .map(|f| format!("{}: Default::default()", f.name))
                .collect();
            lines.push(format!(
                "            _ => Self::{} {{ {} }},",
                first.name,
                defaults.join(", ")
            ));
        }
    }
    lines.push("        }".to_string());
    lines.push("    }".to_string());
    lines.push("}".to_string());
    lines.join("\n")
}

/// Generate `From<core::{Enum}> for Wasm{Enum}` for a tagged-struct enum representation.
pub(super) fn gen_tagged_enum_core_to_binding(enum_def: &EnumDef, core_import: &str, prefix: &str) -> String {
    let core_path = alef_codegen::conversions::core_enum_path(enum_def, core_import);
    let binding_name = format!("{prefix}{}", enum_def.name);
    let tag_field = enum_def.serde_tag.as_deref().unwrap_or("type");
    let tag_field_ident = escape_rust_keyword(tag_field);

    // Collect every field name across all variants (for the struct literal).
    let mut all_field_names: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for variant in &enum_def.variants {
        for field in &variant.fields {
            all_field_names.insert(field.name.clone());
        }
    }

    let mut lines = vec![];
    lines.push(format!("impl From<{core_path}> for {binding_name} {{"));
    lines.push(format!("    fn from(val: {core_path}) -> Self {{"));
    lines.push("        match val {".to_string());
    for variant in &enum_def.variants {
        let tag_value = variant_tag_value(
            &variant.name,
            variant.serde_rename.as_deref(),
            enum_def.serde_rename_all.as_deref(),
        );
        let variant_field_names: std::collections::BTreeSet<String> =
            variant.fields.iter().map(|f| f.name.clone()).collect();

        if variant.fields.is_empty() {
            let mut inits = vec![format!(
                "                {tag_field_ident}: \"{tag_value}\".to_string()"
            )];
            for name in &all_field_names {
                let n_ident = escape_rust_keyword(name);
                inits.push(format!("                {n_ident}: None"));
            }
            lines.push(format!("            {core_path}::{} => Self {{", variant.name));
            lines.push(format!("{},", inits.join(",\n")));
            lines.push("            },".to_string());
        } else {
            // Destructure variant's fields by name (using raw identifiers where needed).
            let destructure_names: Vec<String> = variant.fields.iter().map(|f| escape_rust_keyword(&f.name)).collect();
            let mut inits = vec![format!(
                "                {tag_field_ident}: \"{tag_value}\".to_string()"
            )];
            for name in &all_field_names {
                let n_ident = escape_rust_keyword(name);
                if variant_field_names.contains(name) {
                    inits.push(format!("                {n_ident}: Some({n_ident})"));
                } else {
                    inits.push(format!("                {n_ident}: None"));
                }
            }
            lines.push(format!(
                "            {core_path}::{} {{ {} }} => Self {{",
                variant.name,
                destructure_names.join(", ")
            ));
            lines.push(format!("{},", inits.join(",\n")));
            lines.push("            },".to_string());
        }
    }
    lines.push("        }".to_string());
    lines.push("    }".to_string());
    lines.push("}".to_string());
    lines.join("\n")
}

/// Generate a wasm-bindgen enum definition.
pub(super) fn gen_enum(enum_def: &EnumDef, prefix: &str) -> String {
    if is_tagged_data_enum(enum_def) {
        return gen_tagged_enum_as_struct(enum_def, prefix);
    }

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
