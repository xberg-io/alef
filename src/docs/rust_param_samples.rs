use crate::core::config::Language;
use crate::core::ir::{ApiSurface, ParamDef, TypeRef};

pub(crate) fn rust_original_param_sample(param: &ParamDef, lang: Language, api: &ApiSurface) -> Option<String> {
    if lang != Language::Rust {
        return None;
    }
    let original = param.original_type.as_deref().or(match &param.ty {
        TypeRef::Named(name) => Some(name.as_str()),
        _ => None,
    })?;
    let short_name = original.rsplit("::").next().unwrap_or(original);

    if let Some(enum_def) = api.enums.iter().find(|enum_def| enum_def.rust_path == original) {
        return Some(enum_sample(enum_def, original));
    }
    if let Some(type_def) = api.types.iter().find(|type_def| type_def.rust_path == original) {
        return Some(default_or_placeholder(original, type_def.has_default));
    }
    let live_enums: Vec<_> = api
        .enums
        .iter()
        .filter(|type_def| type_def.name == short_name)
        .collect();
    let live_types: Vec<_> = api
        .types
        .iter()
        .filter(|type_def| type_def.name == short_name)
        .collect();
    if live_enums.len() + live_types.len() != 1 {
        return Some("todo!()".to_string());
    }
    if let Some(enum_def) = live_enums.first() {
        return Some(enum_sample(enum_def, original));
    }
    if let Some(type_def) = live_types.first() {
        return Some(default_or_placeholder(original, type_def.has_default));
    }
    Some("todo!()".to_string())
}

fn enum_sample(enum_def: &crate::core::ir::EnumDef, original: &str) -> String {
    let variant = enum_def
        .variants
        .iter()
        .find(|variant| variant.is_default && variant.fields.is_empty())
        .or_else(|| enum_def.variants.iter().find(|variant| variant.fields.is_empty()));
    match variant {
        Some(variant) => format!("{original}::{}", variant.name),
        None => default_or_placeholder(original, enum_def.has_default),
    }
}

fn default_or_placeholder(original: &str, has_default: bool) -> String {
    if has_default {
        format!("{original}::default()")
    } else {
        "todo!()".to_string()
    }
}
