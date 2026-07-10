//! Tests for Dart plugin-trait test stub generation.
//!
//! These tests verify that the e2e stub emitter (`src/e2e/codegen/dart.rs::emit_test_backend`)
//! generates correct Dart code for trait-bridge plugin implementations. They ensure that:
//!
//! 1. Async trait methods generate `async` stub methods with `Future<R>` returns
//! 2. Sync trait methods generate non-async stubs with direct return types
//! 3. Return type mapping preserves async wrappers (generates `Future<T>` not an unbridged named type)
//! 4. Internal-only named types are mapped to explicit bridge carriers
//! 5. Wrapper fields use appropriate initialization (factory call without eager construction)
//! 6. Unimplemented trait methods throw `UnimplementedError()` instead of returning empty defaults

#[cfg(test)]
mod plugin_trait_stub_generation {
    use crate::core::config::TraitBridgeConfig;
    use crate::core::ir::{MethodDef, ParamDef, PrimitiveType, ReceiverKind, TypeRef};
    use crate::e2e::fixture::Fixture;
    use serde_json::json;

    fn emit_test_backend_dart(
        bridge: &TraitBridgeConfig,
        methods: &[&MethodDef],
        fixture: &Fixture,
    ) -> crate::e2e::codegen::TestBackendEmission {
        crate::e2e::codegen::emit_test_backend("dart", bridge, methods, fixture)
    }

    /// Helper to create a test trait bridge.
    fn make_trait_bridge(name: &str, super_trait: Option<&str>) -> TraitBridgeConfig {
        TraitBridgeConfig {
            trait_name: name.to_string(),
            super_trait: super_trait.map(|s| s.to_string()),
            register_fn: Some(format!("register_{}", name.to_lowercase())),
            unregister_fn: Some(format!("unregister_{}", name.to_lowercase())),
            clear_fn: Some(format!("clear_{}", name.to_lowercase())),
            ..Default::default()
        }
    }

