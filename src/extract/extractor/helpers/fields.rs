use crate::core::ir::{DefaultValue, FieldDef};

use crate::extract::type_resolver;

use super::attributes::{
    extract_cfg_condition, extract_field_binding_exclusion_reason, extract_serde_default_path, extract_serde_flatten,
    extract_serde_rename, extract_serde_skip, extract_serde_skip_serializing_if, extract_serde_with,
    extract_version_annotation, has_serde_default,
};
use super::field_types::{
    detect_core_wrapper, detect_vec_inner_core_wrapper, extract_field_type_rust_path, syn_type_is_boxed,
    unwrap_optional,
};
use super::rustdoc::extract_doc_comments;

/// Extract a struct field into a `FieldDef`, alongside the serde reader's own idea of the
/// field's default.
///
/// The second element is the serde reader's default, kept separate from
/// `FieldDef::typed_default` because a later `#[derive(Default)]` or manual `impl Default`
/// write unconditionally overwrites that field (see `extract::extractor::types::extract_struct`
/// and `extract::extractor::defaults::extract_default_values`) — nothing on `FieldDef` itself
/// survives to be compared against. Callers that need the comparison (struct field extraction)
/// keep this value out-of-band; callers that don't (enum variant fields, which carry no
/// `impl Default` of their own) simply discard it. `#[serde(default = "path")]` becomes
/// `FunctionCall(path)`; bare `#[serde(default)]` becomes `Empty`, the same "known type-zero"
/// `#[derive(Default)]` asserts for every field elsewhere. ~keep
///
/// When `crate_name` is provided, `crate::` prefixes in field type paths are resolved
/// to the crate name, enabling disambiguation of types with the same short name.
pub(crate) fn extract_field(field: &syn::Field, crate_name: Option<&str>) -> (FieldDef, Option<DefaultValue>) {
    let name = field.ident.as_ref().map(|i| i.to_string()).unwrap_or_default();
    let doc = extract_doc_comments(&field.attrs);
    let cfg = extract_cfg_condition(&field.attrs);
    let binding_exclusion_reason = extract_field_binding_exclusion_reason(&field.attrs, &field.ty);
    let binding_excluded = binding_exclusion_reason.is_some();

    let is_boxed = syn_type_is_boxed(&field.ty);
    let type_rust_path = extract_field_type_rust_path(&field.ty, crate_name);
    let core_wrapper = detect_core_wrapper(&field.ty);
    let vec_inner_core_wrapper = detect_vec_inner_core_wrapper(&field.ty);

    let resolved = type_resolver::resolve_type(&field.ty);
    let (ty, optional) = unwrap_optional(resolved);

    let serde_rename = extract_serde_rename(&field.attrs);
    let serde_flatten = extract_serde_flatten(&field.attrs);
    let serde_with = extract_serde_with(&field.attrs);
    let serde_skip_serializing_if = extract_serde_skip_serializing_if(&field.attrs);
    let serde_skip = extract_serde_skip(&field.attrs);
    let has_serde_default_attr = has_serde_default(&field.attrs);
    let version = extract_version_annotation(&field.attrs);

    // If the field has #[serde(default)], mark it as having a default value.
    // (`#[serde(default = "path")]`), preserve it verbatim so bindings can emit an
    let serde_default_path = extract_serde_default_path(&field.attrs);
    let default = match serde_default_path.as_deref() {
        Some(path) => Some(format!("serde(default = \"{path}\")")),
        None if has_serde_default_attr => Some("/* serde(default) */".to_string()),
        None => None,
    };

    let typed_default = serde_default_path.map(DefaultValue::FunctionCall);
    let serde_typed_default = match &typed_default {
        Some(value) => Some(value.clone()),
        None if has_serde_default_attr => Some(DefaultValue::Empty),
        None => None,
    };

    (
        FieldDef {
            name,
            ty,
            optional,
            default,
            doc,
            sanitized: false,
            is_boxed,
            type_rust_path,
            cfg,
            typed_default,
            core_wrapper,
            vec_inner_core_wrapper,
            newtype_wrapper: None,
            serde_rename,
            serde_flatten,
            serde_with,
            serde_skip_serializing_if,
            serde_skip,
            binding_excluded,
            binding_exclusion_reason,
            original_type: None,
            version,
        },
        serde_typed_default,
    )
}

/// Returns true if any subtype within `ty` is a trait object (`dyn Trait`).
///
/// Walks through `Option<T>`, `Vec<T>`, `Arc<T>`, and other generic wrappers by
/// recursing into `Type::Path` generic arguments, `Type::Reference` inner types,
/// `Type::Tuple` elements, `Type::Group`, and `Type::Paren`. This covers the common
/// cases like `Arc<dyn Trait>`, `Option<Box<dyn Trait>>`, and `Vec<Arc<dyn Trait>>`.
pub(crate) fn has_dyn_trait_object(ty: &syn::Type) -> bool {
    match ty {
        syn::Type::TraitObject(_) => true,
        syn::Type::Path(type_path) => type_path.path.segments.iter().any(|seg| {
            if let syn::PathArguments::AngleBracketed(args) = &seg.arguments {
                args.args.iter().any(|arg| {
                    if let syn::GenericArgument::Type(inner) = arg {
                        has_dyn_trait_object(inner)
                    } else {
                        false
                    }
                })
            } else {
                false
            }
        }),
        syn::Type::Reference(type_ref) => has_dyn_trait_object(&type_ref.elem),
        syn::Type::Tuple(type_tuple) => type_tuple.elems.iter().any(has_dyn_trait_object),
        syn::Type::Group(type_group) => has_dyn_trait_object(&type_group.elem),
        syn::Type::Paren(type_paren) => has_dyn_trait_object(&type_paren.elem),
        _ => false,
    }
}
