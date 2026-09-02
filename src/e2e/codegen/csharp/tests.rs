use crate::core::config::ResolvedCrateConfig;
use crate::e2e::config::{ArgMapping, CallConfig, E2eConfig, SelectWhen};
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::{Assertion, Fixture};
use heck::ToPascalCase;
use std::collections::{HashMap, HashSet};

use super::stubs::emit_test_backend_with_class_name;

fn make_fixture_with_input(id: &str, input: serde_json::Value) -> Fixture {
    Fixture {
        docs: None,
        requirements: Vec::new(),
        id: id.to_string(),
        category: None,
        description: "test fixture".to_string(),
        tags: vec![],
        skip: None,
        env: None,
        setup: Vec::new(),
        call: None,
        input,
        mock_response: None,
        source: String::new(),
        http: None,
        asyncapi: None,
        websocket: None,
        preserve_input_urls: false,
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
        crate::e2e::codegen::call_ir::TargetParams::IrAbsent,
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
    };

    let bridge = TraitBridgeConfig {
        trait_name: "TestTrait".to_string(),
        super_trait: Some("Plugin".to_string()),
        register_fn: Some("register_test_trait".to_string()),
        ..Default::default()
    };

    let fixture = Fixture {
        docs: None,
        requirements: Vec::new(),
        id: "my_fixture".to_string(),
        category: None,
        description: "test".to_string(),
        tags: vec![],
        skip: None,
        env: None,
        setup: Vec::new(),
        call: None,
        input: serde_json::Value::Null,
        mock_response: None,
        source: String::new(),
        http: None,
        asyncapi: None,
        websocket: None,
        preserve_input_urls: false,
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
        cfg: None,
        sanitized: false,
        trait_source: Some("Plugin".to_string()),
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    };

    let bridge = TraitBridgeConfig {
        trait_name: "ImageBackend".to_string(),
        super_trait: Some("Plugin".to_string()),
        ..Default::default()
    };

    let fixture = Fixture {
        docs: None,
        requirements: Vec::new(),
        id: "test_ocr".to_string(),
        category: None,
        description: "test".to_string(),
        tags: vec![],
        skip: None,
        env: None,
        setup: Vec::new(),
        call: None,
        input: serde_json::json!({"name": "my_ocr"}),
        mock_response: None,
        source: String::new(),
        http: None,
        asyncapi: None,
        websocket: None,
        preserve_input_urls: false,
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

/// Test that the C# e2e codegen emits the correct facade class name,
/// derived from crate_name, not from stale alef.toml overrides.
/// For a crate named "sample_processor", the facade should be
/// "SampleProcessorConverter", not a stale override.
#[test]
fn test_csharp_facade_class_name_is_computed_correctly() {
    // The C# e2e codegen computes class_name via csharp_wrapper_class_name(&config.name, "")
    // which converts "sample_processor" -> "SampleProcessor" -> "SampleProcessorConverter"
    let computed = crate::codegen::naming::csharp_wrapper_class_name("sample_processor", "");
    assert_eq!(computed, "SampleProcessorConverter");

    // Verify the naming transformation chain:
    // "sample_processor" -> to_csharp_name -> "SampleProcessor" -> strip "Rs" (not present)
    // -> append "Converter"
    let pascal = "sample_processor".to_pascal_case();
    assert_eq!(pascal, "SampleProcessor");
    let stripped = pascal.strip_suffix("Rs").unwrap_or(&pascal);
    assert_eq!(stripped, "SampleProcessor");
    let with_converter = format!("{}Converter", stripped);
    assert_eq!(with_converter, "SampleProcessorConverter");
}

fn fixture_with_declared_error(value: &str) -> Fixture {
    let mut fixture = make_fixture_with_input("declares_error", serde_json::json!({}));
    fixture.assertions = vec![Assertion {
        assertion_type: "error".to_string(),
        value: Some(serde_json::Value::String(value.to_string())),
        ..Assertion::default()
    }];
    fixture
}

#[test]
fn declared_error_value_check_returns_none_without_a_declared_value() {
    let fixture = make_fixture_with_input("no_error", serde_json::json!({}));
    assert_eq!(super::declared_error_value_check(&fixture, &[]), None);
}

/// With no `errors` IR supplied — as every one of these hand-built fixtures has — a value
/// cannot be recognised as a known variant name, so it renders exactly like a message-style
/// value always did before this fix.
#[test]
fn declared_error_value_check_asserts_message_or_type_name() {
    let fixture = fixture_with_declared_error("BadRequest");
    let check = super::declared_error_value_check(&fixture, &[]).expect("expected a rendered check");
    assert!(
        check.contains("thrown.Message != null && thrown.Message.Contains(\"BadRequest\")"),
        "got: {check}"
    );
    assert!(
        check.contains("thrown.GetType().Name.Contains(\"BadRequest\")"),
        "got: {check}"
    );
}

#[test]
fn declared_error_value_check_escapes_quotes_and_backslashes() {
    let fixture = fixture_with_declared_error("bad \"field\" \\ value");
    let check = super::declared_error_value_check(&fixture, &[]).expect("expected a rendered check");
    assert!(
        check.contains("bad \\\"field\\\" \\\\ value"),
        "expected escaped literal, got: {check}"
    );
}

/// The defect this fix closes: a declared value that names a real `ErrorVariant` — C#'s
/// generated binding never differentiates one from another for an ordinary business-call
/// failure — must render the registered skip, not an assertion that can never pass.
#[test]
fn declared_error_value_check_skips_a_known_variant_c_sharp_cannot_substantiate() {
    use crate::core::ir::{ErrorDef, ErrorVariant};

    let fixture = fixture_with_declared_error("Authentication");
    let errors = vec![ErrorDef {
        name: "ApiError".to_string(),
        rust_path: "lib::ApiError".to_string(),
        original_rust_path: String::new(),
        variants: vec![ErrorVariant {
            name: "Authentication".to_string(),
            error_code: Some(100),
            is_unit: true,
            ..ErrorVariant::default()
        }],
        doc: String::new(),
        methods: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }];

    let check = super::declared_error_value_check(&fixture, &errors).expect("expected a rendered skip");
    assert_eq!(
        check,
        "        // skipped: declared error variant 'Authentication' not yet preserved as a distinct identity by \
         this backend's generator"
    );
    assert!(
        !check.contains("Assert.True"),
        "must not render an assertion that can never pass, got: {check}"
    );
}

/// Renders a fixture through the real per-fixture entry point
/// (`render_test_method`) whose sole assertion targets a field absent from
/// `result_fields`. Proves the `field '<name>' not available on result type`
/// marker `assertions.rs`'s `is_valid_for_result` skip branch emits survives
/// into the rendered method body, which is what
/// `fail_on_unavailable_field_markers` scans at generation time.
#[test]
fn dropped_field_assertion_carries_the_marker_in_the_rendered_test_method() {
    let fixture = Fixture {
        id: "widget_smoke".into(),
        description: "Widget smoke".into(),
        assertions: vec![Assertion {
            assertion_type: "not_empty".into(),
            field: Some("definitely_missing_field".into()),
            ..Assertion::default()
        }],
        ..Fixture::default()
    };
    let mut e2e_config = E2eConfig::default();
    e2e_config.call.function = "get_widget".into();
    e2e_config.call.result_var = "result".into();
    e2e_config.result_fields = ["content".to_string()].into_iter().collect::<HashSet<_>>();

    let field_resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    );
    let config = ResolvedCrateConfig {
        name: "sample".into(),
        ..ResolvedCrateConfig::default()
    };

    let mut out = String::new();
    let mut visitor_class_decls: Vec<String> = Vec::new();
    super::render_test_method(
        &mut out,
        &mut visitor_class_decls,
        &fixture,
        "Widget",
        "GetWidget",
        "WidgetException",
        "result",
        &[],
        &field_resolver,
        false,
        false,
        &e2e_config,
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
        &[],
        &config,
        &[],
        &[],
        &[],
        &[],
    );

    assert!(
        out.contains("skipped: field 'definitely_missing_field' not available on result type"),
        "expected the unavailable-field marker in the rendered test method, got:\n{out}"
    );
}

/// Renders a streaming fixture directly through `render_streaming_test_method`
/// (csharp's structurally separate streaming assertion path) whose sole
/// assertion targets a field the non-chat-stream branch's `result_fields`
/// predicate rejects. Proves the `unsupported field '<name>'`-shaped marker
/// `streaming.rs` emits (a differently-worded variant from the main assertions
/// path) also survives into the rendered method body.
#[test]
fn dropped_streaming_field_assertion_carries_the_unsupported_field_marker() {
    let fixture = Fixture {
        id: "stream_widget_smoke".into(),
        description: "Streaming widget smoke".into(),
        assertions: vec![Assertion {
            assertion_type: "not_empty".into(),
            field: Some("weird_untracked_field".into()),
            ..Assertion::default()
        }],
        ..Fixture::default()
    };
    let call_config = CallConfig::default();
    let e2e_config = E2eConfig::default();
    let config = ResolvedCrateConfig {
        name: "sample".into(),
        ..ResolvedCrateConfig::default()
    };

    let mut out = String::new();
    super::render_streaming_test_method(
        &mut out,
        &fixture,
        "Widget",
        &call_config,
        None,
        &e2e_config,
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
        "WidgetException",
        &[],
        &config,
        &[],
        &[],
        &[],
        Some("ChunkItem"),
    );

    assert!(
        out.contains("skipped: streaming assertion on unsupported field 'weird_untracked_field'"),
        "expected the streaming unsupported-field marker in the rendered method, got:\n{out}"
    );
}

/// Regression test: `render_streaming_test_method`'s non-chat-stream branch must gate field
/// assertions on `e2e_config.effective_result_fields(call_config)`, not on
/// `call_config.result_fields` directly. A per-call `result_fields` list REPLACES the global
/// one rather than merging with it, so a crate that configures only `[crates.e2e].result_fields`
/// leaves every per-call `result_fields` empty — and the gate this test exercises,
/// `!result_fields.iter().any(|f| field.starts_with(f))`, is vacuously true on an empty set,
/// which degraded every non-chat streaming assertion to a skip comment while the suite still
/// reported green (fixed in 4fc82aca8).
///
/// The existing `emit_non_chat_stream_assertion`-level test in `streaming.rs` cannot catch this:
/// it calls the leaf helper directly with a hand-built `HashSet`, so it never observes which set
/// `render_streaming_test_method` actually threads through. This test goes through the real
/// wiring instead — only `[e2e].result_fields` (the global set) names the asserted field; the
/// call config's own `result_fields` is left empty, exactly the shape that reached production. ~keep
#[test]
fn render_streaming_test_method_gates_non_chat_assertions_on_the_effective_result_fields() {
    let fixture = Fixture {
        id: "stream_custom_field_smoke".into(),
        description: "Streaming custom field smoke".into(),
        assertions: vec![Assertion {
            assertion_type: "not_empty".into(),
            field: Some("custom_field".into()),
            ..Assertion::default()
        }],
        ..Fixture::default()
    };
    // No per-call override: `call_config.result_fields` is empty, as it always is for a crate
    // that only ever configures the global `[e2e].result_fields`.
    let call_config = CallConfig::default();
    let e2e_config = E2eConfig {
        result_fields: ["custom_field".to_string()].into_iter().collect::<HashSet<_>>(),
        ..E2eConfig::default()
    };
    let config = ResolvedCrateConfig {
        name: "sample".into(),
        ..ResolvedCrateConfig::default()
    };

    let mut out = String::new();
    super::render_streaming_test_method(
        &mut out,
        &fixture,
        "Widget",
        &call_config,
        None,
        &e2e_config,
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
        "WidgetException",
        &[],
        &config,
        &[],
        &[],
        &[],
        Some("ChunkItem"),
    );

    assert!(
        out.contains("Assert.NotEmpty(chunks);"),
        "expected a real assertion for a field named by the global result_fields set, got:\n{out}"
    );
    assert!(
        !out.contains("skipped: streaming assertion on unsupported field 'custom_field'"),
        "the globally-configured field must not degrade to a skip comment, got:\n{out}"
    );
}

/// Negative control for the test above: a field that is genuinely absent from the effective
/// `result_fields` set — global or per-call — must still degrade to a skip comment. Without
/// this control, a "fix" that stopped gating on `result_fields` at all (emitting a real
/// assertion for every field, tracked or not) would pass the positive test above while quietly
/// destroying the skip mechanism `fail_on_unavailable_field_markers` depends on. ~keep
#[test]
fn render_streaming_test_method_still_skips_a_field_absent_from_the_effective_result_fields() {
    let fixture = Fixture {
        id: "stream_missing_field_smoke".into(),
        description: "Streaming missing field smoke".into(),
        assertions: vec![Assertion {
            assertion_type: "not_empty".into(),
            field: Some("definitely_missing_field".into()),
            ..Assertion::default()
        }],
        ..Fixture::default()
    };
    let call_config = CallConfig::default();
    let e2e_config = E2eConfig {
        result_fields: ["custom_field".to_string()].into_iter().collect::<HashSet<_>>(),
        ..E2eConfig::default()
    };
    let config = ResolvedCrateConfig {
        name: "sample".into(),
        ..ResolvedCrateConfig::default()
    };

    let mut out = String::new();
    super::render_streaming_test_method(
        &mut out,
        &fixture,
        "Widget",
        &call_config,
        None,
        &e2e_config,
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
        "WidgetException",
        &[],
        &config,
        &[],
        &[],
        &[],
        Some("ChunkItem"),
    );

    assert!(
        out.contains("skipped: streaming assertion on unsupported field 'definitely_missing_field'"),
        "a field absent from the effective result_fields set must still be skipped, got:\n{out}"
    );
    assert!(
        !out.contains("Assert.NotEmpty(chunks);"),
        "no real assertion should be emitted for an unsupported field, got:\n{out}"
    );
}

/// Regression test for alef task #86: a `visitor` fixture whose options type resolves
/// from neither `[e2e.call]` nor any `[[crates.trait_bridges]]` entry used to emit
/// `[Fact] public void TestX() { return; }` — a test xUnit reports as PASSING while
/// exercising none of the visitor behavior, so the emitted suite tested strictly less
/// than it claimed. It must now fail at generation time, naming the fixture and the
/// missing options type — mirroring `c/assertions.rs`'s `build_args_string_c` and
/// `kotlin/args.rs`, which already refuse to emit for an unresolvable trait bridge.
#[test]
#[should_panic(expected = "C# e2e generator: fixture `visitor_smoke` declares a `visitor`")]
fn visitor_fixture_without_trait_bridge_options_type_fails_loudly_instead_of_emitting_an_empty_test() {
    use crate::e2e::fixture::{CallbackAction, VisitorSpec};

    let fixture = Fixture {
        id: "visitor_smoke".into(),
        description: "Visitor smoke".into(),
        visitor: Some(VisitorSpec {
            callbacks: [("visit_element".to_string(), CallbackAction::Skip)]
                .into_iter()
                .collect(),
        }),
        ..Fixture::default()
    };
    let mut e2e_config = E2eConfig::default();
    e2e_config.call.function = "convert".into();
    e2e_config.call.result_var = "result".into();

    let field_resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    );
    // No `[[crates.trait_bridges]]` entries declared — nothing supplies an `options_type`.
    let config = ResolvedCrateConfig {
        name: "sample".into(),
        ..ResolvedCrateConfig::default()
    };

    let mut out = String::new();
    let mut visitor_class_decls: Vec<String> = Vec::new();
    super::render_test_method(
        &mut out,
        &mut visitor_class_decls,
        &fixture,
        "Widget",
        "Convert",
        "WidgetException",
        "result",
        &[],
        &field_resolver,
        false,
        false,
        &e2e_config,
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
        &[],
        &config,
        &[],
        &[],
        &[],
        &[],
    );
}

