//! Trait-bridge helpers for the Kotlin JVM backend.
//!
//! Handles detection of functions whose signatures involve trait types, which
//! must be excluded from the Kotlin wrapper (the Java facade handles trait
//! registration via a separate interface).

use alef_core::ir::TypeRef;

/// Returns `true` if the type reference, recursively, includes any name in the
/// supplied set. Used by the Kotlin/Java wrappers to skip functions whose
/// signature touches trait types (the Java facade doesn't expose those).
pub(super) fn type_ref_uses_named(ty: &TypeRef, names: &std::collections::HashSet<&str>) -> bool {
    match ty {
        TypeRef::Named(n) => names.contains(n.as_str()),
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => type_ref_uses_named(inner, names),
        TypeRef::Map(k, v) => type_ref_uses_named(k, names) || type_ref_uses_named(v, names),
        _ => false,
    }
}
