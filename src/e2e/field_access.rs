//! Field path resolution for nested struct/map access in e2e assertions.
//!
//! The `FieldResolver` maps fixture field paths (e.g., "metadata.title") to
//! actual API struct paths (e.g., "metadata.document.title") and generates
//! language-specific accessor expressions.

mod ir_collection;
mod ir_enum;
mod ir_result_fields;
mod leaf_anchor;
mod optional_renderers;
mod parse;
mod python_renderer;
mod python_typeddict;
mod renderers;
mod resolver;
mod types;

pub use leaf_anchor::LeafAnchor;
pub(crate) use types::WasmEnumRepresentation;
pub use types::{
    DartFirstClassMap, FieldResolver, IrCollectionMap, IrEnumMap, IrResultFieldMap, JsonNavStep, PhpGetterMap,
    PythonTypedDictMap, StringyField, StringyFieldKind, SwiftFirstClassMap, VariantAccessorMap,
};

#[cfg(test)]
#[path = "field_access/variant_narrowing_tests.rs"]
mod variant_narrowing_tests;

#[cfg(test)]
#[path = "field_access/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "field_access/ir_enum_tests.rs"]
mod ir_enum_tests;

#[cfg(test)]
#[path = "field_access/csharp_optional_index_tests.rs"]
mod csharp_optional_index_tests;

#[cfg(test)]
#[path = "field_access/zig_method_call_accessor_tests.rs"]
mod zig_method_call_accessor_tests;

#[cfg(test)]
#[path = "field_access/ir_wire_optional_fields_tests.rs"]
mod ir_wire_optional_fields_tests;

#[cfg(test)]
#[path = "field_access/is_array_namespace_tests.rs"]
mod is_array_namespace_tests;

#[cfg(test)]
#[path = "field_access/accessor_namespace_agreement_tests.rs"]
mod accessor_namespace_agreement_tests;

#[cfg(test)]
#[path = "field_access/wasm_accessor_tests.rs"]
mod wasm_accessor_tests;

#[cfg(test)]
#[path = "field_access/map_key_quoting_tests.rs"]
mod map_key_quoting_tests;

#[cfg(test)]
#[path = "field_access/swift_json_bridged_alias_prefix_tests.rs"]
mod swift_json_bridged_alias_prefix_tests;

#[cfg(test)]
#[path = "field_access/swift_json_bridged_navigation_tests.rs"]
mod swift_json_bridged_navigation_tests;

#[cfg(test)]
#[path = "field_access/envelope_nested_path_tests.rs"]
mod envelope_nested_path_tests;

#[cfg(test)]
#[path = "field_access/is_valid_for_result_anchoring_tests.rs"]
mod is_valid_for_result_anchoring_tests;

#[cfg(test)]
#[path = "field_access/config_declared_optional_provenance_tests.rs"]
mod config_declared_optional_provenance_tests;

#[cfg(test)]
#[path = "field_access/tagged_union_method_call_extension_tests.rs"]
mod tagged_union_method_call_extension_tests;

#[cfg(test)]
#[path = "field_access/python_optional_index_tests.rs"]
mod python_optional_index_tests;

#[cfg(test)]
#[path = "field_access/is_array_ir_fallback_tests.rs"]
mod is_array_ir_fallback_tests;

#[cfg(test)]
#[path = "field_access/byte_payload_result_tests.rs"]
mod byte_payload_result_tests;
