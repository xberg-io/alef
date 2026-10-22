//! Regression coverage for the type-method analog of `foreign_enum_tests.rs`: a fieldless enum
//! parameter on an INFALLIBLE instance method.
//!
//! `emit_type_method_shims` (`wrappers/methods.rs`) reconstructs a unit-enum parameter from its
//! wire `String` via a fallible helper and, when the method itself has no `error_type`, forces
//! the SHIM's return type to `Result<_, AlefBridgeError>` purely so the reconstruction's `?` has
//! somewhere to propagate -- see `has_fallible_enum_param`/`forced_fallible` there.
//! `emit_extern_block_for_type_methods` (`extern_block.rs`) builds the separate
//! `#[swift_bridge::bridge]` extern declaration for the same method; before this fix it computed
//! the declared return type from `method.error_type.is_some()` alone, so it never noticed the
//! shim's forced fallibility and declared a bare, non-`Result` return. swift-bridge parses the
//! emitted `pub fn` against that declaration and rejects the mismatch with `error[E0308]`. ~keep

use super::emit_extern_block_for_type_methods;
use crate::core::ir::{MethodDef, ParamDef, ReceiverKind, TypeDef, TypeRef};
use std::collections::HashSet;

/// Fieldless enum name standing in for a wire-`String`-crossing enum parameter. Ownership (host
/// vs. foreign) is irrelevant to this defect, as with the free-function fix. A neutral name is
/// used rather than any real consumer fixture name.
const ENUM_NAME: &str = "PaletteTag";
const TYPE_NAME: &str = "Mixer";

fn enum_param(name: &str) -> ParamDef {
    ParamDef {
        name: name.to_string(),
        ty: TypeRef::Named(ENUM_NAME.to_string()),
        ..ParamDef::default()
    }
}

fn string_param(name: &str) -> ParamDef {
    ParamDef {
        name: name.to_string(),
        ty: TypeRef::String,
        ..ParamDef::default()
    }
}

fn enum_sets() -> (HashSet<&'static str>, HashSet<&'static str>) {
    let enum_names: HashSet<&str> = [ENUM_NAME].into_iter().collect();
    let unit_enum_names: HashSet<&str> = [ENUM_NAME].into_iter().collect();
    (enum_names, unit_enum_names)
}

fn render(ty: &TypeDef, enum_names: &HashSet<&str>, unit_enum_names: &HashSet<&str>) -> String {
    let handle_returned = HashSet::new();
    emit_extern_block_for_type_methods(ty, &handle_returned, enum_names, unit_enum_names)
        .expect("emit_extern_block_for_type_methods")
}

/// The exact shape that fails to compile today: a unit enum used as the parameter of an
/// otherwise-infallible instance method (no `error_type`). The declared return type must become
/// `Result<{Enum}, AlefBridgeError>` -- Result-wrapped only because the parameter reconstruction can fail,
/// exactly mirroring `foreign_unit_enum_param_and_return_forces_a_result_wrapped_declaration` in
/// `foreign_enum_tests.rs` for the free-function path.
///
/// Revert the `forced_fallible` fix (drop the `|| forced_fallible` from the `return_ty` branch
/// condition in `emit_extern_block_for_type_methods`) to sabotage-verify: the declared return
/// type reverts to a bare `PaletteTag`, and this assertion fails with:
/// `expected block to declare a Result-wrapped return, got:\n
/// fn mixer_retint(client: &Mixer, tag: String) -> PaletteTag;`
#[test]
fn foreign_unit_enum_param_forces_a_result_wrapped_method_declaration() {
    let (enum_names, unit_enum_names) = enum_sets();
    let ty = TypeDef {
        name: TYPE_NAME.to_string(),
        is_opaque: true,
        methods: vec![MethodDef {
            name: "retint".to_string(),
            params: vec![enum_param("tag")],
            return_type: TypeRef::Named(ENUM_NAME.to_string()),
            error_type: None,
            receiver: Some(ReceiverKind::Ref),
            ..MethodDef::default()
        }],
        ..TypeDef::default()
    };

    let block = render(&ty, &enum_names, &unit_enum_names);

    assert!(
        block.contains("fn mixer_retint(client: &Mixer, tag: String) -> Result<PaletteTag, AlefBridgeError>;"),
        "expected block to declare a Result-wrapped return, got:\n{block}"
    );
}

