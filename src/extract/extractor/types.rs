use crate::core::ir::{CoreWrapper, DefaultValue, EnumDef, ErrorDef, ErrorVariant, FieldDef, TypeDef};
use ahash::AHashMap;
use syn;

use super::helpers::extract_binding_exclusion_reason;
use crate::extract::type_resolver;

use super::helpers::{
    build_rust_path, extract_alef_error_code, extract_cfg_condition, extract_doc_comments, extract_enum_variant,
    extract_error_message_template, extract_field, extract_field_binding_exclusion_reason,
    extract_field_type_rust_path, extract_serde_container_conversion, extract_serde_rename_all,
    extract_serde_rename_all_fields, extract_serde_skip, extract_serde_skip_serializing_if, extract_version_annotation,
    has_cfg_attribute, has_container_serde_default, has_derive, has_field_attr, is_pub, syn_type_is_boxed,
};

/// Return true when the enum has `#[serde(untagged)]`.
fn has_serde_untagged(attrs: &[syn::Attribute]) -> bool {
    for attr in attrs {
        let tokens = if let Ok(list) = attr.meta.require_list() {
            format!("{}", list.tokens)
        } else {
            continue;
        };
        let mut rest = tokens.as_str();
        while let Some(pos) = rest.find("untagged") {
            let before = &rest[..pos];
            let after = &rest[pos + "untagged".len()..];
            let valid_before = before.is_empty() || before.ends_with(|c: char| !c.is_alphanumeric() && c != '_');
            let valid_after = after.is_empty() || after.starts_with(|c: char| !c.is_alphanumeric() && c != '_');
            let not_kv = !after.trim_start().starts_with('=');
            if valid_before && valid_after && not_kv {
                return true;
            }
            rest = &rest[pos + 1..];
        }
    }
    false
}

/// Extract `tag` value from `#[serde(tag = "...")]` or
/// `#[cfg_attr(..., serde(tag = "..."))]` attributes on enums.
fn extract_serde_name_value(attrs: &[syn::Attribute], key: &str) -> Option<String> {
    for attr in attrs {
        let tokens = if let Ok(list) = attr.meta.require_list() {
            format!("{}", list.tokens)
        } else {
            continue;
        };
        if let Some(pos) = tokens.find(key) {
            let rest = &tokens[pos..];
            let after_key = &rest[key.len()..];
            if !after_key.starts_with('=') && !after_key.trim_start().starts_with('=') {
                continue;
            }
            if let Some(eq_pos) = rest.find('=') {
                let after_eq = rest[eq_pos + 1..].trim_start();
                if let Some(start) = after_eq.find('"') {
                    let after_start = &after_eq[start + 1..];
                    if let Some(end) = after_start.find('"') {
                        return Some(after_start[..end].to_string());
                    }
                }
            }
        }
    }
    None
}

fn extract_serde_tag(attrs: &[syn::Attribute]) -> Option<String> {
    extract_serde_name_value(attrs, "tag")
}

fn extract_serde_content(attrs: &[syn::Attribute]) -> Option<String> {
    extract_serde_name_value(attrs, "content")
}

