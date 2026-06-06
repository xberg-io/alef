//! Dart-specific e2e generator tests.

use super::stubs::emit_test_backend;
use crate::core::config::TraitBridgeConfig;
use crate::core::ir::{MethodDef, PrimitiveType, TypeRef};
use crate::e2e::fixture::Fixture;

fn make_trait_bridge(trait_name: &str) -> TraitBridgeConfig {
    TraitBridgeConfig {
        trait_name: trait_name.to_string(),
        super_trait: Some("Plugin".to_string()),
        register_fn: Some(format!("register_{}", trait_name.to_lowercase())),
        ..Default::default()
    }
}

fn make_method(name: &str, required: bool) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        params: vec![],
        return_type: TypeRef::Primitive(PrimitiveType::Bool),
        is_async: true,
        is_static: false,
        error_type: None,
        doc: String::new(),
        receiver: Some(crate::core::ir::ReceiverKind::Ref),
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: !required,
        binding_excluded: false,
        binding_exclusion_reason: None,
    }
}

fn make_fixture(id: &str) -> Fixture {
    Fixture {
        id: id.to_string(),
        category: None,
        description: "test".to_string(),
        tags: vec![],
        skip: None,
        env: None,
        call: None,
        input: serde_json::Value::Null,
        mock_response: None,
        source: String::new(),
        http: None,
        assertions: vec![],
        visitor: None,
        args: vec![],
        assertion_recipes: vec![],
    }
}

/// Verify that no sample_core-domain names leak into the generated output when
/// the trait bridge is configured for a synthetic `TestTrait` in `testlib`.
#[test]
fn dart_stub_contains_no_sample_crate_domain_names() {
    let bridge = make_trait_bridge("TestTrait");
    let required_method = make_method("doWork", true);
    let methods = [&required_method];
    let fixture = make_fixture("my_test_fixture");

    let emission = emit_test_backend(&bridge, &methods, &fixture);

    let output = format!("{}\n{}", emission.setup_block, emission.arg_expr);

    assert!(
        !output.contains("SampleCrate"),
        "must not contain literal 'SampleCrate', got:\n{output}"
    );
    assert!(
        !output.contains("sample_crate::"),
        "must not contain 'sample_crate::', got:\n{output}"
    );
    assert!(
        !output.contains("SampleCrateBridge"),
        "must not contain 'SampleCrateBridge', got:\n{output}"
    );
    assert!(
        output.contains("TestStubMyTestFixture"),
        "class name must be derived from fixture id, got:\n{output}"
    );
    assert!(
        output.contains("extends TestTrait"),
        "class must extend the configured trait class, got:\n{output}"
    );
    assert!(
        output.contains("doWork"),
        "required method must be emitted, got:\n{output}"
    );
}

fn make_param(name: &str, ty: TypeRef) -> crate::core::ir::ParamDef {
    crate::core::ir::ParamDef {
        name: name.to_string(),
        ty,
        optional: false,
        default: None,
        sanitized: false,
        typed_default: None,
        is_ref: false,
        is_mut: false,
        newtype_wrapper: None,
        original_type: None,
        map_is_ahash: false,
        map_key_is_cow: false,
        vec_inner_is_ref: false,
        map_is_btree: false,
        core_wrapper: crate::core::ir::CoreWrapper::None,
    }
}

fn make_method_with_params(name: &str, required: bool) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        params: vec![
            make_param("content", TypeRef::Bytes),
            make_param("mime_type", TypeRef::String),
        ],
        return_type: TypeRef::Named("SampleResult".to_string()),
        is_async: true,
        is_static: false,
        error_type: Some("anyhow::Error".to_string()),
        doc: String::new(),
        receiver: Some(crate::core::ir::ReceiverKind::Ref),
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: !required,
        binding_excluded: false,
        binding_exclusion_reason: None,
    }
}

/// Verify params use concrete Dart types (not `dynamic`) and no @override annotation.
#[test]
fn dart_stub_uses_typed_params_not_dynamic() {
    let bridge = make_trait_bridge("TestTrait");
    let required_method = make_method_with_params("extract", true);
    let methods = [&required_method];
    let fixture = make_fixture("my_test_fixture");

    let emission = emit_test_backend(&bridge, &methods, &fixture);
    let output = format!("{}\n{}", emission.setup_block, emission.arg_expr);

    assert!(
        !output.contains("dynamic content"),
        "param must not use `dynamic`, got:\n{output}"
    );
    assert!(
        output.contains("Uint8List content"),
        "bytes param must map to Uint8List, got:\n{output}"
    );
    assert!(
        output.contains("String mimeType"),
        "string param must map to String, got:\n{output}"
    );
    assert!(
        output.contains("Future<SampleResult>"),
        "return type must be concrete not dynamic, got:\n{output}"
    );
    assert!(
        !output.contains("@override"),
        "local class members must not use @override annotation, got:\n{output}"
    );
}

/// Verify that `fixture.input["name"]` is used as the plugin name when present.
#[test]
fn dart_stub_uses_fixture_input_name_for_plugin_name() {
    let bridge = make_trait_bridge("TestTrait");
    let required_method = make_method("doWork", true);
    let methods = [&required_method];
    let mut fixture = make_fixture("my_fixture_id");
    fixture.input = serde_json::json!({ "name": "my-backend-name" });

    let emission = emit_test_backend(&bridge, &methods, &fixture);
    let output = format!("{}\n{}", emission.setup_block, emission.arg_expr);

    assert!(
        output.contains("'my-backend-name'"),
        "plugin name must come from fixture.input.name, got:\n{output}"
    );
    assert!(
        !output.contains("my_fixture_id"),
        "fixture id must not appear as plugin name when input.name is set, got:\n{output}"
    );
}
