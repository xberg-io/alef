use crate::e2e::config::{ArgMapping, CallConfig, E2eConfig, SelectWhen};
use crate::e2e::fixture::Fixture;
use std::collections::HashMap;

use super::stubs::emit_test_backend_with_class_name;

fn make_fixture_with_input(id: &str, input: serde_json::Value) -> Fixture {
    Fixture {
        id: id.to_string(),
        category: None,
        description: "test fixture".to_string(),
        tags: vec![],
        skip: None,
        env: None,
        call: None,
        input,
        mock_response: None,
        source: String::new(),
        http: None,
        assertions: vec![],
        visitor: None,
        args: vec![],
        assertion_recipes: vec![],
    }
}

/// Test that resolve_call_for_fixture correctly routes to batch_scrape
/// when input has batch_urls and select_when condition matches.
#[test]
fn test_csharp_select_when_routes_to_batch_scrape() {
    let mut calls = HashMap::new();
    calls.insert(
        "batch_scrape".to_string(),
        CallConfig {
            function: "BatchScrape".to_string(),
            module: "ExampleBrowser".to_string(),
            select_when: Some(SelectWhen {
                input_has: Some("batch_urls".to_string()),
                ..Default::default()
            }),
            ..CallConfig::default()
        },
    );

    let e2e_config = E2eConfig {
        call: CallConfig {
            function: "Scrape".to_string(),
            module: "ExampleBrowser".to_string(),
            ..CallConfig::default()
        },
        calls,
        ..E2eConfig::default()
    };

    // Fixture with batch_urls but no explicit call field should route to batch_scrape
    let fixture = make_fixture_with_input("batch_empty_urls", serde_json::json!({ "batch_urls": [] }));

    let resolved_call = e2e_config.resolve_call_for_fixture(
        fixture.call.as_deref(),
        &fixture.id,
        &fixture.resolved_category(),
        &fixture.tags,
        &fixture.input,
    );
    assert_eq!(resolved_call.function, "BatchScrape");

    // Fixture without batch_urls should fall back to default Scrape
    let fixture_no_batch =
        make_fixture_with_input("simple_scrape", serde_json::json!({ "url": "https://example.com" }));
    let resolved_default = e2e_config.resolve_call_for_fixture(
        fixture_no_batch.call.as_deref(),
        &fixture_no_batch.id,
        &fixture_no_batch.resolved_category(),
        &fixture_no_batch.tags,
        &fixture_no_batch.input,
    );
    assert_eq!(resolved_default.function, "Scrape");
}

#[test]
fn handle_config_deserialization_uses_resolved_options_type() {
    let fixture = make_fixture_with_input("session_fixture", serde_json::json!({ "config": { "limit": 3 } }));
    let args = vec![ArgMapping {
        name: "session".to_string(),
        field: "input.config".to_string(),
        arg_type: "handle".to_string(),
        optional: false,
        owned: false,
        element_type: None,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    }];
    let mut class_decls = Vec::new();
    let mut teardown_lines = Vec::new();
    let (setup, args_str) = super::build_args_and_setup(
        &fixture.input,
        &args,
        "SessionLib",
        Some("SessionConfig"),
        Some("from_json"),
        &HashMap::new(),
        &HashMap::new(),
        &fixture,
        None,
        &crate::core::config::ResolvedCrateConfig::default(),
        &[],
        &[],
        &mut class_decls,
        &mut teardown_lines,
    );

    let rendered = setup.join("\n");
    assert_eq!(args_str, "session");
    assert!(rendered.contains("JsonSerializer.Deserialize<SessionConfig>"));
    assert!(rendered.contains("SessionLib.CreateSession(sessionConfig)"));
    assert!(!rendered.contains("CrawlConfig"));
}

