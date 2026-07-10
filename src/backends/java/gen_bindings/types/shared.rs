use crate::core::config::{BridgeBinding, TraitBridgeConfig};
use crate::core::ir::TypeRef;

/// Resolve a TypeRef to its Java type, replacing unknown/excluded Named types with JsonNode.
///
/// When a field references a type that was excluded from code generation (e.g. `#[alef(skip)]`),
/// we use `JsonNode` to preserve the object structure without requiring a Java type definition.
pub(super) fn resolve_field_type(ty: &TypeRef, visible_types: &std::collections::HashSet<&str>) -> TypeRef {
    match ty {
        TypeRef::Named(name) if !visible_types.contains(name.as_str()) => TypeRef::Json,
        TypeRef::Optional(inner) => TypeRef::Optional(Box::new(resolve_field_type(inner, visible_types))),
        TypeRef::Vec(inner) => TypeRef::Vec(Box::new(resolve_field_type(inner, visible_types))),
        TypeRef::Map(k, v) => TypeRef::Map(
            Box::new(resolve_field_type(k, visible_types)),
            Box::new(resolve_field_type(v, visible_types)),
        ),
        _ => ty.clone(),
    }
}

pub(super) fn is_options_field_bridge(
    type_name: &str,
    field_name: &str,
    field_ty: &TypeRef,
    trait_bridges: &[TraitBridgeConfig],
) -> bool {
    trait_bridges.iter().any(|bridge| {
        let alias_matches = bridge
            .type_alias
            .as_deref()
            .is_none_or(|alias| matches!(field_ty, TypeRef::Named(name) if name == alias));

        bridge.bind_via == BridgeBinding::OptionsField
            && bridge.options_type.as_deref() == Some(type_name)
            && bridge.resolved_options_field() == Some(field_name)
            && alias_matches
    })
}

pub(super) fn options_field_bridge_trait_name(
    type_name: &str,
    field_name: &str,
    field_ty: &TypeRef,
    trait_bridges: &[TraitBridgeConfig],
) -> Option<String> {
    trait_bridges.iter().find_map(|bridge| {
        let alias_matches = bridge
            .type_alias
            .as_deref()
            .is_none_or(|alias| matches!(field_ty, TypeRef::Named(name) if name == alias));

        if bridge.bind_via == BridgeBinding::OptionsField
            && bridge.options_type.as_deref() == Some(type_name)
            && bridge.resolved_options_field() == Some(field_name)
            && alias_matches
        {
            Some(bridge.trait_name.clone())
        } else {
            None
        }
    })
}
