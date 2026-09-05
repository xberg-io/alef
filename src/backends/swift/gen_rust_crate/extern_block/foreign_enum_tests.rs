//! Regression coverage for a foreign, `[[crates.source_crates]]`-merged fieldless enum used as a
//! function parameter AND return type on an otherwise-infallible function.
//!
//! `emit_function_shim` (`shims.rs`) reconstructs a unit-enum parameter from its wire `String` via
//! a fallible helper (`__alef_{enum}_from_swift_string`, see `type_bridge::enum_from_string_fn_name`)
//! and, when the underlying Rust function has no `error_type` of its own, forces the SHIM's return
//! type to `Result<_, String>` purely so the reconstruction's `?` has somewhere to propagate --
//! see `has_fallible_enum_param`/`forced_fallible` there. `emit_extern_block_for_functions`
//! (`extern_block.rs`) builds the separate `#[swift_bridge::bridge]` extern declaration for the
//! same function; before this fix it computed the declared return type from `f.error_type.is_some()`
//! alone, so it never noticed the shim's forced fallibility and declared a bare, non-`Result`
//! return. swift-bridge parses the emitted `pub fn` against that declaration and rejects the
//! mismatch: error[E0308]: expected enum `Swatch`, found enum `Result<Swatch, String>`. ~keep

use super::emit_extern_block_for_functions;
use crate::core::ir::{FunctionDef, ParamDef, TypeRef};
use std::collections::{BTreeSet, HashMap, HashSet};

/// Fieldless enum name standing in for a `[[crates.source_crates]]`-merged foreign type
/// (`toolkit`'s `Swatch`/`foreign_core::Swatch` in the real fixture). Ownership (host vs.
/// foreign) is irrelevant to this defect -- `unit_enum_names` makes no such distinction, and
/// neither does the fix -- so a neutral name is used rather than reusing the real fixture's.
const ENUM_NAME: &str = "PaletteTag";

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

fn enum_sets() -> (HashSet<String>, HashSet<&'static str>) {
    let enum_names: HashSet<String> = [ENUM_NAME.to_string()].into_iter().collect();
    let unit_enum_names: HashSet<&str> = [ENUM_NAME].into_iter().collect();
    (enum_names, unit_enum_names)
}

fn render(functions: &[FunctionDef], enum_names: &HashSet<String>, unit_enum_names: &HashSet<&str>) -> String {
    let handle_returned = HashSet::new();
    let deferred_empty = BTreeSet::new();
    let capsule_types = HashMap::new();
    let opaque_types = ahash::AHashSet::default();

    emit_extern_block_for_functions(
        functions,
        &handle_returned,
        enum_names,
        unit_enum_names,
        &deferred_empty,
        &capsule_types,
        &opaque_types,
    )
    .expect("emit_extern_block_for_functions")
}

/// The exact shape that failed to compile: a unit enum used as BOTH the parameter and the return
/// type of a function with no `error_type` of its own. The parameter must cross as `String` (the
/// established enum-crossing convention -- swift-bridge cannot parse a fieldless enum from Swift
/// directly), and the return type must become `Result<{Enum}, String>`: the raw enum type (not
/// `String` -- the Ok side crosses as the real opaque handle, matching the shim's own
/// `Swatch::from(...)` wrap), Result-wrapped only because the parameter reconstruction can fail.
///
/// Revert the `forced_fallible` fix in `extern_block.rs` (drop the `|| forced_fallible` from the
/// `return_ty` branch condition) to sabotage-verify: the declared return type reverts to a bare
/// `PaletteTag`, and this assertion fails with:
/// `expected block to declare a Result-wrapped return, got:\n    fn retint(tag: String) -> PaletteTag;`
#[test]
fn foreign_unit_enum_param_and_return_forces_a_result_wrapped_declaration() {
    let (enum_names, unit_enum_names) = enum_sets();
    let functions = vec![FunctionDef {
        name: "retint".to_string(),
        params: vec![enum_param("tag")],
        return_type: TypeRef::Named(ENUM_NAME.to_string()),
        error_type: None,
        ..Default::default()
    }];

    let block = render(&functions, &enum_names, &unit_enum_names);

    assert!(
        block.contains("fn retint(tag: String) -> Result<PaletteTag, String>;"),
        "expected block to declare a Result-wrapped return, got:\n{block}"
    );
}

/// Positive control: when the function is already fallible (`error_type` set), the declared
/// return type was always `Result<_, String>` even before this fix -- `forced_fallible` must be
/// additive, not a behavior change for the already-correct case.
#[test]
fn already_fallible_function_is_unaffected() {
    let (enum_names, unit_enum_names) = enum_sets();
    let functions = vec![FunctionDef {
        name: "retint".to_string(),
        params: vec![enum_param("tag")],
        return_type: TypeRef::Named(ENUM_NAME.to_string()),
        error_type: Some("String".to_string()),
        ..Default::default()
    }];

    let block = render(&functions, &enum_names, &unit_enum_names);

    assert!(
        block.contains("fn retint(tag: String) -> Result<PaletteTag, String>;"),
        "an already-fallible function's declaration must be unchanged, got:\n{block}"
    );
}

/// Negative control: a function with no enum parameter at all must not be forced fallible --
/// `has_fallible_enum_param` must not spuriously match a plain `String` parameter.
#[test]
fn non_enum_param_does_not_force_fallibility() {
    let (enum_names, unit_enum_names) = enum_sets();
    let functions = vec![FunctionDef {
        name: "recolor_by_name".to_string(),
        params: vec![string_param("name")],
        return_type: TypeRef::Named(ENUM_NAME.to_string()),
        error_type: None,
        ..Default::default()
    }];

    let block = render(&functions, &enum_names, &unit_enum_names);

    assert!(
        block.contains("fn recolor_by_name(name: String) -> PaletteTag;"),
        "a function with no enum parameter must keep its bare, non-Result return, got:\n{block}"
    );
    assert!(
        !block.contains("Result<PaletteTag"),
        "no Result wrapping should be introduced without a fallible enum param, got:\n{block}"
    );
}

/// A `Vec<{Enum}>` parameter is reconstructed element-wise (`swift_call_arg`'s vec-of-enum
/// branch) and is exactly as fallible as a single enum parameter -- `has_fallible_enum_param`
/// must check the `Vec<Named>` shape too, not only the bare `Named` shape.
#[test]
fn vec_of_foreign_unit_enum_param_also_forces_a_result_wrapped_declaration() {
    let (enum_names, unit_enum_names) = enum_sets();
    let functions = vec![FunctionDef {
        name: "retint_many".to_string(),
        params: vec![ParamDef {
            name: "tags".to_string(),
            ty: TypeRef::Vec(Box::new(TypeRef::Named(ENUM_NAME.to_string()))),
            ..ParamDef::default()
        }],
        return_type: TypeRef::Unit,
        error_type: None,
        ..Default::default()
    }];

    let block = render(&functions, &enum_names, &unit_enum_names);

    assert!(
        block.contains("fn retint_many(tags: Vec<String>) -> Result<(), String>;"),
        "a Vec<enum> parameter must also force a Result-wrapped unit return, got:\n{block}"
    );
}