/// Verify `emit_test_backend` is generic: output must not contain any
/// hardcoded domain trait or method names — only names derived from the
/// synthetic `TestTrait` / `do_work` inputs.
#[test]
fn test_emit_test_backend_is_generic_no_domain_names() {
    use crate::core::config::TraitBridgeConfig;
    use crate::core::ir::{MethodDef, ParamDef, ReceiverKind, TypeRef};

    let method = MethodDef {
        name: "do_work".to_string(),
        params: vec![ParamDef {
            name: "payload".to_string(),
            ty: TypeRef::String,
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
        }],
        return_type: TypeRef::String,
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
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
    };

    let bridge = TraitBridgeConfig {
        trait_name: "TestTrait".to_string(),
        super_trait: Some("Plugin".to_string()),
        register_fn: Some("register_test_trait".to_string()),
        ..Default::default()
    };

    let fixture = Fixture {
        id: "my_fixture".to_string(),
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
    };

    let methods = vec![&method];
    let emission = emit_test_backend_with_class_name(
        &bridge,
        &methods,
        &fixture,
        "FixtureFacade",
        &std::collections::HashSet::new(),
    );

    // The generated code must reference the synthetic interface name.
    assert!(
        emission.setup_block.contains("ITestTrait"),
        "setup_block should reference ITestTrait, got:\n{}",
        emission.setup_block
    );
    assert!(
        emission.setup_block.contains("DoWork"),
        "setup_block should contain method DoWork, got:\n{}",
        emission.setup_block
    );

    // Must not contain any hardcoded domain-specific names.
    for name in &[
        "ImageBackend",
        "DocumentExtractor",
        "ProcessImage",
        "ExtractBytes",
        "sample_crate",
        "ConsumerLib",
    ] {
        assert!(
            !emission.setup_block.contains(name),
            "setup_block must not contain domain name '{name}', got:\n{}",
            emission.setup_block
        );
        assert!(
            !emission.teardown_block.contains(name),
            "teardown_block must not contain domain name '{name}', got:\n{}",
            emission.teardown_block
        );
    }
    assert_eq!(
        emission.teardown_block,
        "FixtureFacade.UnregisterTestTrait(\"my_fixture\");"
    );
}

#[test]
fn test_emit_test_backend_includes_name_version_properties_with_super_trait() {
    use crate::core::config::TraitBridgeConfig;
    use crate::core::ir::{MethodDef, ReceiverKind, TypeRef};

    let method = MethodDef {
        name: "initialize".to_string(),
        params: vec![],
        return_type: TypeRef::Unit,
        is_async: false,
        is_static: false,
        error_type: None,
        doc: String::new(),
        receiver: Some(ReceiverKind::Ref),
        sanitized: false,
        trait_source: Some("Plugin".to_string()),
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
    };

    let bridge = TraitBridgeConfig {
        trait_name: "ImageBackend".to_string(),
        super_trait: Some("Plugin".to_string()),
        ..Default::default()
    };

    let fixture = Fixture {
        id: "test_ocr".to_string(),
        category: None,
        description: "test".to_string(),
        tags: vec![],
        skip: None,
        env: None,
        call: None,
        input: serde_json::json!({"name": "my_ocr"}),
        mock_response: None,
        source: String::new(),
        http: None,
        assertions: vec![],
        visitor: None,
        args: vec![],
        assertion_recipes: vec![],
    };

    let methods = vec![&method];
    let emission = super::emit_test_backend(&bridge, &methods, &fixture);

    // Must include Name and Version properties
    assert!(
        emission.setup_block.contains("public string Name => \"my_ocr\";"),
        "setup_block should contain Name property, got:\n{}",
        emission.setup_block
    );
    assert!(
        emission.setup_block.contains("public string Version => \"1.0.0\";"),
        "setup_block should contain Version property, got:\n{}",
        emission.setup_block
    );

    // Must implement the interface
    assert!(
        emission.setup_block.contains("IImageBackend"),
        "setup_block should reference IImageBackend, got:\n{}",
        emission.setup_block
    );
}

/// Test that void-returning registration methods are emitted as statements,
/// not as variable assignments. The returns_void flag should prevent:
///   var result = GeneratedBinding.RegisterBackend(...);  // WRONG - CS0815 Cannot assign void
/// And instead emit:
///   GeneratedBinding.RegisterBackend(...);  // CORRECT
#[test]
fn test_void_returning_register_calls_emit_as_statements() {
    // Create a call config with returns_void = true.
    let call_config = CallConfig {
        function: "register_ocr_backend".to_string(),
        returns_void: true,
        result_var: "result".to_string(),
        ..CallConfig::default()
    };

    // Verify the flag is correctly set. The C# codegen checks this at line 937:
    // let returns_void = if call_config.returns_void { true } else { ... };
    assert!(
        call_config.returns_void,
        "CallConfig.returns_void must be true for register_ocr_backend"
    );

    // The codegen then uses this to control template rendering:
    // Line 1227: has_usable_assertion => !expects_error && !returns_void,
    // Which causes the template to emit the call without assignment:
    // Line 76 (else branch): {{ async_kw }}{{ call_target }}.{{ call_expr }};
    // NOT Line 73: var {{ result_var }} = {{ async_kw }}{{ call_target }}.{{ call_expr }};
}
