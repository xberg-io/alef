//! WASM enum code generation.

use crate::core::ir::{EnumDef, FieldDef, TypeRef};

use crate::backends::wasm::type_map::WasmMapper;
use crate::codegen::naming::{to_node_name, wire_variant_value};
use crate::codegen::type_mapper::TypeMapper;

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

/// Returns true when `field` was originally `Vec<(K, V)>` but was sanitized to `Vec<String>`.
///
/// These fields must be stored as `Option<JsValue>` in the wasm struct so that the JS wire
/// representation (`[[k, v], ...]`) round-trips correctly through `serde_wasm_bindgen` rather
/// than collapsing to a flat `Vec<String>`.
fn is_sanitized_tuple_vec(field: &FieldDef) -> bool {
    field.sanitized && field.original_type.as_deref().is_some_and(|s| s.starts_with("Vec<("))
}

/// Returns true when `field` was originally a fixed-size array of tuples (`[(K, V); N]`)
/// but was sanitized to `String` (JSON-encoded).
///
/// Like `is_sanitized_tuple_vec`, these fields must be stored as `Option<JsValue>` so that
/// serde round-trips the structured JS value through `serde_wasm_bindgen` rather than treating
/// the field as a plain string.
fn is_sanitized_fixed_tuple_array(field: &FieldDef) -> bool {
    field.sanitized
        && field
            .original_type
            .as_deref()
            .is_some_and(|s| s.starts_with("[(") && s.contains(");"))
}

fn tagged_enum_binding_to_core_expr(field_ident: &str, field_ty: &TypeRef, field_optional: bool) -> String {
    if field_optional {
        return match field_ty {
            TypeRef::Named(_) => format!("val.{field_ident}.clone().map(Into::into)"),
            // Path (PathBuf): String → PathBuf via Into::into
            TypeRef::Path => format!("val.{field_ident}.clone().map(Into::into)"),
            // Map fields ride as `Option<JsValue>` in the binding (map_uses_jsvalue);
            // deserialize back to the core HashMap via serde_wasm_bindgen.
            TypeRef::Map(_, _) => {
                format!("val.{field_ident}.clone().and_then(|v| serde_wasm_bindgen::from_value(v).ok())")
            }
            _ => format!("val.{field_ident}.clone()"),
        };
    }
    match field_ty {
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(_) => format!("val.{field_ident}.clone().map(Into::into)"),
            // Path (PathBuf): String → PathBuf via Into::into
            TypeRef::Path => format!("val.{field_ident}.clone().map(Into::into)"),
            TypeRef::Map(_, _) => {
                format!("val.{field_ident}.clone().and_then(|v| serde_wasm_bindgen::from_value(v).ok())")
            }
            _ => format!("val.{field_ident}.clone()"),
        },
        TypeRef::Named(_) => format!("val.{field_ident}.clone().map(Into::into).unwrap_or_default()"),
        // Vec<Named>: binding stores Vec<BindingType>, core expects Vec<CoreType> — map with .into().
        TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Named(_)) => {
            format!("val.{field_ident}.clone().unwrap_or_default().into_iter().map(Into::into).collect()")
        }
        // Path (PathBuf): String → PathBuf via Into::into
        TypeRef::Path => format!("val.{field_ident}.clone().map(Into::into).unwrap_or_default()"),
        // Non-optional Map field on a tagged-enum variant: binding holds Option<JsValue>;
        // deserialize via serde_wasm_bindgen with a Default fallback when None / parse fails.
        TypeRef::Map(_, _) => format!(
            "val.{field_ident}.clone().and_then(|v| serde_wasm_bindgen::from_value(v).ok()).unwrap_or_default()"
        ),
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
            // Path (PathBuf): PathBuf → String via to_string_lossy
            TypeRef::Path => format!("                {field_ident}: {local}.map(|p| p.to_string_lossy().to_string())"),
            // Map fields in the binding struct become `Option<JsValue>` (per
            // ConversionConfig::map_uses_jsvalue); convert the destructured HashMap into a
            // JsValue via serde_wasm_bindgen rather than passing the raw HashMap.
            TypeRef::Map(_, _) => {
                format!(
                    "                {field_ident}: {local}.as_ref().and_then(|m| serde_wasm_bindgen::to_value(m).ok())"
                )
            }
            _ => format!("                {field_ident}: {local}"),
        };
    }
    match field_ty {
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(_) => format!("                {field_ident}: {local}.map(Into::into)"),
            // Path (PathBuf): PathBuf → String via to_string_lossy
            TypeRef::Path => format!("                {field_ident}: {local}.map(|p| p.to_string_lossy().to_string())"),
            TypeRef::Map(_, _) => {
                format!(
                    "                {field_ident}: {local}.as_ref().and_then(|m| serde_wasm_bindgen::to_value(m).ok())"
                )
            }
            _ => format!("                {field_ident}: {local}"),
        },
        TypeRef::Named(_) => format!("                {field_ident}: Some({local}.into())"),
        // Vec<Named>: core has Vec<CoreType>, binding holds Option<Vec<BindingType>>. Map each
        // element via Into::into and wrap with Some.
        TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Named(_)) => {
            format!("                {field_ident}: Some({local}.into_iter().map(Into::into).collect())")
        }
        // Path (PathBuf): PathBuf → String via to_string_lossy
        TypeRef::Path => format!("                {field_ident}: Some({local}.to_string_lossy().to_string())"),
        // Non-optional Map field on a tagged-enum variant: binding holds Option<JsValue>, so
        // serialize the HashMap via serde_wasm_bindgen and wrap with Some.
        TypeRef::Map(_, _) => format!("                {field_ident}: serde_wasm_bindgen::to_value(&{local}).ok()"),
        _ => format!("                {field_ident}: Some({local})"),
    }
}

