use crate::core::ir::TypeRef;
use ahash::AHashSet;

/// Returns `true` when `ty` is `Vec<Named>` where `Named` is a unit enum (in `enum_names`
/// but NOT in `tagged_data_enum_names`).
pub(super) fn is_vec_of_unit_enum(
    ty: &TypeRef,
    enum_names: &AHashSet<String>,
    tagged_data_enum_names: &AHashSet<String>,
) -> bool {
    matches!(
        ty,
        TypeRef::Vec(inner)
            if matches!(inner.as_ref(), TypeRef::Named(n)
                if enum_names.contains(n) && !tagged_data_enum_names.contains(n))
    )
}

/// Resolve the prefixed binding name for a unit-enum element inside a `Vec<UnitEnum>` field.
pub(super) fn vec_unit_enum_inner_name(
    ty: &TypeRef,
    enum_names: &AHashSet<String>,
    tagged_data_enum_names: &AHashSet<String>,
    prefix: &str,
) -> Option<String> {
    if let TypeRef::Vec(inner) = ty
        && let TypeRef::Named(n) = inner.as_ref()
        && enum_names.contains(n)
        && !tagged_data_enum_names.contains(n)
    {
        Some(format!("{prefix}{n}"))
    } else {
        None
    }
}
