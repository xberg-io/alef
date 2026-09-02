//! alef #102 follow-up: two Magnus trait-bridge generators each hand-recomputed a subset of
//! `has_error` (just `func.error_type.is_some()`), independent of a `?` their own body emits
//! unconditionally elsewhere. A function with no declared error type then got a bare-`T`
//! signature wrapped around a body containing a `?`, which rustc rejects with E0277.
//!
//! - `gen_options_field_bridge_function` always parses its `args: &[magnus::Value]` through
//!   `scan_args::<...>(args)?` — that `?` fires regardless of `func.error_type`.
//! - `gen_bridge_function` emits `serde_json::from_str(..)?` / `.transpose()?` for any non-opaque
//!   `Named`/`Optional<Named>` parameter that isn't a configured "default type" — gated on
//!   parameter shape, never on `func.error_type`.
//!
//! 0.82.1 regression (shipped, fixed here): PR #292 changed `gen_bridge_function`'s bridge
//! construction from an infallible `{struct_name}::new({param})` to a fallible
//! `{struct_name}::new({param}, {name})?`, unconditionally on every call — but nothing taught
//! `has_error` about it, so a function with neither a declared error type nor a fallible param
//! kept a bare-`T` signature wrapped around that same unconditional `?`. Every generated Magnus
//! trait-bridge free function is Result-shaped now: the bridge constructor's own fallibility is
//! never conditional.

use alef::backends::magnus::trait_bridge::{gen_bridge_function, gen_options_field_bridge_function};
use alef::codegen::type_mapper::IdentityMapper;
use alef::core::config::{BridgeBinding, TraitBridgeConfig};
use alef::core::ir::{ApiSurface, FunctionDef, ParamDef, TypeRef};

fn function_param_bridge_config() -> TraitBridgeConfig {
    TraitBridgeConfig {
        trait_name: "Visitor".to_string(),
        type_alias: Some("VisitorHandle".to_string()),
        bind_via: BridgeBinding::FunctionParam,
        ..Default::default()
    }
}

fn options_field_bridge_config() -> TraitBridgeConfig {
    TraitBridgeConfig {
        trait_name: "Visitor".to_string(),
        type_alias: Some("VisitorHandle".to_string()),
        param_name: Some("visitor".to_string()),
        bind_via: BridgeBinding::OptionsField,
        options_type: Some("RenderOptions".to_string()),
        options_field: Some("visitor".to_string()),
        ..Default::default()
    }
}

/// A bridge-param function with no declared error type, whose one non-bridge param
/// (`options: RenderOptions`) is a non-opaque `Named` type that isn't a configured "default
/// type" — so `gen_bridge_function` must emit a fallible `serde_json::from_str(..)?` deser
/// binding for it, independent of `error_type`.
fn render_document_function() -> FunctionDef {
    FunctionDef {
        name: "render_document".to_string(),
        rust_path: "sample_core::render_document".to_string(),
        params: vec![
            ParamDef {
                name: "visitor".to_string(),
                ty: TypeRef::Named("VisitorHandle".to_string()),
                ..ParamDef::default()
            },
            ParamDef {
                name: "options".to_string(),
                ty: TypeRef::Named("RenderOptions".to_string()),
                ..ParamDef::default()
            },
        ],
        return_type: TypeRef::String,
        error_type: None,
        ..FunctionDef::default()
    }
}

#[test]
fn bridge_function_without_error_type_stays_result_shaped_when_a_param_needs_fallible_deser() {
    let code = gen_bridge_function(
        &ApiSurface::default(),
        &render_document_function(),
        0,
        &function_param_bridge_config(),
        &IdentityMapper,
        &ahash::AHashSet::new(),
        &std::collections::HashSet::new(),
        "sample_core",
    );

    assert!(
        code.contains("-> Result<"),
        "a param needing fallible deser must force a Result-shaped signature even with no \
         declared error type, got: {code}"
    );
    assert!(
        code.contains("serde_json::from_str(&options)") && code.contains('?'),
        "the deser binding for a non-default-type param must stay fallible, got: {code}"
    );
    assert!(
        code.contains("Ok("),
        "the tail expression must be Ok(..)-wrapped to fit the Result-shaped signature, got: {code}"
    );
}

/// The same function, but `options` becomes an opaque type (skips deser entirely) and there's no
/// other fallible source among the params or `error_type` — the signature must still be
/// `Result`-shaped, because the bridge parameter itself (`visitor`) is always constructed via a
/// fallible `RbVisitorBridge::new(..)?` (the constructor validates required methods and builds
/// the runtime dispatcher). A bare-`T` return here would wrap that same unconditional `?` in a
/// body rustc rejects with E0277 — the 0.82.1 regression this test pins.
#[test]
fn bridge_function_without_error_type_and_without_fallible_params_stays_result_shaped_for_fallible_constructor() {
    let mut func = render_document_function();
    func.params[1].ty = TypeRef::Primitive(alef::core::ir::PrimitiveType::U32);
    func.params[1].name = "count".to_string();

    let code = gen_bridge_function(
        &ApiSurface::default(),
        &func,
        0,
        &function_param_bridge_config(),
        &IdentityMapper,
        &ahash::AHashSet::new(),
        &std::collections::HashSet::new(),
        "sample_core",
    );

    assert!(
        code.contains("-> Result<"),
        "the bridge constructor's own `?` is unconditional, so the signature must stay \
         Result-shaped even with no declared error type and no fallible param, got: {code}"
    );
    assert!(
        code.contains("RbVisitorBridge::new(") && code.contains('?'),
        "the bridge construction must stay fallible, got: {code}"
    );
    assert!(
        code.contains("Ok("),
        "the tail expression must be Ok(..)-wrapped to fit the Result-shaped signature, got: {code}"
    );
}

