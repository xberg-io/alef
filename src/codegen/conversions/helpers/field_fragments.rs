use crate::core::ir::TypeRef;

/// Inverse of the sanitization in [`core_to_binding`] for `Vec<_>` fields:
/// given a sanitized binding-side type, emit the expression that rebuilds the
/// core-side value. The default fallback assumes the sanitized form is
/// `Vec<String>` of JSON-serialized elements (the `Vec<Json>` shape); the
/// `Vec<Vec<String>>` special case rebuilds `Vec<(String, String)>` from
/// 2-element inner Vecs (the `parse_homogeneous_tuple` shape — see
/// `core_to_binding::field_conversion_to_binding_cfg`).
pub(crate) fn sanitized_vec_field_to_core_expr(name: &str, ty: &TypeRef) -> String {
    if let TypeRef::Vec(outer_inner) = ty {
        if let TypeRef::Vec(inner) = outer_inner.as_ref() {
            if matches!(inner.as_ref(), TypeRef::String) {
                return format!(
                    "{name}.iter().filter_map(|inner| {{ let mut it = inner.iter().cloned(); Some((it.next()?, it.next()?)) }}).collect()"
                );
            }
        }
    }
    format!("{name}.iter().filter_map(|s| serde_json::from_str(s).ok()).collect()")
}
