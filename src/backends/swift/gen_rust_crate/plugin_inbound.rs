//! Emits the **inbound** plugin trait bridge — Swift implements a Rust trait, Rust calls back.
//!
//! Whereas [`trait_bridge`](super::trait_bridge) generates **outbound** glue (Swift caller →
//! Rust trait object), this module generates the inverse: a Swift class conforms to a
//! protocol, Rust holds a handle, and Rust calls each method on the Swift instance via
//! `extern "Swift"` declarations.
//!
//! This facade preserves the historical `gen_rust_crate::plugin_inbound` module path
//! while keeping inbound generation split by concern.

mod inbound_externs;
mod method_impls;
mod options_fields;
mod wrappers;

use crate::backends::swift::gen_rust_crate::type_bridge::{needs_json_bridge, swift_bridge_rust_type};
use crate::core::config::TraitBridgeConfig;
use crate::core::ir::TypeRef;

pub(crate) use inbound_externs::{emit_extern_block_for_inbound, emit_extern_block_for_inbound_registration};
pub(crate) use options_fields::{
    emit_options_field_factory, emit_options_field_from_impls, emit_options_field_options_helper,
};
pub(crate) use wrappers::{emit_inbound_wrapper, emit_plugin_error_helper};

/// Inbound-specific type bridging.
///
/// All `Named` types are JSON-bridged at the inbound boundary because the Swift side of an
/// `extern "Swift"` shim cannot produce the opaque Rust newtype the way `extern "Rust"`
/// callers do; it has to send a JSON payload that Rust deserialises into the source type.
/// Primitive scalars, `String`, `Vec<u8>`, and `Vec<leaf>` pass through as-is.
pub(super) fn inbound_bridge_type(ty: &TypeRef) -> String {
    match ty {
        TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Named(_)) => "String".to_string(),
        TypeRef::Optional(inner) => format!("Option<{}>", inbound_bridge_type(inner)),
        TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Bytes) => "Vec<u8>".to_string(),
        TypeRef::Vec(inner) => format!("Vec<{}>", inbound_bridge_type(inner)),
        TypeRef::Map(k, v) => {
            format!(
                "std::collections::HashMap<{}, {}>",
                inbound_bridge_type(k),
                inbound_bridge_type(v)
            )
        }
        _ if needs_inbound_json_bridge(ty) => "String".to_string(),
        _ => swift_bridge_rust_type(ty),
    }
}

/// Like [`needs_json_bridge`] but additionally treats every `Named` type as JSON-bridged
/// for inbound transport. Vec<Named-leaf> stays a typed Vec (e.g. `Vec<String>`) when
/// the inner type is a primitive/leaf — only Named-leaf gets escalated.
pub(super) fn needs_inbound_json_bridge(ty: &TypeRef) -> bool {
    if needs_json_bridge(ty) {
        return true;
    }
    matches!(ty, TypeRef::Named(_))
}

/// Returns true when the trait bridge config declares a Plugin super-trait.
pub(super) fn has_plugin_super(bridge_config: &TraitBridgeConfig) -> bool {
    bridge_config
        .super_trait
        .as_deref()
        .map(|s| s == "Plugin" || s.ends_with("::Plugin"))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_inbound_bridge_type_optional_vec_named() {
        let ty = TypeRef::Optional(Box::new(TypeRef::Vec(Box::new(TypeRef::Named(
            "MyCustomType".to_string(),
        )))));

        let result = inbound_bridge_type(&ty);
        assert_eq!(
            result, "Option<Vec<String>>",
            "Optional<Vec<Named>> should become Option<Vec<String>> for JSON bridging"
        );
    }

    #[test]
    fn test_inbound_bridge_type_optional_named() {
        let ty = TypeRef::Optional(Box::new(TypeRef::Named("MyStruct".to_string())));

        let result = inbound_bridge_type(&ty);
        assert_eq!(
            result, "String",
            "Optional<Named> should become String for JSON bridging"
        );
    }

    #[test]
    fn test_inbound_bridge_type_vec_string_in_optional() {
        let ty = TypeRef::Optional(Box::new(TypeRef::Vec(Box::new(TypeRef::String))));

        let result = inbound_bridge_type(&ty);
        assert_eq!(
            result, "Option<Vec<String>>",
            "Optional<Vec<String>> should pass through unchanged"
        );
    }

    #[test]
    fn test_inbound_bridge_type_map_named_string() {
        let ty = TypeRef::Map(
            Box::new(TypeRef::Named("KeyType".to_string())),
            Box::new(TypeRef::String),
        );

        let result = inbound_bridge_type(&ty);
        assert_eq!(
            result, "std::collections::HashMap<String, String>",
            "Map<Named, String> should become HashMap<String, String> for JSON bridging"
        );
    }

    #[test]
    fn test_inbound_bridge_type_vec_u8() {
        let ty = TypeRef::Vec(Box::new(TypeRef::Bytes));

        let result = inbound_bridge_type(&ty);
        assert_eq!(result, "Vec<u8>", "Vec<u8> (Bytes) should remain Vec<u8>");
    }

    #[test]
    fn test_inbound_bridge_type_vec_named() {
        let ty = TypeRef::Vec(Box::new(TypeRef::Named("Item".to_string())));

        let result = inbound_bridge_type(&ty);
        assert_eq!(
            result, "Vec<String>",
            "Vec<Named> should become Vec<String> for JSON bridging"
        );
    }
}