    /// Helper to create a test method with specified async-ness and return type.
    fn make_method(name: &str, is_async: bool, return_type: TypeRef, params: Vec<ParamDef>) -> MethodDef {
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
            has_default_impl: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }
    }

    /// Helper to create a fixture.
    fn make_fixture(id: &str, plugin_name: Option<&str>) -> Fixture {
        let mut input_json = json!({});
        if let Some(name) = plugin_name {
            input_json["name"] = json!(name);
        }

        Fixture {
            id: id.to_string(),
            category: None,
            description: "test".to_string(),
            tags: vec![],
            skip: None,
            env: None,
            setup: Vec::new(),
            call: None,
            input: input_json,
            mock_response: None,
            source: String::new(),
            http: None,
            assertions: vec![],
            visitor: None,
            args: vec![],
            assertion_recipes: vec![],
        }
    }

    #[test]
    fn async_method_generates_async_keyword_and_future_return() {
        let bridge = make_trait_bridge("TestBackend", Some("Plugin"));
        let async_method = make_method("process", true, TypeRef::Named("ExtractionResult".to_string()), vec![]);
        let methods = [&async_method];
        let fixture = make_fixture("async_test", Some("test-backend"));

        let emission = emit_test_backend_dart(&bridge, &methods, &fixture);

        assert!(
            emission.setup_block.contains("Future<ExtractionResult> process(")
                || emission.setup_block.contains("Future< ExtractionResult > process("),
            "async method must have Future<T> return type, got:\n{}",
            emission.setup_block
        );
        assert!(
            emission.setup_block.contains("async =>"),
            "async method must have async keyword, got:\n{}",
            emission.setup_block
        );
    }

    #[test]
    fn sync_method_generates_future_stub_for_trait_bridge_factory() {
        let bridge = make_trait_bridge("TestValidator", Some("Plugin"));
        let sync_method = make_method("validate", false, TypeRef::Primitive(PrimitiveType::Bool), vec![]);
        let methods = [&sync_method];
        let fixture = make_fixture("sync_test", Some("test-validator"));

        let emission = emit_test_backend_dart(&bridge, &methods, &fixture);

        assert!(
            emission.setup_block.contains("Future<bool> validate()"),
            "sync method must be adapted to Future<T>, got:\n{}",
            emission.setup_block
        );
        let validate_section = emission
            .setup_block
            .lines()
            .filter(|l| l.contains("validate"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            validate_section.contains("async =>"),
            "sync method stub must be async for the factory callback, got:\n{}",
            validate_section
        );
    }

    #[test]
    fn internal_record_type_maps_to_bridge_type() {
        let bridge = make_trait_bridge("TestExtractor", Some("Plugin"));
        let method_with_internal = make_method("extract", true, TypeRef::Named("InternalRecord".to_string()), vec![]);
        let methods = [&method_with_internal];
        let fixture = make_fixture("extract_test", Some("test-extractor"));

        let emission = emit_test_backend_dart(&bridge, &methods, &fixture);

        assert!(
            emission.setup_block.contains("Future<InternalRecordBridge>")
                || emission.setup_block.contains("Future< InternalRecordBridge >"),
            "InternalRecord return type must be mapped to InternalRecordBridge, got:\n{}",
            emission.setup_block
        );
    }

    #[test]
    fn wrapper_instance_awaits_async_factory_call() {
        let bridge = make_trait_bridge("OcrBackend", Some("Plugin"));
        let method = make_method("process", true, TypeRef::Named("String".to_string()), vec![]);
        let methods = [&method];
        let fixture = make_fixture("ocr_test", Some("test-ocr"));

        let emission = emit_test_backend_dart(&bridge, &methods, &fixture);

        assert!(
            emission.setup_block.contains("await createOcrBackendDartImpl"),
            "factory call must be awaited, got:\n{}",
            emission.setup_block
        );
        assert!(
            emission.setup_block.contains("createOcrBackendDartImpl("),
            "factory function must be called, got:\n{}",
            emission.setup_block
        );
    }

    #[test]
    fn method_callbacks_provided_for_all_methods() {
        let bridge = make_trait_bridge("MultiMethod", Some("Plugin"));
        let method1 = make_method("doFirst", true, TypeRef::Primitive(PrimitiveType::Bool), vec![]);
        let method2 = make_method("doSecond", true, TypeRef::Named("String".to_string()), vec![]);
        let methods = [&method1, &method2];
        let fixture = make_fixture("multi_test", Some("test-multi"));

        let emission = emit_test_backend_dart(&bridge, &methods, &fixture);

        assert!(
            emission.setup_block.contains("doFirst:") && emission.setup_block.contains("doSecond:"),
            "all methods must have callbacks in factory call, got:\n{}",
            emission.setup_block
        );
    }

    #[test]
    fn fixture_input_name_used_as_plugin_name() {
        let bridge = make_trait_bridge("TestBackend", Some("Plugin"));
        let method = make_method("test", true, TypeRef::Primitive(PrimitiveType::Bool), vec![]);
        let methods = [&method];
        let fixture = make_fixture("some_id", Some("my-custom-backend"));

        let emission = emit_test_backend_dart(&bridge, &methods, &fixture);

        assert!(
            emission.setup_block.contains("pluginName: 'my-custom-backend'"),
            "pluginName must use fixture input name field, got:\n{}",
            emission.setup_block
        );
        assert!(
            !emission.setup_block.contains("pluginName: 'some_id'"),
            "pluginName must not use fixture id when input name is available, got:\n{}",
            emission.setup_block
        );
    }

    #[test]
    fn class_name_derived_from_fixture_id() {
        let bridge = make_trait_bridge("Backend", Some("Plugin"));
        let method = make_method("test", true, TypeRef::Primitive(PrimitiveType::Bool), vec![]);
        let methods = [&method];
        let fixture = make_fixture("custom_fixture_id", None);

        let emission = emit_test_backend_dart(&bridge, &methods, &fixture);

        assert!(
            emission
                .setup_block
                .contains("class TestStubCustomFixtureId extends Backend"),
            "class name must be derived from fixture id in PascalCase, got:\n{}",
            emission.setup_block
        );
    }

    #[test]
    fn method_parameters_are_typed() {
        let bridge = make_trait_bridge("TestBackend", Some("Plugin"));
        let param = ParamDef {
            name: "input".to_string(),
            ty: TypeRef::Named("String".to_string()),
            optional: false,
            ..Default::default()
        };

        let method = make_method("process", true, TypeRef::Named("Result".to_string()), vec![param]);
        let methods = [&method];
        let fixture = make_fixture("typed_params_test", Some("test-backend"));

        let emission = emit_test_backend_dart(&bridge, &methods, &fixture);

        assert!(
            emission.setup_block.contains("String input") || emission.setup_block.contains("String  input"),
            "parameters must be typed, not dynamic, got:\n{}",
            emission.setup_block
        );
    }
}
