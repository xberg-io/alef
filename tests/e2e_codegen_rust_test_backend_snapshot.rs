//! Snapshot tests for the Rust e2e test-backend stub emitter.
//!
//! Verifies that `emit_test_backend` produces compilable Rust: typed parameter
//! annotations, explicit return-type arrows, and `type_imports` carrying the trait
//! name plus any named types referenced by method signatures.

use alef::core::config::TraitBridgeConfig;
use alef::core::ir::{MethodDef, ParamDef, PrimitiveType, ReceiverKind, TypeRef};
use alef::e2e::codegen::rust::emit_test_backend;

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
    error_type: Option<&str>,
    has_default_impl: bool,
) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        params,
        return_type,
        is_async,
        is_static: false,
        error_type: error_type.map(str::to_string),
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

/// One async method returning `Result<T, E>`, one sync unit method, one sync
/// `Vec<T>` method — covers all three basic return-type shapes.
#[test]
fn snapshot_emit_test_backend_mixed_return_types() {
    let bridge = TraitBridgeConfig {
        trait_name: "MyTrait".to_string(),
        super_trait: Some("Plugin".to_string()),
        ..Default::default()
    };

    let extract = make_method(
        "extract",
        vec![
            make_param("content", TypeRef::Bytes),
            make_param("mime_type", TypeRef::String),
        ],
        TypeRef::Named("ProcessingResult".to_string()),
        true,
        Some("SampleCrateError"),
        false,
    );

    let initialize = make_method("initialize", vec![], TypeRef::Unit, false, None, false);

    let supported = make_method(
        "supported_mime_types",
        vec![],
        TypeRef::Vec(Box::new(TypeRef::String)),
        false,
        None,
        false,
    );

    let methods = [&extract, &initialize, &supported];
    let fixture = make_fixture("mixed_return_types");
    let emission = emit_test_backend(&bridge, &methods, &fixture);

    insta::assert_snapshot!("snapshot_emit_test_backend_setup_block", &emission.setup_block);

    assert!(
        emission.type_imports.contains(&"MyTrait".to_string()),
        "type_imports must include the trait name, got: {:?}",
        emission.type_imports
    );
    assert!(
        emission.type_imports.contains(&"ProcessingResult".to_string()),
        "type_imports must include ProcessingResult, got: {:?}",
        emission.type_imports
    );
    assert!(
        emission.type_imports.contains(&"SampleCrateError".to_string()),
        "type_imports must include SampleCrateError, got: {:?}",
        emission.type_imports
    );
    assert!(
        !emission.type_imports.contains(&"String".to_string()),
        "String must not appear in type_imports (always in scope)"
    );
    assert!(
        !emission.type_imports.contains(&"Vec".to_string()),
        "Vec must not appear in type_imports (always in scope)"
    );
}

/// Verify that param type annotations are emitted correctly for primitive params.
#[test]
fn snapshot_emit_test_backend_typed_params() {
    let bridge = TraitBridgeConfig {
        trait_name: "OcrTrait".to_string(),
        ..Default::default()
    };

    let process = make_method(
        "process_image",
        vec![
            make_param("image_bytes", TypeRef::Bytes),
            make_param("grayscale", TypeRef::Primitive(PrimitiveType::Bool)),
        ],
        TypeRef::String,
        false,
        Some("String"),
        false,
    );

    let methods = [&process];
    let fixture = make_fixture("typed_params_fixture");
    let emission = emit_test_backend(&bridge, &methods, &fixture);

    assert!(
        emission.setup_block.contains("_p0: Vec<u8>"),
        "param 0 must have type annotation Vec<u8>, got:\n{}",
        emission.setup_block
    );
    assert!(
        emission.setup_block.contains("_p1: bool"),
        "param 1 must have type annotation bool, got:\n{}",
        emission.setup_block
    );

    assert!(
        emission.setup_block.contains("-> Result<String, String>"),
        "must emit -> Result<String, String>, got:\n{}",
        emission.setup_block
    );
}

/// Verify that a sync unit-returning method emits no return arrow.
#[test]
fn snapshot_emit_test_backend_unit_return_no_arrow() {
    let bridge = TraitBridgeConfig {
        trait_name: "ValidatorTrait".to_string(),
        ..Default::default()
    };

    let validate = make_method("validate", vec![], TypeRef::Unit, false, None, false);
    let methods = [&validate];
    let fixture = make_fixture("unit_return_fixture");
    let emission = emit_test_backend(&bridge, &methods, &fixture);

    assert!(
        !emission.setup_block.contains("->"),
        "unit-returning method must not emit a return arrow, got:\n{}",
        emission.setup_block
    );

    assert!(
        emission.setup_block.contains("{ () }"),
        "unit method body must be `{{ () }}`, got:\n{}",
        emission.setup_block
    );
}

/// Verify async fn keyword is emitted and arg_expr uses Arc::new.
#[test]
fn snapshot_emit_test_backend_async_method() {
    let bridge = TraitBridgeConfig {
        trait_name: "AsyncTrait".to_string(),
        ..Default::default()
    };

    let embed = make_method(
        "embed",
        vec![make_param("text", TypeRef::String)],
        TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::F32))),
        true,
        Some("String"),
        false,
    );

    let methods = [&embed];
    let fixture = make_fixture("async_method_fixture");
    let emission = emit_test_backend(&bridge, &methods, &fixture);

    assert!(
        emission.setup_block.contains("async fn embed("),
        "must emit `async fn embed(`, got:\n{}",
        emission.setup_block
    );
    assert!(
        emission.setup_block.contains("-> Result<Vec<f32>, String>"),
        "must emit return type `-> Result<Vec<f32>, String>`, got:\n{}",
        emission.setup_block
    );
    assert!(
        emission.arg_expr.contains("Arc::new"),
        "arg_expr must use Arc::new, got: {}",
        emission.arg_expr
    );
}