fn render_csharp_error_method(extra: Vec<Assertion>) -> String {
    let mut assertions = vec![Assertion {
        assertion_type: "error".into(),
        ..Assertion::default()
    }];
    assertions.extend(extra);
    let fixture = Fixture {
        id: "rate_limited".into(),
        description: "Rejects the request".into(),
        assertions,
        ..Fixture::default()
    };
    let mut e2e_config = E2eConfig::default();
    e2e_config.call.function = "get_widget".into();
    e2e_config.call.result_var = "result".into();

    let field_resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    );
    let config = ResolvedCrateConfig {
        name: "sample".into(),
        ..ResolvedCrateConfig::default()
    };

    let mut out = String::new();
    let mut visitor_class_decls: Vec<String> = Vec::new();
    let _ = crate::e2e::codegen::take_skip_records();
    super::render_test_method(
        &mut out,
        &mut visitor_class_decls,
        &fixture,
        "Widget",
        "GetWidget",
        "WidgetException",
        "result",
        &[],
        &field_resolver,
        false,
        false,
        &e2e_config,
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
        &[],
        &config,
        &[],
        &[],
        &[],
        &[],
    );
    out
}

/// C#'s `expects_error` branch renders `Assert.ThrowsAny<..>` and returns, so every other
/// assertion on the fixture used to leave no trace in the generated test at all.
#[test]
fn csharp_equals_on_an_error_field_is_named_instead_of_dropped() {
    let out = render_csharp_error_method(vec![Assertion {
        assertion_type: "equals".into(),
        field: Some("error.status_code".into()),
        ..Assertion::default()
    }]);

    // Positive first: the error block really rendered.
    assert!(out.contains("ThrowsAny"), "the error block must render, got:\n{out}");
    assert!(
        out.contains(
            "// skipped: assertion type 'equals' has no accessor for error field error.status_code in this backend"
        ),
        "got:\n{out}"
    );

    let records = crate::e2e::codegen::take_skip_records();
    assert_eq!(records.len(), 1, "got: {records:?}");
    assert_eq!(records[0].language, "csharp");
    assert_eq!(records[0].field, "equals");
}

