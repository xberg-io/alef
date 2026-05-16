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

/// Compute the set of field names that appear in multiple variants with different Named types.
///
/// When the same positional or named field (e.g. `_0`) carries a different inner type per
/// variant (e.g. `SystemMessage` vs `UserMessage`), the binding struct cannot represent it
/// as a single concrete WASM type. Instead the field is stored as `JsValue` and converted
/// via `serde_wasm_bindgen` per-variant in the From impls.
fn mixed_type_fields(enum_def: &EnumDef) -> std::collections::BTreeSet<String> {
    let mut field_types: std::collections::HashMap<String, std::collections::BTreeSet<String>> =
        std::collections::HashMap::new();
    for variant in &enum_def.variants {
        for field in &variant.fields {
            if let TypeRef::Named(n) = &field.ty {
                field_types.entry(field.name.clone()).or_default().insert(n.clone());
            }
        }
    }
    field_types
        .into_iter()
        .filter(|(_, types)| types.len() > 1)
        .map(|(name, _)| name)
        .collect()
}

fn tagged_enum_binding_to_core_expr(field_ident: &str, field_ty: &TypeRef, field_optional: bool) -> String {
    if field_optional {
        return match field_ty {
            TypeRef::Named(_) => format!("val.{field_ident}.clone().map(Into::into)"),
            _ => format!("val.{field_ident}.clone()"),
        };
    }
    match field_ty {
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(_) => format!("val.{field_ident}.clone().map(Into::into)"),
            _ => format!("val.{field_ident}.clone()"),
        },
        TypeRef::Named(_) => format!("val.{field_ident}.clone().map(Into::into).unwrap_or_default()"),
        _ => format!("val.{field_ident}.clone().unwrap_or_default()"),
    }
}