/// Extract a public struct into a `TypeDef`.
/// Returns `None` for structs with type or const generic parameters — they can't be
/// directly exposed to FFI. Structs with only lifetime parameters (e.g. `Foo<'a>`) are
/// accepted; `has_lifetime_params` is set to `true` so backends can emit the appropriate
/// lifetime placeholders in `From<T<'_>>` and `T<'static>` positions. ~keep
/// Returns the extracted `TypeDef` alongside the serde reader's per-field defaults (field name →
/// value), for the caller to thread on to `functions::impl_blocks` so a later manual
/// `impl Default` can still be compared against them — see `serde_defaults` above. ~keep
pub(crate) fn extract_struct(
    item: &syn::ItemStruct,
    crate_name: &str,
    module_path: &str,
) -> Option<(TypeDef, AHashMap<String, DefaultValue>)> {
    let has_non_lifetime = item
        .generics
        .params
        .iter()
        .any(|p| !matches!(p, syn::GenericParam::Lifetime(_)));
    if has_non_lifetime {
        return None;
    }
    let has_lifetime_params = !item.generics.params.is_empty();
    let binding_exclusion_reason = extract_binding_exclusion_reason(&item.attrs);
    let binding_excluded = binding_exclusion_reason.is_some();
    let cfg = extract_cfg_condition(&item.attrs);
    let name = item.ident.to_string();

    let has_private_fields = match &item.fields {
        syn::Fields::Named(named) => named.named.iter().any(|f| !is_pub(&f.vis)),
        _ => false,
    };

    let extracted_fields: Vec<(FieldDef, Option<DefaultValue>)> = match &item.fields {
        syn::Fields::Named(named) => named
            .named
            .iter()
            .filter(|f| is_pub(&f.vis))
            .map(|f| extract_field(f, Some(crate_name)))
            .collect(),
        syn::Fields::Unnamed(unnamed) if unnamed.unnamed.len() == 1 && is_pub(&unnamed.unnamed[0].vis) => {
            let field = &unnamed.unnamed[0];
            let (mut extracted, serde_default) = extract_field(field, Some(crate_name));
            extracted.name = "_0".to_string();
            vec![(extracted, serde_default)]
        }
        _ => vec![],
    };

    // The serde reader's own default for each field, captured here because
    // `#[derive(Default)]` below — or a manual `impl Default`, folded later by
    // `functions::impl_blocks` — unconditionally overwrites `FieldDef::typed_default` and would
    // otherwise erase it. Compared against the final value by a single deferred pass,
    // `postprocess::warn_on_default_disagreements`, run once the whole crate has been extracted
    // (see `extract::extractor::mod::extract`) via the `pending_serde_defaults` map threaded
    // through `extract::extractor::mod::extract_items`. ~keep
    let serde_defaults: AHashMap<String, DefaultValue> = extracted_fields
        .iter()
        .filter_map(|(field, serde_default)| serde_default.clone().map(|value| (field.name.clone(), value)))
        .collect();
    let mut fields: Vec<FieldDef> = extracted_fields.into_iter().map(|(field, _)| field).collect();

    let is_clone = has_derive(item.attrs.as_slice(), "Clone");
    let is_copy = has_derive(item.attrs.as_slice(), "Copy");
    let has_default = has_derive(item.attrs.as_slice(), "Default");
    let has_serde = has_derive(item.attrs.as_slice(), "Serialize") && has_derive(item.attrs.as_slice(), "Deserialize");
    let serde_container_default = has_container_serde_default(&item.attrs);
    let serde_container_conversion = extract_serde_container_conversion(&item.attrs);
    let serde_rename_all = extract_serde_rename_all(&item.attrs);
    let doc = extract_doc_comments(&item.attrs);
    let is_opaque = fields.is_empty() && !(has_default && has_serde);
    let rust_path = build_rust_path(crate_name, module_path, &name);

    // `#[derive(Default)]` is the one case where `DefaultValue::Empty` is an *assertion* rather
    // than a fallback: the derived impl gives every field its type's zero, so a backend
    // substituting its own zero is exact. A manual `impl Default` is read instead by
    // `extract::extractor::defaults`, which writes `DefaultValue::Unresolved` when it cannot
    // recover the real values — the distinction this seeding must not be confused with. Note that
    // `has_default` itself does *not* carry it: `functions::impl_blocks` sets the same flag for a
    // manual impl.
    //
    // That assertion is scoped to fields the derived impl actually fills, which is why this is a
    // *precedence* rule and not an assignment. A field carrying `#[serde(default = "path")]` is
    // filled by `path()` when its wire key is absent — `Default::default()` is never consulted —
    // so its value is whatever that function returns and emphatically not the type's zero.
    // Blanket-overwriting the `FunctionCall` that `helpers::fields::extract_field` recorded
    // downgraded "alef does not know this value" to "alef knows it is empty", and every backend
    // that keys its refusal on `FunctionCall` (`backends::kotlin`'s `kotlin_field_default`,
    // `backends::swift`'s `emit_decoder_init`, `backends::csharp`'s `gen_record_type`) then
    // fabricated `emptyList()`/`[]`/`null` over a populated allow-list or deny-list. `Empty` is
    // therefore seeded only where nothing stronger was already recorded; a bare
    // `#[serde(default)]` records no `typed_default` at all and still lands on `Empty`, which is
    // correct because for it `Default::default()` genuinely is the value. ~keep
    if has_default {
        for field in &mut fields {
            if field.typed_default.is_none() {
                field.typed_default = Some(DefaultValue::Empty);
            }
        }
    }

    let has_stripped_cfg_fields = fields.iter().any(|f| f.cfg.is_some());

    let mut typedef = TypeDef {
        rust_path,
        original_rust_path: String::new(),
        name,
        fields,
        methods: vec![],
        is_opaque,
        is_clone,
        is_copy,
        is_trait: false,
        has_default,
        has_stripped_cfg_fields,
        is_return_type: false,
        doc,
        cfg,
        serde_rename_all,
        has_serde,
        serde_container_default,
        serde_container_conversion,
        super_traits: vec![],
        binding_excluded,
        binding_exclusion_reason,
        is_variant_wrapper: false,
        version: extract_version_annotation(&item.attrs),
        ..Default::default()
    };
    typedef.has_lifetime_params = has_lifetime_params;
    typedef.has_private_fields = has_private_fields;
    Some((typedef, serde_defaults))
}

