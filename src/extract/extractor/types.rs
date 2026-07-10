use crate::core::ir::{CoreWrapper, DefaultValue, EnumDef, ErrorDef, ErrorVariant, FieldDef, TypeDef};
use syn;

use super::helpers::{detect_core_wrapper, detect_vec_inner_core_wrapper, extract_binding_exclusion_reason};
use crate::extract::type_resolver;

use super::helpers::{
    build_rust_path, extract_cfg_condition, extract_doc_comments, extract_enum_variant, extract_error_message_template,
    extract_field, extract_field_binding_exclusion_reason, extract_field_type_rust_path, extract_serde_rename_all,
    extract_version_annotation, has_cfg_attribute, has_derive, has_field_attr, is_pub, syn_type_is_boxed,
    unwrap_optional,
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
fn extract_serde_tag(attrs: &[syn::Attribute]) -> Option<String> {
    for attr in attrs {
        let tokens = if let Ok(list) = attr.meta.require_list() {
            format!("{}", list.tokens)
        } else {
            continue;
        };
        if let Some(pos) = tokens.find("tag") {
            let rest = &tokens[pos..];
            let after_tag = &rest[3..];
            if !after_tag.starts_with('=') && !after_tag.trim_start().starts_with('=') {
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

/// Extract a public struct into a `TypeDef`.
/// Returns `None` for structs with type or const generic parameters — they can't be
/// directly exposed to FFI. Structs with only lifetime parameters (e.g. `Foo<'a>`) are
/// accepted; `has_lifetime_params` is set to `true` so backends can emit the appropriate
/// lifetime placeholders in `From<T<'_>>` and `T<'static>` positions.
pub(crate) fn extract_struct(item: &syn::ItemStruct, crate_name: &str, module_path: &str) -> Option<TypeDef> {
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

    let mut fields = match &item.fields {
        syn::Fields::Named(named) => named
            .named
            .iter()
            .filter(|f| is_pub(&f.vis))
            .map(|f| extract_field(f, Some(crate_name)))
            .collect(),
        syn::Fields::Unnamed(unnamed) if unnamed.unnamed.len() == 1 && is_pub(&unnamed.unnamed[0].vis) => {
            let field = &unnamed.unnamed[0];
            let resolved = type_resolver::resolve_type(&field.ty);
            let (ty, optional) = unwrap_optional(resolved);
            vec![FieldDef {
                name: "_0".to_string(),
                ty,
                optional,
                default: None,
                doc: String::new(),
                sanitized: false,
                is_boxed: syn_type_is_boxed(&field.ty),
                type_rust_path: extract_field_type_rust_path(&field.ty, Some(crate_name)),
                cfg: None,
                typed_default: None,
                core_wrapper: detect_core_wrapper(&field.ty),
                vec_inner_core_wrapper: detect_vec_inner_core_wrapper(&field.ty),
                newtype_wrapper: None,
                serde_rename: None,
                serde_flatten: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
                original_type: None,
            }]
        }
        _ => vec![],
    };

    let is_clone = has_derive(item.attrs.as_slice(), "Clone");
    let is_copy = has_derive(item.attrs.as_slice(), "Copy");
    let has_default = has_derive(item.attrs.as_slice(), "Default");
    let has_serde = has_derive(item.attrs.as_slice(), "Serialize") && has_derive(item.attrs.as_slice(), "Deserialize");
    let serde_rename_all = extract_serde_rename_all(&item.attrs);
    let doc = extract_doc_comments(&item.attrs);
    let is_opaque = fields.is_empty() && !(has_default && has_serde);
    let rust_path = build_rust_path(crate_name, module_path, &name);

    // #[derive(Default)] — all fields get DefaultValue::Empty (type's own Default)
    if has_default {
        for field in &mut fields {
            field.typed_default = Some(DefaultValue::Empty);
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
        super_traits: vec![],
        binding_excluded,
        binding_exclusion_reason,
        is_variant_wrapper: false,
        version: extract_version_annotation(&item.attrs),
        ..Default::default()
    };
    typedef.has_lifetime_params = has_lifetime_params;
    typedef.has_private_fields = has_private_fields;
    Some(typedef)
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
    let serde_untagged = has_serde_untagged(&item.attrs);
    let serde_rename_all = extract_serde_rename_all(&item.attrs);
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
        serde_untagged,
        serde_rename_all,
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
                                source = true; // #[from] implies source
                            }
                            extract_field(f, Some(crate_name))
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
                                binding_excluded,
                                binding_exclusion_reason,
                                original_type: None,
                            }
                        })
                        .collect();
                    (fields, source, from, false, true)
                }
                syn::Fields::Unit => (vec![], false, false, true, false),
            };

            ErrorVariant {
                name: v.ident.to_string(),
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

    let rust_path = build_rust_path(crate_name, module_path, &name);

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