/// Negative control: a lone `error` assertion must leave the generated method marker-free.
#[test]
fn csharp_a_lone_error_assertion_renders_no_marker() {
    let out = render_csharp_error_method(Vec::new());

    assert!(out.contains("ThrowsAny"), "the error block must render, got:\n{out}");
    assert!(!out.contains("has no accessor for error field"), "got:\n{out}");
}

// --- typed-argument lowering (alef #227) -----------------------------------------------------

/// An `args` entry with the default `arg_type` (`"string"`) — the shape that used to send every
/// value through `json_to_csharp` regardless of what the parameter it fills is declared as. ~keep
fn default_typed_arg(name: &str) -> ArgMapping {
    ArgMapping {
        name: name.to_string(),
        field: format!("input.{name}"),
        arg_type: "string".to_string(),
        optional: false,
        owned: false,
        element_type: None,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    }
}

fn csharp_args_for(
    args: &[ArgMapping],
    fixture: &Fixture,
    type_defs: &[crate::core::ir::TypeDef],
    enums: &[crate::core::ir::EnumDef],
    target_params: crate::e2e::codegen::call_ir::TargetParams<'_>,
) -> String {
    let mut class_decls = Vec::new();
    let mut teardown_lines = Vec::new();
    let (_setup, args_str) = super::build_args_and_setup(
        &fixture.input,
        args,
        "Sample",
        None,
        None,
        &HashMap::new(),
        &HashMap::new(),
        fixture,
        None,
        &ResolvedCrateConfig::default(),
        type_defs,
        enums,
        target_params,
        &mut class_decls,
        &mut teardown_lines,
    );
    args_str
}