/// Compute the serde wire tag value for a variant — what JS supplies in the `type` field.
pub(super) fn variant_tag_value(
    variant_name: &str,
    serde_rename: Option<&str>,
    serde_rename_all: Option<&str>,
) -> String {
    wire_variant_value(variant_name, serde_rename, serde_rename_all)
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
    // Use the prefixed Rust struct name as the JS export name too — keeps the
    // wasm-bindgen JS API in sync with the alef-e2e codegen's imports, which
    // reference types by their prefixed Rust identifier.
    lines.push("#[wasm_bindgen]".to_string());
    lines.push("#[derive(Clone, Default)]".to_string());
    lines.push(format!("pub struct {js_name} {{"));
    lines.push(format!("    pub(crate) {tag_field_ident}: String,"));

    // Union of all variant fields. De-duplicate by name; if a name appears in multiple variants
    // with different Named types (e.g. `_0: SystemMessage` vs `_0: UserMessage`) we fall back
    // to `JsValue` (callers convert via serde_wasm_bindgen per-variant in the From impls).
    // This mirrors the NAPI `mixed_named_fields` / `serde_json` path.
    let mixed = mixed_type_fields(enum_def);
    // Collect field names that are sanitized Vec<(K, V)> or fixed [(K,V); N] — must use JsValue
    // for round-trip so the structured JS representation survives serde_wasm_bindgen.
    let tuple_vec_fields: std::collections::BTreeSet<String> = enum_def
        .variants
        .iter()
        .flat_map(|v| v.fields.iter())
        .filter(|f| is_sanitized_tuple_vec(f) || is_sanitized_fixed_tuple_array(f))
        .map(|f| f.name.clone())
        .collect();
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut field_entries: Vec<(String, String)> = Vec::new();
    for variant in &enum_def.variants {
        for field in &variant.fields {
            if !seen.insert(field.name.clone()) {
                continue;
            }
            let field_ty = if mixed.contains(&field.name) || tuple_vec_fields.contains(&field.name) {
                // Mixed-type Named field or sanitized tuple-vec/fixed-array: store as JsValue,
                // convert via serde_wasm_bindgen per-variant in the From impls.
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
/// Return the first `::` segment of a Rust path (the crate name), normalizing hyphens to
/// underscores to match how Cargo exposes crate names in Rust code.
fn path_crate_segment(path: &str) -> &str {
    path.split("::").next().unwrap_or("")
}

/// True when `rust_path` resolves to a crate other than `core_import`.
/// Such types are not in the WASM consumer's Cargo dependency graph, so emitting
/// `serde_wasm_bindgen::from_value::<{rust_path}>()` would produce E0433.
fn is_external_crate_type(rust_path: &str, core_import: &str) -> bool {
    let crate_seg = path_crate_segment(rust_path);
    // Normalise hyphens → underscores (Cargo crate name vs Rust module name).
    let core_seg = core_import.replace('-', "_");
    crate_seg.replace('-', "_") != core_seg
}

pub(super) fn gen_tagged_enum_binding_to_core(enum_def: &EnumDef, core_import: &str, prefix: &str) -> String {
    let core_path = crate::codegen::conversions::core_enum_path(enum_def, core_import);
    let binding_name = format!("{prefix}{}", enum_def.name);
    let tag_field = enum_def.serde_tag.as_deref().unwrap_or("type");
    let tag_field_ident = escape_rust_keyword(tag_field);
    let mixed = mixed_type_fields(enum_def);
    let tuple_vec_fields: std::collections::BTreeSet<String> = enum_def
        .variants
        .iter()
        .flat_map(|v| v.fields.iter())
        .filter(|f| is_sanitized_tuple_vec(f) || is_sanitized_fixed_tuple_array(f))
        .map(|f| f.name.clone())
        .collect();

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
        // Add cfg guard if variant is behind a feature gate
        if let Some(cfg) = variant.cfg.as_deref() {
            lines.push(format!("            #[cfg({})]", cfg));
        }
        if variant.fields.is_empty() {
            lines.push(format!("            \"{tag_value}\" => Self::{},", variant.name));
        } else if variant.is_tuple {
            // Tuple/newtype variant: construct positionally.
            // For mixed-type Named fields or sanitized tuple-vec fields the binding struct
            // stores `JsValue`; deserialize via serde_wasm_bindgen to the variant-specific
            // core type. For single-type Named fields the binding struct stores the binding
            // wrapper; call `.into()`.
            let args: Vec<String> = variant
                .fields
                .iter()
                .map(|f| {
                    let f_ident = escape_rust_keyword(&f.name);
                    if mixed.contains(&f.name) {
                        // Field is stored as JsValue in the binding struct.
                        // When the field type is from an external crate (not `core_import`),
                        // emitting `serde_wasm_bindgen::from_value::<ext_crate::T>()` would
                        // fail with E0433 because the external crate is not a dep of the WASM
                        // consumer. Fall back to Default::default() for such fields — the value
                        // cannot be reconstructed without the dep anyway.
                        let external = f
                            .type_rust_path
                            .as_deref()
                            .is_some_and(|p| is_external_crate_type(p, core_import));
                        if external {
                            let expr = "Default::default()".to_string();
                            if f.is_boxed { format!("Box::new({expr})") } else { expr }
                        } else {
                            // Prefer the full type_rust_path when available so that types
                            // are reconstructed via the correct fully-qualified path rather
                            // than `{core_import}::{short_name}`.
                            let core_inner = if let Some(ref path) = f.type_rust_path {
                                path.replace('-', "_")
                            } else {
                                match &f.ty {
                                    TypeRef::Named(n) => format!("{core_import}::{n}"),
                                    _ => "serde_json::Value".to_string(),
                                }
                            };
                            let expr = format!(
                                "val.{f_ident}.as_ref().and_then(|v| serde_wasm_bindgen::from_value::<{core_inner}>(v.clone()).ok()).unwrap_or_default()"
                            );
                            // Box<T> variants: the reconstructed value must be heap-allocated.
                            if f.is_boxed { format!("Box::new({expr})") } else { expr }
                        }
                    } else if tuple_vec_fields.contains(&f.name) {
                        // Sanitized Vec<(K, V)> or fixed-tuple-array stored as JsValue — decode via serde.
                        let orig = f.original_type.as_deref().unwrap_or("Vec<(String, String)>");
                        format!(
                            "val.{f_ident}.as_ref().and_then(|v| serde_wasm_bindgen::from_value::<{orig}>(v.clone()).ok()).unwrap_or_default()"
                        )
                    } else {
                        // Single-type Named or primitive field. Wrap in Box::new() when the
                        // core variant holds Box<T> (is_boxed on the field definition).
                        let expr = tagged_enum_binding_to_core_expr(&f_ident, &f.ty, f.optional);
                        if f.is_boxed { format!("Box::new({expr})") } else { expr }
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
                    if tuple_vec_fields.contains(&f.name) {
                        // Sanitized Vec<(K, V)> stored as JsValue — decode via serde.
                        let orig = f.original_type.as_deref().unwrap_or("Vec<(String, String)>");
                        format!(
                            "{}: val.{f_ident}.as_ref().and_then(|v| serde_wasm_bindgen::from_value::<{orig}>(v.clone()).ok()).unwrap_or_default()",
                            f.name
                        )
                    } else {
                        format!(
                            "{}: {}",
                            f.name,
                            tagged_enum_binding_to_core_expr(&f_ident, &f.ty, f.optional)
                        )
                    }
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
    let core_path = crate::codegen::conversions::core_enum_path(enum_def, core_import);
    let binding_name = format!("{prefix}{}", enum_def.name);
    let tag_field = enum_def.serde_tag.as_deref().unwrap_or("type");
    let tag_field_ident = escape_rust_keyword(tag_field);
    let mixed = mixed_type_fields(enum_def);
    let tuple_vec_fields: std::collections::BTreeSet<String> = enum_def
        .variants
        .iter()
        .flat_map(|v| v.fields.iter())
        .filter(|f| is_sanitized_tuple_vec(f) || is_sanitized_fixed_tuple_array(f))
        .map(|f| f.name.clone())
        .collect();

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
        // Add cfg guard if variant is behind a feature gate
        if let Some(cfg) = variant.cfg.as_deref() {
            lines.push(format!("            #[cfg({})]", cfg));
        }
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
                    } else if tuple_vec_fields.contains(name) {
                        // Sanitized Vec<(K, V)>: serialize to JsValue via serde.
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
                    let init = if tuple_vec_fields.contains(name) {
                        // Sanitized Vec<(K, V)>: serialize to JsValue so JS sees [[k,v],...].
                        format!("                {n_ident}: serde_wasm_bindgen::to_value(&{n_ident}).ok()")
                    } else if let Some(field) = variant.fields.iter().find(|f| &f.name == name) {
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
    // Wildcard arm to handle feature-gated variants excluded from alef IR
    // (e.g. variants marked `#[doc(hidden)]` or `#[alef(skip)]`).
    // Without this, adding a cfg-gated variant to the source enum causes a
    // non-exhaustive compile error in generated `From` impls.
    lines.push(format!(
        "            _ => ::std::panic!(\"unmapped {} variant\"),",
        enum_def.name
    ));
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
    // Use the prefixed Rust enum name as the JS export name too — keeps the
    // wasm-bindgen JS API in sync with the alef-e2e codegen's imports.
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
            let wire = wire_variant_value(
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
            let wire = wire_variant_value(
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
mod tests;