fn tagged_enum_core_to_binding_expr(
    field_ident: &str,
    local: &str,
    field_ty: &TypeRef,
    field_optional: bool,
) -> String {
    if field_optional {
        return match field_ty {
            TypeRef::Named(_) => format!("                {field_ident}: {local}.map(Into::into)"),
            _ => format!("                {field_ident}: {local}"),
        };
    }
    match field_ty {
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(_) => format!("                {field_ident}: {local}.map(Into::into)"),
            _ => format!("                {field_ident}: {local}"),
        },
        TypeRef::Named(_) => format!("                {field_ident}: Some({local}.into())"),
        _ => format!("                {field_ident}: Some({local})"),
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
    lines.push(format!("#[wasm_bindgen(js_name = \"{}\")]", enum_def.name));
    lines.push("#[derive(Clone, Default)]".to_string());
    lines.push(format!("pub struct {js_name} {{"));
    lines.push(format!("    pub(crate) {tag_field_ident}: String,"));

    // Union of all variant fields. De-duplicate by name; if a name appears in multiple variants
    // with different Named types (e.g. `_0: SystemMessage` vs `_0: UserMessage`) we fall back
    // to `JsValue` (callers convert via serde_wasm_bindgen per-variant in the From impls).
    // This mirrors the NAPI `mixed_named_fields` / `serde_json` path.
    let mixed = mixed_type_fields(enum_def);
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut field_entries: Vec<(String, String)> = Vec::new();
    for variant in &enum_def.variants {
        for field in &variant.fields {
            if !seen.insert(field.name.clone()) {
                continue;
            }
            let field_ty = if mixed.contains(&field.name) {
                // Mixed-type Named field: store as JsValue, convert via serde_wasm_bindgen.
                "Option<JsValue>".to_string()
            } else {
                let mapped = mapper.map_type(&field.ty);
                if matches!(&field.ty, TypeRef::Optional(_)) {
                    mapped
                } else {
                    format!("Option<{mapped}>")
                }
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
    //
    // Positional fields from tuple-variant wrapper structs are named `_0`, `_1`, … by the
    // extractor.  Rust's non_snake_case lint rejects `set__0` (the naive `format!("set_{name}")`)
    // because of the double-underscore.  We rename the Rust identifier to `set_field_0` (getter
    // `field_0`) while keeping the JS-visible name unchanged via the `js_name` attribute.
    for (name, ty) in &field_entries {
        let js_name_for_field = to_node_name(name);
        let field_name = name.as_str();
        // Detect positional fields: `_` followed by one or more ASCII digits.
        let rust_getter_ident = if field_name.starts_with('_')
            && field_name.len() > 1
            && field_name[1..].chars().all(|c| c.is_ascii_digit())
        {
            format!("field_{}", &field_name[1..])
        } else {
            escape_rust_keyword(field_name)
        };
        let rust_setter_ident = format!("set_{rust_getter_ident}");
        let struct_field_ident = escape_rust_keyword(field_name);
        lines.push(String::new());
        lines.push(format!(
            "    #[wasm_bindgen(getter, js_name = \"{js_name_for_field}\")]"
        ));
        lines.push(format!(
            "    pub fn {rust_getter_ident}(&self) -> {ty} {{ self.{struct_field_ident}.clone() }}"
        ));
        lines.push(format!(
            "    #[wasm_bindgen(setter, js_name = \"{js_name_for_field}\")]"
        ));
        lines.push(format!(
            "    pub fn {rust_setter_ident}(&mut self, value: {ty}) {{ self.{struct_field_ident} = value; }}"
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
    let mixed = mixed_type_fields(enum_def);

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
        } else if variant.is_tuple {
            // Tuple/newtype variant: construct positionally.
            // For mixed-type Named fields the binding struct stores `JsValue`; deserialize
            // via serde_wasm_bindgen to the variant-specific core type. For single-type
            // Named fields the binding struct stores the binding wrapper; call `.into()`.
            let args: Vec<String> = variant
                .fields
                .iter()
                .map(|f| {
                    let f_ident = escape_rust_keyword(&f.name);
                    if mixed.contains(&f.name) {
                        // Field is stored as JsValue in the binding struct.
                        let core_inner = match &f.ty {
                            TypeRef::Named(n) => format!("{core_import}::{n}"),
                            _ => "serde_json::Value".to_string(),
                        };
                        format!(
                            "val.{f_ident}.as_ref().and_then(|v| serde_wasm_bindgen::from_value::<{core_inner}>(v.clone()).ok()).unwrap_or_default()"
                        )
                    } else {
                        // Single-type Named or primitive field.
                        tagged_enum_binding_to_core_expr(&f_ident, &f.ty, f.optional)
                    }
                })
                .collect();
            lines.push(format!(
                "            \"{tag_value}\" => Self::{}({}),",
                variant.name,
                args.join(", ")
            ));
        } else {
            // Struct variant: construct with named fields.
            // For Named-type fields the binding struct stores the binding wrapper type
            // (e.g. `Option<WasmImageUrl>`), so we must call `.into()` to convert to
            // the core type (e.g. `ImageUrl`).
            let inits: Vec<String> = variant
                .fields
                .iter()
                .map(|f| {
                    let f_ident = escape_rust_keyword(&f.name);
                    format!(
                        "{}: {}",
                        f.name,
                        tagged_enum_binding_to_core_expr(&f_ident, &f.ty, f.optional)
                    )
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
        } else if first.is_tuple {
            let args: Vec<String> = first.fields.iter().map(|_| "Default::default()".to_string()).collect();
            lines.push(format!("            _ => Self::{}({}),", first.name, args.join(", ")));
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
    let mixed = mixed_type_fields(enum_def);

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
        } else if variant.is_tuple {
            // Tuple/newtype variant: destructure positionally.
            // Use synthetic local names `field0`, `field1`, … to avoid conflicts with
            // Rust keywords and to keep the binding struct field names separate.
            let local_names: Vec<String> = variant
                .fields
                .iter()
                .enumerate()
                .map(|(i, _)| format!("field{i}"))
                .collect();
            let destructure = local_names.join(", ");
            let mut inits = vec![format!(
                "                {tag_field_ident}: \"{tag_value}\".to_string()"
            )];
            for name in &all_field_names {
                let n_ident = escape_rust_keyword(name);
                if variant_field_names.contains(name) {
                    // Map the positional field (e.g. `_0`) to the local binding name.
                    let pos = variant.fields.iter().position(|f| &f.name == name).unwrap();
                    let local = &local_names[pos];
                    let init = if mixed.contains(name) {
                        // Mixed-type Named field: stored as JsValue in the binding struct.
                        format!("                {n_ident}: serde_wasm_bindgen::to_value(&{local}).ok()")
                    } else if let Some(field) = variant.fields.iter().find(|f| &f.name == name) {
                        tagged_enum_core_to_binding_expr(&n_ident, local, &field.ty, field.optional)
                    } else {
                        format!("                {n_ident}: None")
                    };
                    inits.push(init);
                } else {
                    inits.push(format!("                {n_ident}: None"));
                }
            }
            lines.push(format!(
                "            {core_path}::{}({}) => Self {{",
                variant.name, destructure
            ));
            lines.push(format!("{},", inits.join(",\n")));
            lines.push("            },".to_string());
        } else {
            // Struct variant: destructure by field name.
            // For Named-type fields the binding struct expects the binding wrapper type
            // (e.g. `Option<WasmImageUrl>`), so wrap with `.into()` to convert the core
            // value (e.g. `ImageUrl`) to the binding type.
            let destructure_names: Vec<String> = variant.fields.iter().map(|f| escape_rust_keyword(&f.name)).collect();
            let mut inits = vec![format!(
                "                {tag_field_ident}: \"{tag_value}\".to_string()"
            )];
            for name in &all_field_names {
                let n_ident = escape_rust_keyword(name);
                if variant_field_names.contains(name) {
                    let init = if let Some(field) = variant.fields.iter().find(|f| &f.name == name) {
                        tagged_enum_core_to_binding_expr(&n_ident, &n_ident, &field.ty, field.optional)
                    } else {
                        format!("                {n_ident}: None")
                    };
                    inits.push(init);
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
        format!("#[wasm_bindgen(js_name = \"{}\")]", enum_def.name),
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

        // `from_api_str()` — parses a serde wire string and returns the corresponding variant.
        // Used by Vec<UnitEnum> parameter deserialization to convert string values from JS.
        lines.push(String::new());
        lines.push(
            "    /// Parses a serde wire string and returns the corresponding variant, or None if unrecognized."
                .to_string(),
        );
        lines.push("    pub fn from_api_str(s: &str) -> Option<Self> {".to_string());
        lines.push("        match s {".to_string());
        for variant in &enum_def.variants {
            let wire = variant_serde_name(
                &variant.name,
                variant.serde_rename.as_deref(),
                enum_def.serde_rename_all.as_deref(),
            );
            lines.push(format!("            \"{}\" => Some(Self::{}),", wire, variant.name));
        }
        lines.push("            _ => None,".to_string());
        lines.push("        }".to_string());
        lines.push("    }".to_string());

        lines.push("}".to_string());
    }

    lines.join("\n")
}
#[cfg(test)]
mod tests {
    use super::{gen_enum, gen_tagged_enum_binding_to_core, gen_tagged_enum_core_to_binding};
    use alef_core::ir::{EnumDef, EnumVariant, FieldDef, TypeRef};

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

    /// Build a tagged enum where every non-empty variant is a newtype/tuple variant
    /// (single positional field named `_0`), as emitted by the alef extractor for
    /// `pub enum Message { System(SystemMessage), User(UserMessage) }`.
    fn make_tagged_tuple_enum() -> EnumDef {
        let make_tuple_variant = |variant_name: &str, tag: &str| EnumVariant {
            name: variant_name.to_string(),
            fields: vec![FieldDef {
                name: "_0".to_string(),
                ty: TypeRef::Named(format!("{variant_name}Message")),
                optional: false,
                default: None,
                doc: String::new(),
                sanitized: false,
                is_boxed: false,
                type_rust_path: None,
                cfg: None,
                typed_default: None,
                core_wrapper: alef_core::ir::CoreWrapper::None,
                vec_inner_core_wrapper: alef_core::ir::CoreWrapper::None,
                newtype_wrapper: None,
                serde_rename: Some(tag.to_string()),
                serde_flatten: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
            }],
            is_tuple: true,
            doc: String::new(),
            is_default: false,
            serde_rename: Some(tag.to_string()),
        };

        EnumDef {
            name: "Message".to_string(),
            rust_path: "test_lib::types::Message".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                make_tuple_variant("System", "system"),
                make_tuple_variant("User", "user"),
            ],
            doc: String::new(),
            cfg: None,
            is_copy: false,
            has_serde: true,
            serde_tag: Some("role".to_string()),
            serde_untagged: false,
            serde_rename_all: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }
    }

    /// Regression test: `gen_tagged_enum_core_to_binding` must emit tuple-pattern destructuring
    /// (`EnumName::Variant(field0)`) for tuple/newtype variants, not struct-pattern
    /// (`EnumName::Variant { _0 }`).
    #[test]
    fn gen_tagged_enum_core_to_binding_uses_tuple_pattern_for_tuple_variants() {
        let e = make_tagged_tuple_enum();
        let result = gen_tagged_enum_core_to_binding(&e, "test_lib", "Wasm");

        // Must NOT use struct-pattern destructure for tuple variants.
        assert!(
            !result.contains("Message::System { _0 }"),
            "must not emit struct destructure for tuple variant;\nactual:\n{result}"
        );
        assert!(
            !result.contains("Message::User { _0 }"),
            "must not emit struct destructure for tuple variant;\nactual:\n{result}"
        );

        // Must use tuple-pattern destructure.
        assert!(
            result.contains("Message::System(field0)"),
            "must emit tuple destructure for tuple variant;\nactual:\n{result}"
        );
        assert!(
            result.contains("Message::User(field0)"),
            "must emit tuple destructure for tuple variant;\nactual:\n{result}"
        );

        // The positional value must be converted and stored in the `_0` binding struct field.
        // Since the variants have different Named types, the struct stores JsValue and the
        // conversion uses serde_wasm_bindgen.
        assert!(
            result.contains("_0: serde_wasm_bindgen::to_value(&field0).ok()"),
            "positional value must be serialized via serde_wasm_bindgen into _0 field;\nactual:\n{result}"
        );
    }

    /// Regression test: `gen_tagged_enum_binding_to_core` must emit tuple construction
    /// (`Self::Variant(val)`) for tuple/newtype variants, not struct construction
    /// (`Self::Variant { _0: val }`).
    #[test]
    fn gen_tagged_enum_binding_to_core_uses_tuple_construction_for_tuple_variants() {
        let e = make_tagged_tuple_enum();
        let result = gen_tagged_enum_binding_to_core(&e, "test_lib", "Wasm");

        // Must NOT use struct-construction syntax for tuple variants.
        assert!(
            !result.contains("Self::System { _0:"),
            "must not emit struct construction for tuple variant;\nactual:\n{result}"
        );
        assert!(
            !result.contains("Self::User { _0:"),
            "must not emit struct construction for tuple variant;\nactual:\n{result}"
        );

        // Must use tuple construction.
        assert!(
            result.contains("Self::System("),
            "must emit tuple construction for tuple variant;\nactual:\n{result}"
        );
        assert!(
            result.contains("Self::User("),
            "must emit tuple construction for tuple variant;\nactual:\n{result}"
        );

        // Mixed-type Named fields must use serde_wasm_bindgen::from_value to deserialize
        // the JsValue binding struct field to the variant-specific core type.
        assert!(
            result.contains("serde_wasm_bindgen::from_value::<test_lib::SystemMessage>"),
            "binding→core must deserialize mixed-type field via serde_wasm_bindgen;\nactual:\n{result}"
        );
        assert!(
            result.contains("serde_wasm_bindgen::from_value::<test_lib::UserMessage>"),
            "binding→core must deserialize mixed-type field via serde_wasm_bindgen;\nactual:\n{result}"
        );
    }

    /// Smoke test: a tagged enum with plain unit variants (no fields) is unaffected by the
    /// tuple-variant fix and still emits valid unit-variant arms.
    #[test]
    fn gen_tagged_enum_core_to_binding_unit_variants_unchanged() {
        let e = EnumDef {
            name: "Status".to_string(),
            rust_path: "test_lib::Status".to_string(),
            original_rust_path: String::new(),
            variants: vec![
                EnumVariant {
                    name: "Active".to_string(),
                    fields: vec![],
                    is_tuple: false,
                    doc: String::new(),
                    is_default: false,
                    serde_rename: Some("active".to_string()),
                },
                EnumVariant {
                    name: "Inactive".to_string(),
                    fields: vec![],
                    is_tuple: false,
                    doc: String::new(),
                    is_default: false,
                    serde_rename: Some("inactive".to_string()),
                },
            ],
            doc: String::new(),
            cfg: None,
            is_copy: true,
            has_serde: true,
            serde_tag: Some("state".to_string()),
            serde_untagged: false,
            serde_rename_all: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        };

        let core_to_binding = gen_tagged_enum_core_to_binding(&e, "test_lib", "Wasm");
        // Unit variants must still emit simple `CorePath::Variant => Self { ... }` arms.
        assert!(
            core_to_binding.contains("test_lib::Status::Active => Self {"),
            "unit variant arm must use simple path;\nactual:\n{core_to_binding}"
        );

        let binding_to_core = gen_tagged_enum_binding_to_core(&e, "test_lib", "Wasm");
        // Unit variants in binding→core direction: `"active" => Self::Active`
        assert!(
            binding_to_core.contains("\"active\" => Self::Active"),
            "unit variant arm must match tag string;\nactual:\n{binding_to_core}"
        );
    }

    /// Smoke test: a tagged enum with struct variants (named fields) is unaffected and still
    /// emits struct-pattern destructuring.
    #[test]
    fn gen_tagged_enum_core_to_binding_struct_variants_unchanged() {
        let e = EnumDef {
            name: "Auth".to_string(),
            rust_path: "test_lib::Auth".to_string(),
            original_rust_path: String::new(),
            variants: vec![EnumVariant {
                name: "Basic".to_string(),
                fields: vec![FieldDef {
                    name: "username".to_string(),
                    ty: TypeRef::String,
                    optional: false,
                    default: None,
                    doc: String::new(),
                    sanitized: false,
                    is_boxed: false,
                    type_rust_path: None,
                    cfg: None,
                    typed_default: None,
                    core_wrapper: alef_core::ir::CoreWrapper::None,
                    vec_inner_core_wrapper: alef_core::ir::CoreWrapper::None,
                    newtype_wrapper: None,
                    serde_rename: None,
                    serde_flatten: false,
                    binding_excluded: false,
                    binding_exclusion_reason: None,
                }],
                is_tuple: false, // struct variant
                doc: String::new(),
                is_default: false,
                serde_rename: Some("basic".to_string()),
            }],
            doc: String::new(),
            cfg: None,
            is_copy: false,
            has_serde: true,
            serde_tag: Some("type".to_string()),
            serde_untagged: false,
            serde_rename_all: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        };

        let result = gen_tagged_enum_core_to_binding(&e, "test_lib", "Wasm");
        // Struct variant must still use `{ username }` destructure.
        assert!(
            result.contains("Auth::Basic { username }"),
            "struct variant must keep struct destructure;\nactual:\n{result}"
        );
    }

    /// Regression: tagged struct variants whose source field type is already `Option<T>`
    /// must preserve that option layer. The flat wasm struct stores every variant field as
    /// `Option<T>`; wrapping an already-optional core field in `Some(...)` produces
    /// `Option<Option<T>>`, and unwrapping it in the reverse direction produces `T`.
    #[test]
    fn gen_tagged_enum_struct_variant_preserves_optional_fields() {
        let field = |name: &str, ty: TypeRef, optional: bool| FieldDef {
            name: name.to_string(),
            ty,
            optional,
            default: None,
            doc: String::new(),
            sanitized: false,
            is_boxed: false,
            type_rust_path: None,
            cfg: None,
            typed_default: None,
            core_wrapper: alef_core::ir::CoreWrapper::None,
            vec_inner_core_wrapper: alef_core::ir::CoreWrapper::None,
            newtype_wrapper: None,
            serde_rename: None,
            serde_flatten: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
        };
        let e = EnumDef {
            name: "SecuritySchemeInfo".to_string(),
            rust_path: "test_lib::SecuritySchemeInfo".to_string(),
            original_rust_path: String::new(),
            variants: vec![EnumVariant {
                name: "Http".to_string(),
                fields: vec![
                    field("scheme", TypeRef::String, false),
                    field("bearer_format", TypeRef::String, true),
                ],
                is_tuple: false,
                doc: String::new(),
                is_default: false,
                serde_rename: Some("http".to_string()),
            }],
            doc: String::new(),
            cfg: None,
            is_copy: false,
            has_serde: true,
            serde_tag: Some("type".to_string()),
            serde_untagged: false,
            serde_rename_all: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        };

        let binding_to_core = gen_tagged_enum_binding_to_core(&e, "test_lib", "Wasm");
        assert!(
            binding_to_core.contains("bearer_format: val.bearer_format.clone()"),
            "binding→core must preserve Option<String>;\nactual:\n{binding_to_core}"
        );
        assert!(
            !binding_to_core.contains("bearer_format: val.bearer_format.clone().unwrap_or_default()"),
            "binding→core must not unwrap source Option<String>;\nactual:\n{binding_to_core}"
        );

        let core_to_binding = gen_tagged_enum_core_to_binding(&e, "test_lib", "Wasm");
        assert!(
            core_to_binding.contains("bearer_format: bearer_format"),
            "core→binding must not wrap Option<String> in Some(...);\nactual:\n{core_to_binding}"
        );
        assert!(
            !core_to_binding.contains("bearer_format: Some(bearer_format)"),
            "core→binding must not create Option<Option<String>>;\nactual:\n{core_to_binding}"
        );
    }

    /// Regression: tuple-variant enums with positional `_0` fields must not emit
    /// `set__0` as the setter name — that double-underscore form is rejected by the
    /// `non_snake_case` lint under `RUSTFLAGS="-D warnings"`.  The generated Rust
    /// identifier must be `set_field_0` (getter: `field_0`) while the JS-visible
    /// name is controlled by `js_name` and remains unchanged.
    #[test]
    fn gen_tagged_enum_as_struct_positional_field_setter_snake_case() {
        use super::gen_tagged_enum_as_struct;

        let e = make_tagged_tuple_enum();
        let result = gen_tagged_enum_as_struct(&e, "Wasm");

        // The problematic setter must not appear.
        assert!(
            !result.contains("fn set__0("),
            "must not emit `set__0` — double-underscore violates non_snake_case lint;\nactual:\n{result}"
        );

        // The getter must not be named `_0` (also non-snake-case under strict lint).
        // After the fix it is `field_0`.
        assert!(
            result.contains("fn field_0("),
            "getter for positional `_0` field must be named `field_0`;\nactual:\n{result}"
        );

        // The setter must be `set_field_0`.
        assert!(
            result.contains("fn set_field_0("),
            "setter for positional `_0` field must be named `set_field_0`;\nactual:\n{result}"
        );

        // The JS-visible name attribute must still expose the camelCase-converted field name so the
        // WASM/JS API is unaffected.  `to_node_name("_0")` strips the leading underscore → `"0"`.
        assert!(
            result.contains("js_name = \"0\""),
            "js_name attribute must use the to_node_name result for `_0` field;\nactual:\n{result}"
        );

        // The struct field access inside the getter/setter body must still reference
        // `self._0` (the actual struct field identifier).
        assert!(
            result.contains("self._0"),
            "getter/setter body must access `self._0` (the struct field);\nactual:\n{result}"
        );
    }

    /// Regression test D4-WASM-A: tagged enum with unit variant emits { kind: 'bold' }
    /// as a tagged-union type alias, not a numeric enum.
    #[test]
    fn gen_tagged_enum_unit_variant_emits_tagged_union() {
        use super::gen_tagged_enum_as_struct;

        let mut e = make_tagged_tuple_enum();
        // Modify to have a unit variant and a tuple variant
        e.variants[0].fields.clear();
        e.variants[0].is_tuple = false;

        let result = gen_tagged_enum_as_struct(&e, "Wasm");

        // Must emit a #[wasm_bindgen] struct with a discriminator field ("kind" or similar).
        assert!(
            result.contains("#[wasm_bindgen]") && result.contains("pub struct Wasm"),
            "WASM tagged enum must emit wasm_bindgen struct, not numeric enum;\nactual:\n{result}"
        );

        // Must have a discriminator field named "kind" (not "role" or "annotation_type").
        // The tag field in WASM should also use "kind" for consistency with NAPI.
        assert!(
            result.contains("pub(crate)") && (result.contains("kind") || result.contains("getter")),
            "WASM tagged enum struct must have a discriminator field for the tag;\nactual:\n{result}"
        );
    }

    /// Regression test D4-WASM-B: tagged enum variant tag values use camelCase.
    /// E.g., `"fontSize"` not `"font_size"`.
    #[test]
    fn gen_tagged_enum_binding_to_core_matches_camel_case_tags() {
        use super::gen_tagged_enum_binding_to_core;

        let e = make_tagged_tuple_enum();
        let result = gen_tagged_enum_binding_to_core(&e, "test_lib", "Wasm");

        // The variant tag match arms must use camelCase or the explicit serde_rename.
        // In make_tagged_tuple_enum, System has serde_rename = Some("system"), User = Some("user").
        // These are already lowercase, but the regex pattern should respect explicit renames.
        assert!(
            result.contains("match val.") && result.contains("as_str()"),
            "binding→core must dispatch on tag field string value;\nactual:\n{result}"
        );
    }
}
