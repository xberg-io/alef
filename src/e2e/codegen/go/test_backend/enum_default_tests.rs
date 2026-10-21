//! Regression coverage for `go_enum_variant_default_expression`: a Go trait-bridge stub's
//! default return value for an enum-typed method must match the enum's real Go representation --
//! a bare package-level constant for a string-backed enum, but a composite literal for a
//! sealed-interface (or other struct-shaped) one, which has no such constant.
//!
//! Split out of `test_backend.rs`, which is at the 1,000-line cap and may not grow.

use super::emit_test_backend_with_context;
use crate::core::config::TraitBridgeConfig;
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, MethodDef, ParamDef, PrimitiveType, ReceiverKind, TypeRef};

fn make_fixture(id: &str) -> crate::e2e::fixture::Fixture {
    crate::e2e::fixture::Fixture {
        docs: None,
        requirements: Vec::new(),
        id: id.to_string(),
        category: None,
        description: "test".to_string(),
        tags: vec![],
        skip: None,
        env: None,
        setup: Vec::new(),
        call: None,
        input: serde_json::Value::Null,
        mock_response: Some(crate::e2e::fixture::MockResponse {
            status: 200,
            body: Some(serde_json::Value::Null),
            stream_chunks: None,
            headers: std::collections::BTreeMap::new(),
            delay_ms: None,
            stream_chunk_delay_ms: None,
        }),
        source: String::new(),
        http: None,
        asyncapi: None,
        websocket: None,
        client_options: None,
        preserve_input_urls: false,
        assertions: vec![],
        visitor: None,
        args: vec![],
        assertion_recipes: vec![],
    }
}

fn make_method(name: &str, return_type: TypeRef) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        params: Vec::<ParamDef>::new(),
        return_type,
        is_async: false,
        is_static: false,
        error_type: None,
        doc: String::new(),
        receiver: Some(ReceiverKind::Ref),
        cfg: None,
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

/// A sealed-interface enum (one whose Go declaration is `type X interface { .. }`, with a
/// concrete struct per variant) has no package-level constant naming its first variant --
/// unlike a plain string enum. Regression for the trait-bridge fixture failure where the Go
/// stub emitted the bare identifier `binding.SampleSemanticsLegibility`, which names a
/// *type*, not a value, and fails with "is not an expression". The stub must construct the
/// concrete variant struct instead: `binding.SampleSemanticsLegibility{}`.
#[test]
fn test_go_stub_constructs_a_sealed_interface_enum_default_instead_of_naming_a_bare_type() {
    let data_enum = EnumDef {
        name: "SampleSemantics".to_string(),
        rust_path: "sample_crate::SampleSemantics".to_string(),
        variants: vec![EnumVariant {
            name: "Legibility".to_string(),
            fields: vec![FieldDef {
                name: "scale_max".to_string(),
                ty: TypeRef::Primitive(PrimitiveType::F64),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    };
    let method = make_method("confidence_semantics", TypeRef::Named("SampleSemantics".to_string()));

    let trait_bridge = TraitBridgeConfig {
        trait_name: "SampleBackend".to_string(),
        super_trait: None,
        register_fn: Some("register_sample_backend".to_string()),
        ..TraitBridgeConfig::default()
    };
    let fixture = make_fixture("confidence_semantics_test");
    let methods = vec![&method];
    let mut enum_names = std::collections::HashSet::new();
    enum_names.insert("SampleSemantics");
    let excluded = std::collections::HashSet::new();

    let emission = emit_test_backend_with_context(
        &trait_bridge,
        &methods,
        &fixture,
        &excluded,
        "binding",
        &enum_names,
        std::slice::from_ref(&data_enum),
    );

    assert!(
        emission.setup_block.contains("binding.SampleSemanticsLegibility{}"),
        "a sealed-interface enum's default must construct the concrete variant struct, got:\n{}",
        emission.setup_block
    );
    assert!(
        !emission
            .setup_block
            .contains("return binding.SampleSemanticsLegibility }"),
        "must not name the bare variant type as if it were a constant value, got:\n{}",
        emission.setup_block
    );
}

/// Negative control for the sealed-interface fix above: a plain string-backed enum (the shape
/// most IR enums have) still emits its
/// first variant as a bare package-level constant, with no `{}` appended. A fix that
/// over-applied the composite-literal construction to every enum would fail this.
#[test]
fn test_go_stub_still_names_a_unit_string_enum_variant_as_a_bare_constant() {
    let unit_enum = EnumDef {
        name: "SampleOrientation".to_string(),
        rust_path: "sample_crate::SampleOrientation".to_string(),
        variants: vec![EnumVariant {
            name: "SelfCorrecting".to_string(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let method = make_method(
        "page_orientation_handling",
        TypeRef::Named("SampleOrientation".to_string()),
    );
    let trait_bridge = TraitBridgeConfig {
        trait_name: "SampleBackend".to_string(),
        super_trait: None,
        register_fn: Some("register_sample_backend".to_string()),
        ..TraitBridgeConfig::default()
    };
    let fixture = make_fixture("page_orientation_test");
    let methods = vec![&method];
    let mut enum_names = std::collections::HashSet::new();
    enum_names.insert("SampleOrientation");
    let excluded = std::collections::HashSet::new();

    let emission = emit_test_backend_with_context(
        &trait_bridge,
        &methods,
        &fixture,
        &excluded,
        "binding",
        &enum_names,
        std::slice::from_ref(&unit_enum),
    );

    assert!(
        emission
            .setup_block
            .contains("return binding.SampleOrientationSelfCorrecting"),
        "a unit-string enum's default must stay a bare package-level constant, got:\n{}",
        emission.setup_block
    );
    assert!(
        !emission.setup_block.contains("SampleOrientationSelfCorrecting{}"),
        "a unit-string enum must not get a composite-literal default, got:\n{}",
        emission.setup_block
    );
}
