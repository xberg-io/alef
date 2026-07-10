use crate::core::ir::{MethodDef, TypeRef};

use super::super::helpers::{extract_binding_exclusion_reason, extract_doc_comments, extract_version_annotation};
use super::returns::detect_cow_return;
use super::{detect_receiver, extract_params, resolve_return_type, unwrap_future_return};

/// Extract a single method from an impl block.
/// `parent_type_name` is used to resolve `Self` references in return types and params.
/// `trait_source` is the fully qualified trait path if this method comes from a trait impl.
pub(crate) fn extract_method(
    method: &syn::ImplItemFn,
    _crate_name: &str,
    parent_type_name: &str,
    trait_source: Option<String>,
    result_wrapping_aliases: &ahash::AHashSet<String>,
) -> MethodDef {
    let name = method.sig.ident.to_string();
    let doc = extract_doc_comments(&method.attrs);
    let binding_exclusion_reason = extract_binding_exclusion_reason(&method.attrs);
    let binding_excluded = binding_exclusion_reason.is_some();
    let mut is_async = method.sig.asyncness.is_some();

    let (mut return_type, mut error_type, returns_ref) = resolve_return_type(&method.sig.output);

    let returns_cow = detect_cow_return(&method.sig.output);

    if !is_async {
        if let Some((inner, future_error_type)) = unwrap_future_return(&method.sig.output, result_wrapping_aliases) {
            is_async = true;
            return_type = inner;
            if future_error_type.is_some() {
                error_type = future_error_type;
            }
        }
    }

    resolve_self_refs(&mut return_type, parent_type_name);

    let (receiver, is_static) = detect_receiver(&method.sig.inputs);
    let mut params = extract_params(&method.sig.inputs);
    for param in &mut params {
        resolve_self_refs(&mut param.ty, parent_type_name);
    }

    MethodDef {
        name,
        params,
        return_type,
        is_async,
        is_static,
        error_type,
        doc,
        receiver,
        sanitized: false,
        trait_source,
        returns_ref,
        returns_cow,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded,
        binding_exclusion_reason,
        version: extract_version_annotation(&method.attrs),
    }
}

/// Replace `TypeRef::Named("Self")` with the actual parent type name, recursively.
fn resolve_self_refs(ty: &mut TypeRef, parent_type_name: &str) {
    match ty {
        TypeRef::Named(n) if n == "Self" => *n = parent_type_name.to_string(),
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => resolve_self_refs(inner, parent_type_name),
        TypeRef::Map(k, v) => {
            resolve_self_refs(k, parent_type_name);
            resolve_self_refs(v, parent_type_name);
        }
        _ => {}
    }
}
