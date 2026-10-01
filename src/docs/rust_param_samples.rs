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

    if let Some(enum_def) = uniquely_qualified_match(&api.enums, original, &api.crate_name, |item| &item.rust_path) {
        return Some(enum_sample(enum_def, original));
    }
    if let Some(type_def) = uniquely_qualified_match(&api.types, original, &api.crate_name, |item| &item.rust_path) {
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

fn uniquely_qualified_match<'a, T>(
    items: &'a [T],
    original: &str,
    crate_name: &str,
    rust_path: impl Fn(&T) -> &String,
) -> Option<&'a T> {
    let normalized_crate = crate_name.replace('-', "_");
    let crate_relative = original.strip_prefix("crate::").unwrap_or(original);
    let mut matches = items.iter().filter(|item| {
        let path = rust_path(item).replace('-', "_");
        path == original
            || path == format!("{normalized_crate}::{crate_relative}")
            || (original.contains("::") && path.strip_prefix(&format!("{normalized_crate}::")) == Some(original))
    });
    let found = matches.next()?;
    matches.next().is_none().then_some(found)
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