/// Structural invariant `gen_bridge_function` must satisfy for every function shape: the
/// generated body contains a `?` if and only if the signature it is wrapped in is
/// `Result`-shaped. Unlike the two regression-pin tests above (which fix one input and assert
/// one direction), this checks both directions across several shapes at once, so neither side
/// can be satisfied by a constant — a hardcoded `-> Result<` or a hardcoded bare return would
/// fail whichever shape disagrees with it.
#[test]
fn bridge_function_body_contains_question_mark_iff_signature_is_result_shaped() {
    let opaque_types = ahash::AHashSet::new();
    let mut default_types = std::collections::HashSet::new();

    let mut declared_error_no_fallible_param = render_document_function();
    declared_error_no_fallible_param.error_type = Some("sample_core::Error".to_string());
    declared_error_no_fallible_param.params[1].ty = TypeRef::Primitive(alef::core::ir::PrimitiveType::U32);
    declared_error_no_fallible_param.params[1].name = "count".to_string();

    let no_declared_error_fallible_deser_param = render_document_function();

    let mut no_declared_error_no_fallible_param = render_document_function();
    no_declared_error_no_fallible_param.params[1].ty = TypeRef::Primitive(alef::core::ir::PrimitiveType::U32);
    no_declared_error_no_fallible_param.params[1].name = "count".to_string();

    // Same shape as above, but `RenderOptions` is a configured "default type": `serde_bindings`
    // takes the `.into()` arm instead of the fallible `serde_json::from_str(..)?` arm, so the
    // *only* remaining source of fallibility in the body is `bridge_wrap`'s constructor call.
    let no_declared_error_default_type_param = render_document_function();
    default_types.insert("RenderOptions");

    let mut declared_error_and_fallible_deser_param = render_document_function();
    declared_error_and_fallible_deser_param.error_type = Some("sample_core::Error".to_string());

    let mut optional_bridge_no_declared_error_no_fallible_param = render_document_function();
    optional_bridge_no_declared_error_no_fallible_param.params[0].optional = true;
    optional_bridge_no_declared_error_no_fallible_param.params[1].ty =
        TypeRef::Primitive(alef::core::ir::PrimitiveType::U32);
    optional_bridge_no_declared_error_no_fallible_param.params[1].name = "count".to_string();

    let shapes: Vec<(&str, FunctionDef, std::collections::HashSet<&str>)> = vec![
        (
            "declared error type, no fallible param",
            declared_error_no_fallible_param,
            std::collections::HashSet::new(),
        ),
        (
            "no declared error type, fallible deser param",
            no_declared_error_fallible_deser_param,
            std::collections::HashSet::new(),
        ),
        (
            "no declared error type, no fallible param",
            no_declared_error_no_fallible_param,
            std::collections::HashSet::new(),
        ),
        (
            "no declared error type, default-type param",
            no_declared_error_default_type_param,
            default_types.clone(),
        ),
        (
            "declared error type and fallible deser param",
            declared_error_and_fallible_deser_param,
            std::collections::HashSet::new(),
        ),
        (
            "optional bridge param, no declared error type, no fallible param",
            optional_bridge_no_declared_error_no_fallible_param,
            std::collections::HashSet::new(),
        ),
    ];

    for (label, func, default_types) in shapes {
        let code = gen_bridge_function(
            &ApiSurface::default(),
            &func,
            0,
            &function_param_bridge_config(),
            &IdentityMapper,
            &opaque_types,
            &default_types,
            "sample_core",
        );

        let is_result_shaped = code.contains("-> Result<");
        let body_is_fallible = code.contains('?');
        assert_eq!(
            is_result_shaped, body_is_fallible,
            "[{label}] a Result-shaped signature must correspond exactly to a body containing              `?`, and vice versa — got is_result_shaped={is_result_shaped},              body_is_fallible={body_is_fallible}, code: {code}"
        );
    }
}

#[test]
fn options_field_bridge_function_without_error_type_stays_result_shaped() {
    let mut func = render_document_function();
    func.name = "render_document_via_options".to_string();
    func.params = vec![ParamDef {
        name: "options".to_string(),
        ty: TypeRef::Named("RenderOptions".to_string()),
        ..ParamDef::default()
    }];

    let code = gen_options_field_bridge_function(
        &ApiSurface::default(),
        &func,
        0,
        &options_field_bridge_config(),
        &IdentityMapper,
        &ahash::AHashSet::new(),
        "sample_core",
    );

    assert!(
        code.contains("-> Result<"),
        "this generator always emits an unconditional `scan_args::<...>(args)?`, so it must stay \
         Result-shaped even with no declared error type, got: {code}"
    );
    assert!(
        code.contains(">(args)?;"),
        "the scan_args parse must stay fallible, got: {code}"
    );
    assert!(
        code.contains("Ok("),
        "the tail expression must be Ok(..)-wrapped to fit the Result-shaped signature, got: {code}"
    );
}
