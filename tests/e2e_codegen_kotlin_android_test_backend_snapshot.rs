//! Snapshot tests for the Kotlin Android e2e test-backend stub emitter.
//!
//! Verifies that `emit_test_backend` produces Kotlin code that:
//! - Implements the interface derived from the trait name (e.g., I{TraitName})
//! - Uses concrete Kotlin types for parameters and return types (no `Any`)
//! - Emits async methods with `suspend fun` keyword
//! - Returns defaults via `language_defaults.emit_default()`

use alef::core::config::TraitBridgeConfig;
use alef::core::ir::{MethodDef, ParamDef, PrimitiveType, ReceiverKind, TypeRef};
use alef::e2e::codegen::kotlin_android::emit_test_backend;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_param(name: &str, ty: TypeRef) -> ParamDef {
    ParamDef {
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
        core_wrapper: alef::core::ir::CoreWrapper::None,
    }
}

fn make_method(
    name: &str,
    params: Vec<ParamDef>,
    return_type: TypeRef,
    is_async: bool,
    has_default_impl: bool,
) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        params,
        return_type,
        is_async,
        is_static: false,
        error_type: None,
        doc: String::new(),
        receiver: Some(ReceiverKind::Ref),
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

fn make_fixture(id: &str) -> alef::e2e::fixture::Fixture {
    serde_json::from_value(serde_json::json!({
        "id": id,
        "description": "snapshot fixture",
        "input": { "name": "test-stub" },
        "assertions": []
    }))
    .expect("fixture JSON must parse")
}

// ---------------------------------------------------------------------------
// Snapshot tests
// ---------------------------------------------------------------------------

/// Verify that a stub class implements the interface derived from trait name
/// (e.g., IDocumentExtractor from DocumentExtractor) and emits typed parameters.
#[test]
fn snapshot_emit_test_backend_implements_interface() {
    let bridge = TraitBridgeConfig {
        trait_name: "DocumentExtractor".to_string(),
        super_trait: Some("Plugin".to_string()),
        ..Default::default()
    };

    // async fn extract_bytes(&self, content: ByteArray, mime_type: String) -> ProcessingResult
    let extract = make_method(
        "extract_bytes",
        vec![
            make_param("content", TypeRef::Bytes),
            make_param("mime_type", TypeRef::String),
        ],
        TypeRef::Named("ProcessingResult".to_string()),
        true,
        false,
    );

    let methods = [&extract];
    let fixture = make_fixture("test_extractor");
    let emission = emit_test_backend(&bridge, &methods, &fixture);

    // Must implement the interface derived from trait name.
    assert!(
        emission
            .setup_block
            .contains("class TestStubTestExtractor : IDocumentExtractor"),
        "class must implement interface derived from trait name, got:\n{}",
        emission.setup_block
    );

    // Must emit the name() override for Plugin super-trait.
    assert!(
        emission
            .setup_block
            .contains("override fun name(): String = \"test-stub\""),
        "must emit name() override for Plugin super-trait, got:\n{}",
        emission.setup_block
    );

    // Method must be async (suspend fun).
    assert!(
        emission.setup_block.contains("override suspend fun extractBytes("),
        "async method must use `suspend fun`, got:\n{}",
        emission.setup_block
    );

    // Parameters must have concrete types, not Any.
    assert!(
        emission.setup_block.contains("content: ByteArray"),
        "ByteArray param must be concrete, got:\n{}",
        emission.setup_block
    );
    assert!(
        emission.setup_block.contains("mimeType: String"),
        "String param must be concrete, got:\n{}",
        emission.setup_block
    );

    // Return type must be concrete.
    assert!(
        emission.setup_block.contains("): ProcessingResult"),
        "return type must be concrete not Any, got:\n{}",
        emission.setup_block
    );

    // arg_expr should be a constructor call.
    assert_eq!(
        emission.arg_expr, "TestStubTestExtractor()",
        "arg_expr must be class constructor"
    );
}

/// Verify that a sync non-Plugin trait (no super_trait) does not emit name().
#[test]
fn snapshot_emit_test_backend_no_super_trait_no_name_method() {
    let bridge = TraitBridgeConfig {
        trait_name: "OcrBackend".to_string(),
        super_trait: None,
        ..Default::default()
    };

    // fn recognize(&self, image: ByteArray) -> String
    let recognize = make_method(
        "recognize",
        vec![make_param("image", TypeRef::Bytes)],
        TypeRef::String,
        false,
        false,
    );

    let methods = [&recognize];
    let fixture = make_fixture("my_ocr_backend");
    let emission = emit_test_backend(&bridge, &methods, &fixture);

    // Must NOT emit name() when no super_trait.
    assert!(
        !emission.setup_block.contains("override fun name()"),
        "must not emit name() when no super_trait, got:\n{}",
        emission.setup_block
    );

    // Method must be sync (no suspend).
    assert!(
        emission.setup_block.contains("override fun recognize("),
        "sync method must not use suspend fun, got:\n{}",
        emission.setup_block
    );

    // Return type must be concrete.
    assert!(
        emission.setup_block.contains("): String"),
        "String return type must be concrete, got:\n{}",
        emission.setup_block
    );
}

/// Verify that methods with default implementations are still emitted.
#[test]
fn snapshot_emit_test_backend_emits_default_impl_methods() {
    let bridge = TraitBridgeConfig {
        trait_name: "MyBackend".to_string(),
        super_trait: Some("Plugin".to_string()),
        ..Default::default()
    };

    // fn required_method(&self) -> String — no default
    let required = make_method("required_method", vec![], TypeRef::String, false, false);

    // fn optional_method(&self) — has default impl, but the generated interface
    // still requires a concrete implementation.
    let optional = make_method(
        "optional_method",
        vec![],
        TypeRef::Unit,
        false,
        true, // has_default_impl = true
    );

    let methods = [&required, &optional];
    let fixture = make_fixture("my_fixture");
    let emission = emit_test_backend(&bridge, &methods, &fixture);

    // Must emit the required method.
    assert!(
        emission.setup_block.contains("override fun requiredMethod()"),
        "required method must be emitted, got:\n{}",
        emission.setup_block
    );

    // Must also emit the optional method.
    assert!(
        emission.setup_block.contains("override fun optionalMethod()"),
        "optional method must be emitted, got:\n{}",
        emission.setup_block
    );
}

/// Verify that fixture input["name"] overrides fixture id for the plugin name.
#[test]
fn snapshot_emit_test_backend_uses_fixture_input_name() {
    let bridge = TraitBridgeConfig {
        trait_name: "ValidatorBackend".to_string(),
        super_trait: Some("Plugin".to_string()),
        ..Default::default()
    };

    let validate = make_method(
        "validate",
        vec![],
        TypeRef::Primitive(PrimitiveType::Bool),
        false,
        false,
    );

    let methods = [&validate];
    let mut fixture = make_fixture("internal_fixture_id");
    // Override the plugin name via fixture input.
    fixture.input = serde_json::json!({ "name": "custom-backend-name" });

    let emission = emit_test_backend(&bridge, &methods, &fixture);

    // Must use the input["name"] value, not the fixture id.
    assert!(
        emission
            .setup_block
            .contains("override fun name(): String = \"custom-backend-name\""),
        "must use fixture.input[\"name\"], got:\n{}",
        emission.setup_block
    );

    // arg_expr uses the fixture id for the class name.
    assert!(
        emission.arg_expr.contains("TestStubInternalFixtureId"),
        "class name must derive from fixture id, got: {}",
        emission.arg_expr
    );
}
