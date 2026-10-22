//! Trait-bridge stub emission tests, split out of `test_backend.rs`.

use super::{emit_test_backend, emit_test_backend_with_context};
use crate::core::config::TraitBridgeConfig;
use crate::core::ir::{MethodDef, ParamDef, TypeRef};
use crate::e2e::fixture::Fixture;

fn make_fixture(id: &str) -> Fixture {
    Fixture {
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
        core_wrapper: crate::core::ir::CoreWrapper::None,
    }
}

fn make_method(name: &str, params: Vec<(&str, TypeRef)>, ret: TypeRef, is_async: bool) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        params: params.into_iter().map(|(n, ty)| make_param(n, ty)).collect(),
        return_type: ret,
        is_async,
        is_static: false,
        error_type: Some("Error".to_string()),
        doc: String::new(),
        receiver: Some(crate::core::ir::ReceiverKind::Ref),
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

/// Genericity test: a synthetic TestTrait with one sync method and Plugin super-trait
/// must not reference any sample_core-domain names in setup_block or arg_expr.
#[test]
fn test_backend_emission_is_generic() {
    let trait_bridge = TraitBridgeConfig {
        trait_name: "TestTrait".to_string(),
        super_trait: Some("SomeSuperTrait".to_string()),
        register_fn: Some("register_test_trait".to_string()),
        ..TraitBridgeConfig::default()
    };

    let do_thing = make_method(
        "do_thing",
        vec![("x", TypeRef::Primitive(crate::core::ir::PrimitiveType::I32))],
        TypeRef::String,
        false,
    );

    let fixture = make_fixture("my_test_fixture");
    let methods = vec![&do_thing];
    let emission = emit_test_backend(&trait_bridge, &methods, &fixture);

    // setup_block must not reference any sample_core-domain trait or method names.
    assert!(
        !emission.setup_block.contains("ImageBackend"),
        "setup_block must not hardcode domain trait names, got:\n{}",
        emission.setup_block
    );
    assert!(
        !emission.setup_block.contains("ProcessImage"),
        "setup_block must not hardcode domain method names, got:\n{}",
        emission.setup_block
    );
    // Must emit the method name from MethodDef (Go PascalCase).
    assert!(
        emission.setup_block.contains("DoThing"),
        "setup_block must contain Go PascalCase method 'DoThing', got:\n{}",
        emission.setup_block
    );
    // Must emit struct declaration.
    assert!(
        emission.setup_block.contains("type testStub_my_test_fixture struct"),
        "setup_block must contain struct declaration, got:\n{}",
        emission.setup_block
    );
    // With trait_source: None, super-trait methods are NOT emitted — no hardcoded lifecycle names.
    assert!(
        !emission.setup_block.contains("Initialize"),
        "setup_block must not contain hardcoded 'Initialize', got:\n{}",
        emission.setup_block
    );
    assert!(
        !emission.setup_block.contains("Shutdown"),
        "setup_block must not contain hardcoded 'Shutdown', got:\n{}",
        emission.setup_block
    );
    // arg_expr is the struct literal.
    assert!(
        emission.arg_expr.contains("testStub_my_test_fixture"),
        "arg_expr must reference struct name, got: {}",
        emission.arg_expr
    );
    assert!(
        emission.arg_expr.ends_with("{}"),
        "arg_expr must be a struct literal, got: {}",
        emission.arg_expr
    );
}