/// The defect: a fixture object bound for a record-typed parameter used to become a *quoted JSON
/// string literal*, which `csc` rejects. With the declared type resolved it deserializes. ~keep
#[test]
fn an_object_value_for_an_ir_struct_parameter_deserializes_instead_of_stringifying() {
    use crate::core::ir::{ParamDef, TypeDef, TypeRef};
    let args = vec![default_typed_arg("request")];
    let fixture = make_fixture_with_input("typed", serde_json::json!({ "request": { "prompt": "hi" } }));
    let params = [ParamDef {
        name: "request".to_string(),
        ty: TypeRef::Named("CompletionRequest".to_string()),
        ..ParamDef::default()
    }];
    let type_defs = [TypeDef {
        name: "CompletionRequest".to_string(),
        ..TypeDef::default()
    }];
    let rendered = csharp_args_for(
        &args,
        &fixture,
        &type_defs,
        &[],
        crate::e2e::codegen::call_ir::TargetParams::Known(&params),
    );
    assert_eq!(
        rendered,
        "JsonSerializer.Deserialize<CompletionRequest>(\"{\\\"prompt\\\":\\\"hi\\\"}\", ConfigOptions)!"
    );
}

/// An enum-typed parameter takes the member `gen_enum` actually emitted for the wire value, not a
/// bare string. The variant here is `serde(rename_all = "kebab-case")`, which is precisely the case
/// a `to_upper_camel_case` of the wire value would get wrong. ~keep
#[test]
fn a_string_value_for_an_ir_enum_parameter_names_the_generated_member() {
    use crate::core::ir::{EnumDef, EnumVariant, ParamDef, TypeRef};
    let args = vec![default_typed_arg("purpose")];
    let fixture = make_fixture_with_input("typed", serde_json::json!({ "purpose": "fine-tune" }));
    let params = [ParamDef {
        name: "purpose".to_string(),
        ty: TypeRef::Named("FilePurpose".to_string()),
        ..ParamDef::default()
    }];
    let enums = [EnumDef {
        name: "FilePurpose".to_string(),
        serde_rename_all: Some("kebab-case".to_string()),
        variants: vec![EnumVariant {
            name: "FineTune".to_string(),
            ..EnumVariant::default()
        }],
        ..EnumDef::default()
    }];
    let rendered = csharp_args_for(
        &args,
        &fixture,
        &[],
        &enums,
        crate::e2e::codegen::call_ir::TargetParams::Known(&params),
    );
    assert_eq!(rendered, "FilePurpose.FineTune");
}

