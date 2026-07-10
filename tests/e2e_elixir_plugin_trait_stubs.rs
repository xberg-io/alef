//! Verifies the Elixir e2e codegen emits complete plugin trait-bridge stubs with
//! super-trait methods and generic numeric defaults.

use alef::core::config::TraitBridgeConfig;
use alef::core::ir::{MethodDef, PrimitiveType, ReceiverKind, TypeRef};
use alef::e2e::codegen::elixir::emit_test_backend;
use alef::e2e::fixture::Fixture;
use serde_json::json;

fn make_trait_bridge(trait_name: &str, super_trait: Option<&str>) -> TraitBridgeConfig {
    TraitBridgeConfig {
        trait_name: trait_name.to_string(),
        super_trait: super_trait.map(|s| s.to_string()),
        register_fn: Some(format!("register_{}", trait_name.to_lowercase())),
        ..Default::default()
    }
}

fn make_method(name: &str, return_type: TypeRef, has_default: bool) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        params: vec![],
        return_type,
        is_async: false,
        is_static: false,
        error_type: None,
        doc: String::new(),
        receiver: Some(ReceiverKind::Ref),
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: has_default,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

fn make_fixture(id: &str, input: serde_json::Value) -> Fixture {
    Fixture {
        id: id.to_string(),
        category: None,
        description: "test".to_string(),
        tags: vec![],
        skip: None,
        env: None,
        setup: Vec::new(),
        call: None,
        input,
        mock_response: None,
        source: String::new(),
        http: None,
        assertions: vec![],
        visitor: None,
        args: vec![],
        assertion_recipes: Vec::new(),
    }
}

#[test]
fn elixir_stub_emits_super_trait_name_and_initialize() {
    let bridge = make_trait_bridge("DocumentExtractor", Some("Plugin"));
    let extract_method = make_method("extract_bytes", TypeRef::Named("ProcessingResult".to_string()), false);
    let methods = vec![&extract_method];
    let fixture = make_fixture("my_extractor", json!({ "extractor": { "name": "test-extractor" } }));

    let emission = emit_test_backend(&bridge, &methods, &fixture, "", "");
    let output = format!("{}\n{}", emission.setup_block, emission.arg_expr);

    assert!(
        output.contains("def name, do: \"test-extractor\""),
        "must emit name() from super-trait, got:\n{output}"
    );

    assert!(
        output.contains("def initialize, do: :ok"),
        "must emit initialize() from super-trait, got:\n{output}"
    );

    assert!(
        output.contains("def extract_bytes"),
        "must emit extract_bytes() required method, got:\n{output}"
    );
}

#[test]
fn elixir_stub_emits_integer_methods_as_one() {
    let bridge = make_trait_bridge("VectorBackend", Some("Plugin"));
    let dimensions_method = make_method("dimensions", TypeRef::Primitive(PrimitiveType::U32), false);
    let embed_method = make_method("embed", TypeRef::Named("Vec<Vec<f32>>".to_string()), false);
    let methods = vec![&dimensions_method, &embed_method];
    let fixture = make_fixture("my_backend", json!({ "backend": { "name": "test-vector-backend" } }));

    let emission = emit_test_backend(&bridge, &methods, &fixture, "", "");
    let output = format!("{}\n{}", emission.setup_block, emission.arg_expr);

    assert!(
        output.contains("def dimensions, do: 1"),
        "must emit dimensions() returning 1, got:\n{output}"
    );

    assert!(
        output.contains("def initialize, do: :ok"),
        "must emit initialize() from super-trait, got:\n{output}"
    );

    assert!(
        output.contains("def embed"),
        "must emit embed() required method, got:\n{output}"
    );
}

#[test]
fn elixir_stub_emits_all_required_trait_methods() {
    let bridge = make_trait_bridge("ImageBackend", Some("Plugin"));
    let process_image = make_method("process_image", TypeRef::Named("OcrResult".to_string()), false);
    let supports_language = make_method("supports_language", TypeRef::Primitive(PrimitiveType::Bool), false);
    let backend_type = make_method("backend_type", TypeRef::Named("BackendType".to_string()), false);
    let methods = vec![&process_image, &supports_language, &backend_type];
    let fixture = make_fixture("my_image_backend", json!({ "backend": { "name": "test-backend" } }));

    let emission = emit_test_backend(&bridge, &methods, &fixture, "", "");
    let output = format!("{}\n{}", emission.setup_block, emission.arg_expr);

    assert!(
        output.contains("def process_image"),
        "must emit process_image(), got:\n{output}"
    );
    assert!(
        output.contains("def supports_language"),
        "must emit supports_language(), got:\n{output}"
    );
    assert!(
        output.contains("def backend_type"),
        "must emit backend_type(), got:\n{output}"
    );

    assert!(output.contains("def name, do:"), "must emit name(), got:\n{output}");
    assert!(
        output.contains("def initialize, do:"),
        "must emit initialize(), got:\n{output}"
    );
}

#[test]
fn elixir_stub_emits_on_exit_teardown_when_facade_and_unregister_fn_present() {
    let mut bridge = make_trait_bridge("OcrBackend", Some("Plugin"));
    bridge.unregister_fn = Some("unregister_ocr_backend".to_string());
    let process_image = make_method("process_image", TypeRef::Named("OcrResult".to_string()), false);
    let methods = vec![&process_image];
    let fixture = make_fixture(
        "register_ocr_backend_trait_bridge",
        json!({ "backend": { "name": "test-backend" } }),
    );

    let emission = emit_test_backend(&bridge, &methods, &fixture, "", "Xberg");

    assert!(
        emission
            .teardown_block
            .contains("on_exit(fn -> Xberg.unregister_ocr_backend(\"test-backend\") end)"),
        "must emit on_exit teardown calling the facade's unregister_fn with the fixture's \
         plugin name, got teardown_block:\n{}",
        emission.teardown_block
    );
}

#[test]
fn elixir_stub_emits_no_teardown_when_facade_module_empty() {
    let mut bridge = make_trait_bridge("OcrBackend", Some("Plugin"));
    bridge.unregister_fn = Some("unregister_ocr_backend".to_string());
    let process_image = make_method("process_image", TypeRef::Named("OcrResult".to_string()), false);
    let methods = vec![&process_image];
    let fixture = make_fixture(
        "register_ocr_backend_trait_bridge",
        json!({ "backend": { "name": "test-backend" } }),
    );

    let emission = emit_test_backend(&bridge, &methods, &fixture, "", "");

    assert!(
        emission.teardown_block.is_empty(),
        "must not emit teardown when facade_module is empty (opt-out), got:\n{}",
        emission.teardown_block
    );
}

#[test]
fn elixir_stub_emits_no_teardown_when_trait_bridge_has_no_unregister_fn() {
    let bridge = make_trait_bridge("DocumentExtractor", Some("Plugin")); // unregister_fn defaults to None
    let extract_method = make_method("extract_bytes", TypeRef::Named("ProcessingResult".to_string()), false);
    let methods = vec![&extract_method];
    let fixture = make_fixture("my_extractor", json!({ "extractor": { "name": "test-extractor" } }));

    let emission = emit_test_backend(&bridge, &methods, &fixture, "", "Xberg");

    assert!(
        emission.teardown_block.is_empty(),
        "must not emit a call to a nonexistent unregister function, got:\n{}",
        emission.teardown_block
    );
}
