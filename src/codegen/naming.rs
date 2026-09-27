//! Canonical naming policy for everything alef generates.
//!
//! This module is a facade. It owns no logic; it declares the submodules and re-exports their
//! public surface so that `crate::codegen::naming::<item>` keeps resolving for every backend.
//!
//! The split follows the four name surfaces the project treats as separate — a public host
//! identifier, a serde wire name, an internal generated Rust name, and an ABI/native symbol —
//! rather than following line count. Each surface answers to a different authority (a host
//! language's grammar, serde's derive, rustc, the C linker), so each gets a module that can
//! change for exactly one of those reasons:
//!
//! - [`surfaces`] — the vocabulary ([`NameSurface`], [`IdentifierContext`],
//!   [`PublicIdentifierKind`]) plus collision detection over a generated-name scope.
//! - [`wire`] — serde wire names. Never a host identifier.
//! - [`host`] — public host-language identifiers, the names a binding's consumer types.
//! - [`symbols`] — internal Rust identifiers and C ABI / native symbols.
//! - [`identifiers`] — identifier legality and escaping, applied by every surface above.
//! - [`languages`] — the per-language spelling choice (Go's `URL`, C#'s `Json`, …).
//! - [`case`] — language-agnostic mechanical case conversion, the primitives the rest compose.
//! - [`ts_property_key`] — rendering a wire name into TypeScript key position. ~keep

pub mod case;
pub mod host;
pub mod identifiers;
pub mod languages;
pub mod surfaces;
pub mod symbols;
pub mod ts_property_key;
pub mod wire;

pub use case::{
    pascal_to_camel, pascal_to_pascal, pascal_to_screaming_snake, pascal_to_snake, to_class_name, to_constant_name,
    underscore_camel_case,
};
pub(crate) use host::public_casing;
pub use host::{cased_public_type_name, public_field_name, public_host_identifier, qualified_type_path};
pub use identifiers::{
    dart_tuple_field_identifier, dart_type_identifier, dart_value_identifier, escape_identifier, escape_identifier_for,
    is_valid_identifier, is_valid_identifier_for, validate_identifier,
};
pub use languages::{
    csharp_type_name, csharp_variant_name, csharp_wrapper_class_name, go_error_type_name, go_free_function_name,
    go_package_name_from_module, go_param_name, go_type_name, go_variant_name, kotlin_android_wrapper_object_name,
    node_type_name, to_csharp_name, to_elixir_name, to_go_name, to_java_name, to_node_name, to_php_name,
    to_python_name, to_ruby_name,
};
pub use surfaces::{
    IdentifierContext, NameCollision, NameError, NameSurface, PublicIdentifierKind, detect_name_collisions,
};
pub use symbols::{
    abi_symbol, abi_symbol_from_components, ffi_builtin_error_code_prefix, ffi_error_code_variant_name,
    internal_rust_identifier, to_c_name,
};
pub use wire::{
    apply_serde_rename_all, field_uses_duration_map_wire, serde_wire_name, wire_field_name, wire_field_name_camel,
    wire_variant_value,
};

#[cfg(test)]
mod tests;