/// Extract a public enum into an `EnumDef`.
/// Returns `None` for generic enums — they can't be directly exposed to FFI.
pub(crate) fn extract_enum(item: &syn::ItemEnum, crate_name: &str, module_path: &str) -> Option<EnumDef> {
    if !item.generics.params.is_empty() {
        return None;
    }
    let binding_exclusion_reason = extract_binding_exclusion_reason(&item.attrs);
    let binding_excluded = binding_exclusion_reason.is_some();
    let cfg = extract_cfg_condition(&item.attrs);
    let name = item.ident.to_string();
    let doc = extract_doc_comments(&item.attrs);

    let all_variants: Vec<_> = item.variants.iter().map(extract_enum_variant).collect();
    let (excluded_variants, variants): (Vec<_>, Vec<_>) = all_variants.into_iter().partition(|v| v.binding_excluded);

    let rust_path = build_rust_path(crate_name, module_path, &name);
    let serde_tag = extract_serde_tag(&item.attrs);
    let serde_content = extract_serde_content(&item.attrs);
    let serde_untagged = has_serde_untagged(&item.attrs);
    let serde_rename_all = extract_serde_rename_all(&item.attrs);
    let rename_all_fields = extract_serde_rename_all_fields(&item.attrs);
    let is_copy = has_derive(item.attrs.as_slice(), "Copy");
    let has_serde = has_derive(item.attrs.as_slice(), "Serialize") && has_derive(item.attrs.as_slice(), "Deserialize");
    let has_default = has_derive(item.attrs.as_slice(), "Default");

    Some(EnumDef {
        rust_path,
        original_rust_path: String::new(),
        name,
        variants,
        methods: vec![],
        excluded_variants,
        doc,
        cfg,
        serde_tag,
        serde_content,
        serde_untagged,
        serde_rename_all,
        rename_all_fields,
        is_copy,
        has_serde,
        has_default,
        binding_excluded,
        binding_exclusion_reason,
        version: extract_version_annotation(&item.attrs),
    })
}

