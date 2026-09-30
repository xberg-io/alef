use crate::core::config::Language;
use crate::core::ir::{ApiSurface, ParamDef, RustDocType};

pub(crate) fn rust_original_param_sample(
    param: &ParamDef,
    lang: Language,
    ffi_prefix: &str,
    api: &ApiSurface,
) -> Option<String> {
    if lang != Language::Rust {
        return None;
    }
    let original = param.original_type.as_deref()?;
    let short_name = original.rsplit("::").next().unwrap_or(original);
    let retained_types: Vec<_> = api.retained_rust_doc_types().collect();

    if let Some(enum_def) = api.enums.iter().find(|enum_def| enum_def.rust_path == original) {
        return Some(live_enum_sample(enum_def, original, lang, ffi_prefix));
    }
    if let Some(type_def) = api.types.iter().find(|type_def| type_def.rust_path == original) {
        return Some(default_or_placeholder(original, type_def.has_default));
    }
    if let Some(type_def) = retained_types.iter().find(|type_def| type_def.rust_path == original) {
        return Some(retained_type_sample(original, type_def, lang, ffi_prefix));
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
    let retained: Vec<_> = retained_types
        .iter()
        .filter(|type_def| type_def.name == short_name)
        .collect();
    if live_enums.len() + live_types.len() + retained.len() != 1 {
        return Some("todo!()".to_string());
    }
    if let Some(enum_def) = live_enums.first() {
        return Some(live_enum_sample(enum_def, original, lang, ffi_prefix));
    }
    if let Some(type_def) = live_types.first() {
        return Some(default_or_placeholder(original, type_def.has_default));
    }
    retained
        .first()
        .map(|type_def| retained_type_sample(original, type_def, lang, ffi_prefix))
        .or_else(|| Some("todo!()".to_string()))
}

fn live_enum_sample(enum_def: &crate::core::ir::EnumDef, original: &str, lang: Language, ffi_prefix: &str) -> String {
    enum_sample(
        original,
        enum_def.has_default,
        enum_def
            .variants
            .iter()
            .find(|variant| variant.is_default && variant.fields.is_empty())
            .map(|variant| variant.name.as_str()),
        enum_def
            .variants
            .iter()
            .filter(|variant| variant.fields.is_empty())
            .map(|variant| variant.name.as_str()),
        lang,
        ffi_prefix,
    )
}

fn retained_type_sample(original: &str, type_def: &RustDocType, lang: Language, ffi_prefix: &str) -> String {
    enum_sample(
        original,
        type_def.has_default,
        type_def.default_unit_variant.as_deref(),
        type_def.unit_variants.iter().map(String::as_str),
        lang,
        ffi_prefix,
    )
}

fn enum_sample<'a>(
    original: &str,
    has_default: bool,
    default_unit_variant: Option<&str>,
    unit_variants: impl Iterator<Item = &'a str>,
    lang: Language,
    ffi_prefix: &str,
) -> String {
    let variant = default_unit_variant.or_else(|| unit_variants.into_iter().next());
    match variant {
        Some(variant) => crate::docs::enum_variant_ref::format_enum_variant_ref(original, variant, lang, ffi_prefix),
        None => default_or_placeholder(original, has_default),
    }
}

fn default_or_placeholder(original: &str, has_default: bool) -> String {
    if has_default {
        format!("{original}::default()")
    } else {
        "todo!()".to_string()
    }
}