/// The other half of the three-state trade. Identical arg and fixture value, `IrAbsent` instead of
/// `Known`: the pre-seam lowering must survive verbatim, or every IR-less caller (the snippet path,
/// and every test that renders without an IR) regresses silently. ~keep
#[test]
fn the_same_object_value_still_stringifies_when_the_ir_is_absent() {
    let args = vec![default_typed_arg("request")];
    let fixture = make_fixture_with_input("typed", serde_json::json!({ "request": { "prompt": "hi" } }));
    let rendered = csharp_args_for(
        &args,
        &fixture,
        &[],
        &[],
        crate::e2e::codegen::call_ir::TargetParams::IrAbsent,
    );
    assert_eq!(rendered, "\"{\\\"prompt\\\":\\\"hi\\\"}\"");
}

/// The IrAbsent half for the enum arm: the same string value keeps its string literal. ~keep
#[test]
fn the_same_string_value_still_renders_as_a_literal_when_the_ir_is_absent() {
    let args = vec![default_typed_arg("purpose")];
    let fixture = make_fixture_with_input("typed", serde_json::json!({ "purpose": "fine-tune" }));
    let rendered = csharp_args_for(
        &args,
        &fixture,
        &[],
        &[],
        crate::e2e::codegen::call_ir::TargetParams::IrAbsent,
    );
    assert_eq!(rendered, "\"fine-tune\"");
}