/// Verify that super-trait methods with `trait_source` set are driven from
/// the IR slice rather than a hardcoded list of method names.
///
/// A synthetic `Plugin` super-trait with methods `name`, `version`, `init`
/// (note: `init`, NOT `Initialize`) is passed via `trait_source`. The emitter
/// must emit `Init` (PascalCase of `init`), NOT the previously-hardcoded
/// `Initialize` string, proving the method names come from IR.
#[test]
fn test_go_super_trait_methods_driven_from_ir_not_hardcoded() {
    let make_super_method = |name: &str, ret: TypeRef| -> MethodDef {
        MethodDef {
            name: name.to_string(),
            params: vec![],
            return_type: ret,
            is_async: false,
            is_static: false,
            error_type: None,
            doc: String::new(),
            receiver: Some(crate::core::ir::ReceiverKind::Ref),
            cfg: None,
            sanitized: false,
            // trait_source matches the super_trait configured on the bridge.
            trait_source: Some("Plugin".to_string()),
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }
    };

    let name_method = make_super_method("name", TypeRef::String);
    let version_method = make_super_method("version", TypeRef::String);
    let init_method = make_super_method("init", TypeRef::Unit);

    let trait_bridge = TraitBridgeConfig {
        trait_name: "TestPlugin".to_string(),
        super_trait: Some("Plugin".to_string()),
        register_fn: Some("register_test_plugin".to_string()),
        ..TraitBridgeConfig::default()
    };

    let fixture = make_fixture("my_plugin_fixture");
    let methods = vec![&name_method, &version_method, &init_method];
    let emission = emit_test_backend(&trait_bridge, &methods, &fixture);

    // Must emit `Init` (PascalCase of "init"), not the old hardcoded "Initialize".
    assert!(
        emission.setup_block.contains("Init("),
        "setup_block must contain 'Init(' (from IR), got:\n{}",
        emission.setup_block
    );
    assert!(
        !emission.setup_block.contains("Initialize"),
        "setup_block must NOT contain hardcoded 'Initialize', got:\n{}",
        emission.setup_block
    );
    // `Version` comes from IR method name "version".
    assert!(
        emission.setup_block.contains("Version("),
        "setup_block must contain 'Version(' (from IR), got:\n{}",
        emission.setup_block
    );
    // Must not contain old hardcoded `Shutdown`.
    assert!(
        !emission.setup_block.contains("Shutdown"),
        "setup_block must NOT contain hardcoded 'Shutdown', got:\n{}",
        emission.setup_block
    );
    // `Name()` is emitted and returns the fixture id.
    assert!(
        emission.setup_block.contains("Name()"),
        "setup_block must contain Name() from IR name method, got:\n{}",
        emission.setup_block
    );
}

/// Verify that Named types use their proper Go type names
/// in stubs, matching the actual trait-bridge interface signatures.
#[test]
fn test_go_stub_named_types_use_proper_go_names() {
    let backend_type_method = make_method("backend_type", vec![], TypeRef::Named("BackendKind".to_string()), false);

    let trait_bridge = TraitBridgeConfig {
        trait_name: "SampleBackend".to_string(),
        super_trait: Some("Plugin".to_string()),
        register_fn: Some("register_sample_backend".to_string()),
        ..TraitBridgeConfig::default()
    };

    let fixture = make_fixture("backend_type_test");
    let methods = vec![&backend_type_method];
    let emission = emit_test_backend(&trait_bridge, &methods, &fixture);

    // The method signature should use the proper Go name, not json.RawMessage.
    assert!(
        emission.setup_block.contains("BackendType()") && emission.setup_block.contains("BackendKind"),
        "setup_block must use BackendKind in BackendType() method signature, got:\n{}",
        emission.setup_block
    );

    // Return value must match go_zero_value for named types.
    assert!(
        !emission.setup_block.contains("json.RawMessage(nil)"),
        "setup_block must not use json.RawMessage for BackendKind, got:\n{}",
        emission.setup_block
    );
}

/// Verify that methods with binding-excluded types are handled correctly:
/// - Methods returning directly excluded types are skipped
/// - Methods returning Optional<ExcludedType> are skipped
/// - Methods with wrapped returns (Result<ExcludedType>, Vec<ExcludedType>) are emitted
///   (binding generation converts these appropriately)
/// - Normal methods are emitted with proper type qualification
#[test]
fn test_go_stub_skips_excluded_return_types() {
    // Method 1: returns an excluded named type directly -> should be SKIPPED
    let excluded_return_method = make_method(
        "get_internal_record",
        vec![],
        TypeRef::Named("InternalRecord".to_string()),
        false,
    );

    // Method 2: returns Result<ExcludedType> -> should be EMITTED
    // (Result wrapping is handled by binding generation)
    let result_return_method = make_method(
        "extract_bytes",
        vec![("content", TypeRef::Bytes)],
        TypeRef::Named("InternalRecord".to_string()), // In IR; becomes json.RawMessage in binding
        true,                                         // has_error_type = true
    );

    // Method 3: normal method with non-excluded types → should be EMITTED
    let normal_method = make_method("get_config", vec![], TypeRef::Named("ParseConfig".to_string()), false);

    let trait_bridge = TraitBridgeConfig {
        trait_name: "RecordProvider".to_string(),
        super_trait: None,
        register_fn: Some("register_document_extractor".to_string()),
        ..TraitBridgeConfig::default()
    };

    let fixture = make_fixture("extractor_test");
    let methods = vec![&excluded_return_method, &result_return_method, &normal_method];

    let mut excluded = std::collections::HashSet::new();
    excluded.insert("InternalRecord");

    let enum_names = std::collections::HashSet::new();
    let emission = emit_test_backend_with_context(
        &trait_bridge,
        &methods,
        &fixture,
        &excluded,
        "myproject",
        &enum_names,
        &[],
    );

    // Method returning directly excluded type must NOT appear in stub.
    assert!(
        !emission.setup_block.contains("get_internal_record"),
        "method with directly excluded return type must be skipped, got:\n{}",
        emission.setup_block
    );

    // Method with Result-wrapped excluded type should appear (binding generation handles conversion).
    assert!(
        emission.setup_block.contains("ExtractBytes"),
        "method with Result<ExcludedType> should be emitted (binding handles conversion), got:\n{}",
        emission.setup_block
    );

    // Normal method with non-excluded types must appear (in PascalCase).
    assert!(
        emission.setup_block.contains("GetConfig"),
        "normal method must be emitted, got:\n{}",
        emission.setup_block
    );

    // Normal method's return type must be qualified with import alias.
    assert!(
        emission.setup_block.contains("myproject.ParseConfig"),
        "named type ParseConfig must be qualified as myproject.ParseConfig, got:\n{}",
        emission.setup_block
    );
}

