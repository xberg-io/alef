//! Support for falling back to a nested type's own zero-argument initializer when decoding a
//! `#[serde(default)]` field whose default is `Type::default()` (`typed_default == Empty`).
//!
//! Lives in its own file because `gen_bindings/dto.rs` — the only caller — is at its recorded
//! file-size ceiling (`tests/file_size_baseline.txt`) and may not grow. See
//! `backends::swift::named_serde_default_tests` for the companion test coverage, kept out of
//! `dto.rs` for the same reason.

use crate::backends::swift::gen_bindings::dto::swift_typed_default_literal;
use crate::codegen::shared::binding_fields;
use crate::core::ir::{ApiSurface, DefaultValue, TypeRef};
use std::collections::HashSet;

/// Computes the subset of `known_dto_names` whose generated memberwise `public init` accepts
/// zero arguments — i.e. every visible field either renders a Swift literal default
/// (`= <literal>`) or is Optional (`= nil`), the exact per-field rule `emit_first_class_struct`'s
/// `params` loop uses to decide whether a parameter carries a default.
///
/// A name in this set can stand in for its own Rust `Default::default()` inside another type's
/// decoder as a bare `TypeName()` call: that call is guaranteed to compile, and — because it
/// runs through the same defaults the memberwise init encodes — reconstructs the same field
/// values `Default::default()` would produce in Rust. See [`zero_arg_named_default`].
///
/// Enums are never members: `api.types` holds only structs, so a unit or data-variant serde enum
/// in `known_dto_names` is silently excluded. Swift gives no bare `EnumName()` constructor, and
/// for an `impl Default for SomeEnum` gated behind Cargo features, alef has no way to know which
/// variant it resolves to.
///
/// No fixed-point iteration is needed (unlike `compute_first_class_dto_names`): a field whose own
/// type is `Named` and defaults via `Empty` is *not* given a default at its container's
/// init-parameter level, so nesting cannot make an otherwise-non-constructible type
/// constructible — each type's membership depends only on its own fields, never on another
/// type's membership. ~keep
pub(crate) fn compute_zero_arg_constructible_names(
    api: &ApiSurface,
    known_dto_names: &HashSet<String>,
) -> HashSet<String> {
    api.types
        .iter()
        .filter(|t| !t.is_trait && !t.is_opaque && t.has_serde && !t.fields.is_empty())
        .filter(|t| known_dto_names.contains(&t.name))
        .filter(|t| {
            binding_fields(&t.fields).all(|field| {
                let already_optional = matches!(&field.ty, TypeRef::Optional(_));
                field.optional
                    || already_optional
                    || field
                        .typed_default
                        .as_ref()
                        .and_then(swift_typed_default_literal)
                        .is_some()
            })
        })
        .map(|t| t.name.clone())
        .collect()
}

/// The `TypeName()` fallback for a non-Optional `Named` field decoding its own `Empty` default.
///
/// `swift_type_based_default` has no zero for `Named` — a struct's "zero" is whatever its own
/// fields default to, not a value that function can invent. When `zero_arg_constructible_names`
/// (from [`compute_zero_arg_constructible_names`]) says the named type's own memberwise init
/// takes zero arguments, `TypeName()` runs that same init and reconstructs the exact
/// `Default::default()` value, so it is a safe fallback rather than a guess. Anything else —
/// an enum, a struct with a required field of its own, or a `typed_default` other than `Empty` —
/// returns `None` and leaves the caller's required `decode` in place. ~keep
pub(crate) fn zero_arg_named_default(
    typed_default: &Option<DefaultValue>,
    ty: &TypeRef,
    swift_ty: &str,
    zero_arg_constructible_names: &HashSet<String>,
) -> Option<String> {
    if !matches!(typed_default, Some(DefaultValue::Empty)) {
        return None;
    }
    let TypeRef::Named(name) = ty else {
        return None;
    };
    zero_arg_constructible_names
        .contains(name)
        .then(|| format!("{swift_ty}()"))
}