/// A wire value naming no variant is very often a deliberately invalid value driving the binding's
/// own validation. Inventing a member for it would not compile and would delete the test's point. ~keep
#[test]
fn an_unmatched_enum_wire_value_keeps_its_string_literal() {
    use crate::core::ir::{EnumDef, EnumVariant, ParamDef, TypeRef};
    let args = vec![default_typed_arg("purpose")];
    let fixture = make_fixture_with_input("typed", serde_json::json!({ "purpose": "invalid-purpose" }));
    let params = [ParamDef {
        name: "purpose".to_string(),
        ty: TypeRef::Named("FilePurpose".to_string()),
        ..ParamDef::default()
    }];
    let enums = [EnumDef {
        name: "FilePurpose".to_string(),
        serde_rename_all: Some("kebab-case".to_string()),
        variants: vec![EnumVariant {
            name: "FineTune".to_string(),
            ..EnumVariant::default()
        }],
        ..EnumDef::default()
    }];
    let rendered = csharp_args_for(
        &args,
        &fixture,
        &[],
        &enums,
        crate::e2e::codegen::call_ir::TargetParams::Known(&params),
    );
    assert_eq!(rendered, "\"invalid-purpose\"");
}

/// A declared type absent from both IR registries is not a licence to invent a deserializer: it may
/// be a newtype the C# binding flattens to a `string`. ~keep
#[test]
fn a_declared_type_unknown_to_the_ir_keeps_the_existing_lowering() {
    use crate::core::ir::{ParamDef, TypeRef};
    let args = vec![default_typed_arg("request")];
    let fixture = make_fixture_with_input("typed", serde_json::json!({ "request": "hi" }));
    let params = [ParamDef {
        name: "request".to_string(),
        ty: TypeRef::Named("PromptText".to_string()),
        ..ParamDef::default()
    }];
    let rendered = csharp_args_for(
        &args,
        &fixture,
        &[],
        &[],
        crate::e2e::codegen::call_ir::TargetParams::Known(&params),
    );
    assert_eq!(rendered, "\"hi\"");
}

/// An `Option<T>`/`Vec<T>` parameter wants a wrapper this expression does not build, so the seam
/// must decline rather than unwrap to `T` and trade one compile error for another. ~keep
#[test]
fn a_wrapped_named_parameter_is_left_to_the_existing_lowering() {
    use crate::core::ir::{ParamDef, TypeDef, TypeRef};
    let args = vec![default_typed_arg("request")];
    let fixture = make_fixture_with_input("typed", serde_json::json!({ "request": { "prompt": "hi" } }));
    let params = [ParamDef {
        name: "request".to_string(),
        ty: TypeRef::Optional(Box::new(TypeRef::Named("CompletionRequest".to_string()))),
        ..ParamDef::default()
    }];
    let type_defs = [TypeDef {
        name: "CompletionRequest".to_string(),
        ..TypeDef::default()
    }];
    let rendered = csharp_args_for(
        &args,
        &fixture,
        &type_defs,
        &[],
        crate::e2e::codegen::call_ir::TargetParams::Known(&params),
    );
    assert_eq!(rendered, "\"{\\\"prompt\\\":\\\"hi\\\"}\"");
}