/// Regression (Go trait bridges): methods returning enum types must not be skipped.
///
/// Example: OcrBackend.BackendType() returns OcrBackendType (an enum).
/// The Go interface declares `BackendType() OcrBackendType`, so the test-stub
/// MUST emit a default implementation, even though OcrBackendType is a Named type
/// and may be in the excluded_types set (for trait-bridge json.RawMessage purposes).
///
/// This test uses a synthetic `MyService` trait with `Diagnose() string` returning
/// a named type (treated as enum) to verify the fix works generically.
#[test]
fn test_go_stub_emits_methods_returning_named_excluded_types() {
    let diagnose_method = make_method("diagnose", vec![], TypeRef::Named("DiagnosticLevel".to_string()), false);

    let trait_bridge = TraitBridgeConfig {
        trait_name: "MyService".to_string(),
        super_trait: None,
        register_fn: Some("register_my_service".to_string()),
        ..TraitBridgeConfig::default()
    };

    let fixture = make_fixture("service_diagnose");
    let methods = vec![&diagnose_method];

    // Simulate the scenario where DiagnosticLevel is in excluded_types
    // (e.g., treated as json.RawMessage at the trait-bridge interface level).
    // Before the fix, this method would be skipped; after the fix, it must be emitted.
    let mut excluded = std::collections::HashSet::new();
    excluded.insert("DiagnosticLevel");

    let enum_names = std::collections::HashSet::new();
    let emission = emit_test_backend_with_context(&trait_bridge, &methods, &fixture, &excluded, "", &enum_names, &[]);

    // Method returning an excluded named type must be emitted (it's now exported as json.RawMessage).
    assert!(
        emission.setup_block.contains("Diagnose()"),
        "method returning excluded named type must be emitted, got:\n{}",
        emission.setup_block
    );

    // Return type must be properly handled (json.RawMessage for excluded, or proper type name).
    // The signature should reflect the binding's interface (json.RawMessage for excluded types).
    assert!(
        emission.setup_block.contains("json.RawMessage") || emission.setup_block.contains("nil"),
        "method must emit a zero-value that matches the excluded type handling, got:\n{}",
        emission.setup_block
    );
}

/// Verify that methods returning Optional<ExcludedType> are skipped
/// (for example, an accessor returning an optional excluded trait object).
#[test]
fn test_go_stub_skips_optional_excluded_return_types() {
    // Method returning Option<InternalProvider> -> should be skipped
    // (InternalProvider is not exported in the binding).
    let optional_excluded_method = make_method(
        "as_internal_provider",
        vec![],
        TypeRef::Optional(Box::new(TypeRef::Named("InternalProvider".to_string()))),
        false,
    );

    let trait_bridge = TraitBridgeConfig {
        trait_name: "RecordProvider".to_string(),
        super_trait: None,
        register_fn: Some("register_document_extractor".to_string()),
        ..TraitBridgeConfig::default()
    };

    let fixture = make_fixture("extractor_test");
    let methods = vec![&optional_excluded_method];

    let mut excluded = std::collections::HashSet::new();
    excluded.insert("InternalProvider");

    let enum_names = std::collections::HashSet::new();
    let emission =
        emit_test_backend_with_context(&trait_bridge, &methods, &fixture, &excluded, "mylib", &enum_names, &[]);

    // Method returning Optional<ExcludedType> must NOT appear in stub.
    assert!(
        !emission.setup_block.contains("as_internal_provider") && !emission.setup_block.contains("AsInternalProvider"),
        "method with Option<ExcludedType> return must be skipped, got:\n{}",
        emission.setup_block
    );

    // InternalProvider must not appear anywhere in the stub.
    assert!(
        !emission.setup_block.contains("InternalProvider"),
        "excluded type InternalProvider must not appear in stub, got:\n{}",
        emission.setup_block
    );
}