/// Extract a `#[derive(thiserror::Error)]` enum into an `ErrorDef`.
/// Returns `None` for generic enums.
pub(crate) fn extract_error_enum(item: &syn::ItemEnum, crate_name: &str, module_path: &str) -> Option<ErrorDef> {
    if !item.generics.params.is_empty() {
        return None;
    }
    let binding_exclusion_reason = extract_binding_exclusion_reason(&item.attrs);
    let binding_excluded = binding_exclusion_reason.is_some();
    let name = item.ident.to_string();
    let doc = extract_doc_comments(&item.attrs);
    let rust_path = build_rust_path(crate_name, module_path, &name);

    let variants = item
        .variants
        .iter()
        .filter(|v| !has_cfg_attribute(&v.attrs))
        .map(|v| {
            let message_template = extract_error_message_template(&v.attrs);
            let variant_doc = extract_doc_comments(&v.attrs);

            let (fields, has_source, has_from, is_unit, is_tuple) = match &v.fields {
                syn::Fields::Named(named) => {
                    let mut source = false;
                    let mut from = false;
                    let fields: Vec<FieldDef> = named
                        .named
                        .iter()
                        .map(|f| {
                            if has_field_attr(&f.attrs, "source") {
                                source = true;
                            }
                            if has_field_attr(&f.attrs, "from") {
                                from = true;
                                source = true; // #[from] implies source ~keep
                            }
                            // Error-enum fields carry no `impl Default` of their own; the serde
                            // default half of `extract_field`'s return is discarded. ~keep
                            extract_field(f, Some(crate_name)).0
                        })
                        .collect();
                    (fields, source, from, false, false)
                }
                syn::Fields::Unnamed(unnamed) => {
                    let mut source = false;
                    let mut from = false;
                    let fields: Vec<FieldDef> = unnamed
                        .unnamed
                        .iter()
                        .enumerate()
                        .map(|(i, f)| {
                            if has_field_attr(&f.attrs, "source") {
                                source = true;
                            }
                            if has_field_attr(&f.attrs, "from") {
                                from = true;
                                source = true;
                            }
                            let ty = type_resolver::resolve_type(&f.ty);
                            let optional = type_resolver::is_option_type(&f.ty).is_some();
                            let binding_exclusion_reason = extract_field_binding_exclusion_reason(&f.attrs, &f.ty);
                            let binding_excluded = binding_exclusion_reason.is_some();
                            FieldDef {
                                name: format!("_{i}"),
                                ty,
                                optional,
                                default: None,
                                doc: extract_doc_comments(&f.attrs),
                                sanitized: false,
                                is_boxed: syn_type_is_boxed(&f.ty),
                                type_rust_path: extract_field_type_rust_path(&f.ty, Some(crate_name)),
                                cfg: None,
                                typed_default: None,
                                core_wrapper: CoreWrapper::None,
                                vec_inner_core_wrapper: CoreWrapper::None,
                                newtype_wrapper: None,
                                serde_rename: None,
                                serde_flatten: false,
                                serde_with: None,
                                serde_skip_serializing_if: extract_serde_skip_serializing_if(&f.attrs),
                                serde_skip: extract_serde_skip(&f.attrs),
                                binding_excluded,
                                binding_exclusion_reason,
                                original_type: None,
                                version: extract_version_annotation(&f.attrs),
                            }
                        })
                        .collect();
                    (fields, source, from, false, true)
                }
                syn::Fields::Unit => (vec![], false, false, true, false),
            };

            ErrorVariant {
                name: v.ident.to_string(),
                error_code: extract_alef_error_code(&v.attrs),
                message_template,
                fields,
                has_source,
                has_from,
                is_unit,
                is_tuple,
                doc: variant_doc,
            }
        })
        .collect();

    Some(ErrorDef {
        name,
        rust_path,
        original_rust_path: String::new(),
        variants,
        doc,
        methods: Vec::new(),
        binding_excluded,
        binding_exclusion_reason,
        version: extract_version_annotation(&item.attrs),
    })
}