/// Build the fixture + config pair the refusal tests share: `result_fields` names only `content`,
/// which is what arms the availability oracle — with it empty the resolver is deliberately
/// permissive and no field is ever rejected. ~keep
pub(super) fn render_refusal_candidate(fixture_id: &str, assertions: Vec<Assertion>) -> String {
    let fixture = Fixture {
        id: fixture_id.into(),
        description: "Widget smoke".into(),
        assertions,
        ..Fixture::default()
    };
    let mut e2e_config = E2eConfig::default();
    e2e_config.call.function = "get_widget".into();
    e2e_config.call.result_var = "result".into();
    e2e_config.call.returns_result = true;
    e2e_config.result_fields = ["content".to_string()].into_iter().collect::<HashSet<_>>();

    let field_resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &["content".to_string()].into_iter().collect::<HashSet<_>>(),
        &HashSet::new(),
        &HashSet::new(),
    );
    let config = ResolvedCrateConfig {
        name: "sample".into(),
        ..ResolvedCrateConfig::default()
    };

    let mut out = String::new();
    let mut visitor_class_decls: Vec<String> = Vec::new();
    super::render_test_method(
        &mut out,
        &mut visitor_class_decls,
        &fixture,
        "Widget",
        "GetWidget",
        "WidgetException",
        "result",
        &[],
        &field_resolver,
        false,
        false,
        &e2e_config,
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
        &[],
        &config,
        &[],
        &[],
        &[],
        &[],
    );
    out
}

/// CONTROL for the refusal wired in #233, asserted before any absence assertion: a field the
/// availability oracle resolves still renders its real check, the method keeps a plain `[Fact]`,
/// and nothing is recorded. An over-broad refusal here would silently delete coverage that runs
/// today — the same defect pointing the other way. ~keep
#[test]
fn a_resolvable_assertion_keeps_a_plain_fact_and_is_never_refused() {
    let _ = crate::e2e::codegen::inert_example::take_inert_examples();

    let out = render_refusal_candidate(
        "csharp_control",
        vec![Assertion {
            assertion_type: "not_empty".into(),
            field: Some("content".into()),
            ..Assertion::default()
        }],
    );

    // C# renders `not_empty` as an `Assert.True(... switch { ... })` over the boxed accessor, not
    // as `Assert.NotEmpty`. Pinning the latter pinned a spelling this backend never emits. ~keep
    assert!(
        out.contains("expected non-empty value"),
        "the renderable assertion must still be emitted, got:\n{out}"
    );
    assert!(
        out.contains("[Fact]") && !out.contains("[Fact(Skip"),
        "a live example must keep a plain [Fact], got:\n{out}"
    );
    assert!(
        !out.contains("unresolvedAssertion"),
        "a live example must not be refused, got:\n{out}"
    );
    assert!(
        crate::e2e::codegen::inert_example::take_inert_examples().is_empty(),
        "nothing may be recorded for a live example"
    );
}

/// A field the availability oracle rejects is the consumer's to fix, so the disarmed run that
/// still emits it gets an assertion that FAILS and names the fixture — never `[Fact(Skip = ..)]`,
/// which would let a fixable authoring gap sit quietly in the skipped column forever.
#[test]
fn an_unresolved_field_path_is_refused_with_an_assertion_that_fails() {
    let _ = crate::e2e::codegen::inert_example::take_inert_examples();

    let out = render_refusal_candidate(
        "csharp_unresolved",
        vec![Assertion {
            assertion_type: "not_empty".into(),
            field: Some("definitely_missing_field".into()),
            ..Assertion::default()
        }],
    );

    assert!(
        out.contains("string unresolvedAssertion = \"alef resolved no assertion for fixture `csharp_unresolved`")
            && out.contains("Assert.Null(unresolvedAssertion);"),
        "a consumer-fixable gap must be refused with an assertion that fails, got:\n{out}"
    );
    assert!(
        out.contains("skipped: field 'definitely_missing_field' not available on result type"),
        "the marker must be carried into the refusal, not replaced by silence, got:\n{out}"
    );
    assert!(
        !out.contains("[Fact(Skip"),
        "a consumer-fixable gap must not be parked as skipped, got:\n{out}"
    );
    let refusals = crate::e2e::codegen::inert_example::take_inert_examples();
    assert_eq!(refusals.len(), 1, "the refusal must be recorded once for the summary");
    assert_eq!(refusals[0].fixture_id, "csharp_unresolved");
}

/// alef's own acknowledged debt is not the consumer's to fix. xUnit has no in-body skip, so the
/// method's own attribute becomes `[Fact(Skip = ..)]` — the runner reports it as skipped and never
/// as a pass — and the markers stay in the body so the refusal is not a silent skip.
#[test]
fn acknowledged_generator_debt_is_refused_with_a_fact_skip_attribute() {
    let _ = crate::e2e::codegen::inert_example::take_inert_examples();

    let out = render_refusal_candidate(
        "csharp_generator_debt",
        vec![Assertion {
            assertion_type: "not_empty".into(),
            field: Some("definitely_missing_field".into()),
            skip: Some(crate::e2e::fixture::AssertionSkip::All(true)),
            ..Assertion::default()
        }],
    );

    assert!(
        out.contains("[Fact(Skip = \"alef rendered no runnable expectation for fixture `csharp_generator_debt`"),
        "acknowledged debt must be parked as skipped, got:\n{out}"
    );
    assert!(
        out.contains("skipped: field 'definitely_missing_field' not available on result type"),
        "the marker must survive into the skipped method, got:\n{out}"
    );
    assert!(
        !out.contains("unresolvedAssertion"),
        "alef's own debt must not fail a consumer's suite, got:\n{out}"
    );
    assert_eq!(crate::e2e::codegen::inert_example::take_inert_examples().len(), 1);
}

