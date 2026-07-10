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
            TypeRef::Path => format!("val.{field_ident}.clone().map(Into::into)"),
            TypeRef::Map(_, _) => {
                format!("val.{field_ident}.clone().and_then(|v| serde_wasm_bindgen::from_value(v).ok())")
            }
            _ => format!("val.{field_ident}.clone()"),
        };
    }
    match field_ty {
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(_) => format!("val.{field_ident}.clone().map(Into::into)"),
            TypeRef::Path => format!("val.{field_ident}.clone().map(Into::into)"),
            TypeRef::Map(_, _) => {
                format!("val.{field_ident}.clone().and_then(|v| serde_wasm_bindgen::from_value(v).ok())")
            }
            _ => format!("val.{field_ident}.clone()"),
        },
        TypeRef::Named(_) => format!("val.{field_ident}.clone().map(Into::into).unwrap_or_default()"),
        TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Named(_)) => {
            format!("val.{field_ident}.clone().unwrap_or_default().into_iter().map(Into::into).collect()")
        }
        TypeRef::Path => format!("val.{field_ident}.clone().map(Into::into).unwrap_or_default()"),
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
            TypeRef::Path => format!("                {field_ident}: {local}.map(|p| p.to_string_lossy().to_string())"),
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
            TypeRef::Path => format!("                {field_ident}: {local}.map(|p| p.to_string_lossy().to_string())"),
            TypeRef::Map(_, _) => {
                format!(
                    "                {field_ident}: {local}.as_ref().and_then(|m| serde_wasm_bindgen::to_value(m).ok())"
                )
            }
            _ => format!("                {field_ident}: {local}"),
        },
        TypeRef::Named(_) => format!("                {field_ident}: Some({local}.into())"),
        TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Named(_)) => {
            format!("                {field_ident}: Some({local}.into_iter().map(Into::into).collect())")
        }
        TypeRef::Path => format!("                {field_ident}: Some({local}.to_string_lossy().to_string())"),
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
    let tag_field_ident = escape_rust_keyword(tag_field);
    let tag_js_name = to_node_name(tag_field);
    let mapper = WasmMapper::new(std::collections::HashMap::new(), prefix.to_string());

    let mut lines = vec![];
    let doc = emit_rustdoc(&enum_def.doc);
    if !doc.is_empty() {
        lines.push(doc);
    }

    lines.push("#[wasm_bindgen]".to_string());
    lines.push("#[derive(Clone, Default)]".to_string());
    lines.push(format!("pub struct {js_name} {{"));
    lines.push(format!("    pub(crate) {tag_field_ident}: String,"));

    let mixed = mixed_type_fields(enum_def);
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

    for (name, ty) in &field_entries {
        let js_name_for_field = to_node_name(name);
        let field_name = name.as_str();
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
        if let Some(cfg) = variant.cfg.as_deref() {
            lines.push(format!("            #[cfg({})]", cfg));
        }
        if variant.fields.is_empty() {
            lines.push(format!("            \"{tag_value}\" => Self::{},", variant.name));
        } else if variant.is_tuple {
            let args: Vec<String> = variant
                .fields
                .iter()
                .map(|f| {
                    let f_ident = escape_rust_keyword(&f.name);
                    if mixed.contains(&f.name) {
                        let external = f
                            .type_rust_path
                            .as_deref()
                            .is_some_and(|p| is_external_crate_type(p, core_import));
                        if external {
                            let expr = "Default::default()".to_string();
                            if f.is_boxed { format!("Box::new({expr})") } else { expr }
                        } else {
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
                            if f.is_boxed { format!("Box::new({expr})") } else { expr }
                        }
                    } else if tuple_vec_fields.contains(&f.name) {
                        let orig = f.original_type.as_deref().unwrap_or("Vec<(String, String)>");
                        format!(
                            "val.{f_ident}.as_ref().and_then(|v| serde_wasm_bindgen::from_value::<{orig}>(v.clone()).ok()).unwrap_or_default()"
                        )
                    } else {
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
            let inits: Vec<String> = variant
                .fields
                .iter()
                .map(|f| {
                    let f_ident = escape_rust_keyword(&f.name);
                    if tuple_vec_fields.contains(&f.name) {
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
                    let pos = variant.fields.iter().position(|f| &f.name == name).unwrap();
                    let local = &local_names[pos];
                    let init = if mixed.contains(name) {
                        format!("                {n_ident}: serde_wasm_bindgen::to_value(&{local}).ok()")
                    } else if tuple_vec_fields.contains(name) {
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
            let destructure_names: Vec<String> = variant.fields.iter().map(|f| escape_rust_keyword(&f.name)).collect();
            let mut inits = vec![format!(
                "                {tag_field_ident}: \"{tag_value}\".to_string()"
            )];
            for name in &all_field_names {
                let n_ident = escape_rust_keyword(name);
                if variant_field_names.contains(name) {
                    let init = if tuple_vec_fields.contains(name) {
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
    // (e.g. variants marked `#[doc(hidden)]` or `#[alef(skip)]`).
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
    lines.extend([
        "#[wasm_bindgen]".to_string(),
        "#[derive(Clone, Copy, PartialEq, Eq)]".to_string(),
        format!("pub enum {} {{", js_name),
    ]);

    for (idx, variant) in enum_def.variants.iter().enumerate() {
        lines.push(format!("    {} = {},", variant.name, idx));
    }

    lines.push("}".to_string());

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