/// Positive control: an already-fallible method (`error_type` set) was always declared
/// `Result<_, AlefBridgeError>` -- `forced_fallible` must be additive, not a behavior change for the
/// already-correct case.
#[test]
fn already_fallible_method_is_unaffected() {
    let (enum_names, unit_enum_names) = enum_sets();
    let ty = TypeDef {
        name: TYPE_NAME.to_string(),
        is_opaque: true,
        methods: vec![MethodDef {
            name: "retint".to_string(),
            params: vec![enum_param("tag")],
            return_type: TypeRef::Named(ENUM_NAME.to_string()),
            error_type: Some("String".to_string()),
            receiver: Some(ReceiverKind::Ref),
            ..MethodDef::default()
        }],
        ..TypeDef::default()
    };

    let block = render(&ty, &enum_names, &unit_enum_names);

    assert!(
        block.contains("fn mixer_retint(client: &Mixer, tag: String) -> Result<PaletteTag, AlefBridgeError>;"),
        "an already-fallible method's declaration must be unchanged, got:\n{block}"
    );
}

/// Negative control: a method with no enum parameter must not be forced fallible --
/// `has_fallible_enum_param` must not spuriously match a plain `String` parameter.
#[test]
fn non_enum_param_does_not_force_method_fallibility() {
    let (enum_names, unit_enum_names) = enum_sets();
    let ty = TypeDef {
        name: TYPE_NAME.to_string(),
        is_opaque: true,
        methods: vec![MethodDef {
            name: "recolor_by_name".to_string(),
            params: vec![string_param("name")],
            return_type: TypeRef::Named(ENUM_NAME.to_string()),
            error_type: None,
            receiver: Some(ReceiverKind::Ref),
            ..MethodDef::default()
        }],
        ..TypeDef::default()
    };

    let block = render(&ty, &enum_names, &unit_enum_names);

    assert!(
        block.contains("fn mixer_recolor_by_name(client: &Mixer, name: String) -> PaletteTag;"),
        "a method with no enum parameter must keep its bare, non-Result return, got:\n{block}"
    );
    assert!(
        !block.contains("Result<PaletteTag"),
        "no Result wrapping should be introduced without a fallible enum param, got:\n{block}"
    );
}

/// A `Vec<{Enum}>` parameter is reconstructed element-wise and is exactly as fallible as a
/// single enum parameter -- `has_fallible_enum_param` must check the `Vec<Named>` shape too.
#[test]
fn vec_of_foreign_unit_enum_param_also_forces_a_result_wrapped_method_declaration() {
    let (enum_names, unit_enum_names) = enum_sets();
    let ty = TypeDef {
        name: TYPE_NAME.to_string(),
        is_opaque: true,
        methods: vec![MethodDef {
            name: "retint_many".to_string(),
            params: vec![ParamDef {
                name: "tags".to_string(),
                ty: TypeRef::Vec(Box::new(TypeRef::Named(ENUM_NAME.to_string()))),
                ..ParamDef::default()
            }],
            return_type: TypeRef::Unit,
            error_type: None,
            receiver: Some(ReceiverKind::Ref),
            ..MethodDef::default()
        }],
        ..TypeDef::default()
    };

    let block = render(&ty, &enum_names, &unit_enum_names);

    assert!(
        block.contains("fn mixer_retint_many(client: &Mixer, tags: Vec<String>) -> Result<(), AlefBridgeError>;"),
        "a Vec<enum> parameter must also force a Result-wrapped unit return, got:\n{block}"
    );
}