/// CONTROL: a fixture that declares NO assertions is the deliberate "just call it" smoke contract
/// and must be published exactly as before. ~keep
#[test]
fn a_fixture_with_no_declared_assertions_keeps_its_smoke_test_shape() {
    let _ = crate::e2e::codegen::inert_example::take_inert_examples();

    let out = render_refusal_candidate("csharp_smoke_only", Vec::new());

    assert!(
        out.contains("[Fact]") && !out.contains("[Fact(Skip"),
        "a fixture with no assertions must never be refused, got:\n{out}"
    );
    assert!(
        crate::e2e::codegen::inert_example::take_inert_examples().is_empty(),
        "a fixture with no assertions must never be recorded as refused"
    );
}

/// Regression test for the fabricated-completion defect: a chat-shaped fixture that never
/// declares `stream_complete` (here, `empty_stream`'s real shape -- `count_min chunks >= 0`
/// plus `equals stream_content == ""`, an explicit statement that zero chunks is acceptable)
/// must not have `Assert.True(streamComplete);` invented on its behalf. That expectation would
/// contradict rather than check a fixture like this one. ~keep
#[test]
fn a_fixture_that_never_declares_stream_complete_gets_no_invented_expectation() {
    let fixture = Fixture {
        id: "empty_stream".into(),
        description: "Streaming chat completion that produces no content chunks".into(),
        assertions: vec![
            Assertion {
                assertion_type: "count_min".into(),
                field: Some("chunks".into()),
                value: Some(serde_json::json!(0)),
                ..Assertion::default()
            },
            Assertion {
                assertion_type: "equals".into(),
                field: Some("stream_content".into()),
                value: Some(serde_json::json!("")),
                ..Assertion::default()
            },
        ],
        ..Fixture::default()
    };
    let call_config = CallConfig::default();
    let e2e_config = E2eConfig::default();
    let config = ResolvedCrateConfig {
        name: "sample".into(),
        ..ResolvedCrateConfig::default()
    };

    let mut out = String::new();
    super::render_streaming_test_method(
        &mut out,
        &fixture,
        "Widget",
        &call_config,
        None,
        &e2e_config,
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
        "WidgetException",
        &[],
        &config,
        &[],
        &[],
        &[],
        Some("ChatCompletionChunk"),
    );

    assert!(
        !out.contains("Assert.True(streamComplete);"),
        "no expectation may be invented for a field this fixture never declared. got:\n{out}"
    );
}

/// The other half: a chat-shaped fixture that DOES declare `stream_complete` (the real
/// liter-llm `stream_done_signal` shape) must still get a real, falsifiable expectation -- the
/// fix must not regress the declared case into silence.
#[test]
fn a_fixture_that_declares_stream_complete_still_gets_a_real_expectation() {
    let fixture = Fixture {
        id: "stream_done_signal".into(),
        description: "Verify that the DONE sentinel terminates the stream".into(),
        assertions: vec![
            Assertion {
                assertion_type: "equals".into(),
                field: Some("stream_content".into()),
                value: Some(serde_json::json!("done")),
                ..Assertion::default()
            },
            Assertion {
                assertion_type: "is_true".into(),
                field: Some("stream_complete".into()),
                ..Assertion::default()
            },
        ],
        ..Fixture::default()
    };
    let call_config = CallConfig::default();
    let e2e_config = E2eConfig::default();
    let config = ResolvedCrateConfig {
        name: "sample".into(),
        ..ResolvedCrateConfig::default()
    };

    let mut out = String::new();
    super::render_streaming_test_method(
        &mut out,
        &fixture,
        "Widget",
        &call_config,
        None,
        &e2e_config,
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
        "WidgetException",
        &[],
        &config,
        &[],
        &[],
        &[],
        Some("ChatCompletionChunk"),
    );

    assert_eq!(
        out.matches("Assert.True(streamComplete);").count(),
        1,
        "a declared `stream_complete` assertion must render exactly once. got:\n{out}"
    );
}
