use super::visitor::WasmVisitorBinding;
use super::*;
use crate::core::ir::{FieldDef, PrimitiveType};
use crate::e2e::escape::sanitize_filename;
use crate::e2e::fixture::FixtureGroup;

fn make_field(name: &str, ty: TypeRef) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        optional: false,
        default: None,
        doc: String::new(),
        sanitized: false,
        is_boxed: false,
        type_rust_path: None,
        cfg: None,
        typed_default: None,
        core_wrapper: crate::core::ir::CoreWrapper::None,
        vec_inner_core_wrapper: crate::core::ir::CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    }
}

fn make_type(name: &str, fields: Vec<FieldDef>) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: String::new(),
        original_rust_path: String::new(),
        fields,
        methods: Vec::new(),
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        doc: String::new(),
        cfg: None,
        is_trait: false,
        has_default: true,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: true,
        super_traits: Vec::new(),
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,

        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
    }
}

#[test]
fn derive_nested_types_maps_named_field_to_wasm_class() {
    let message_type = make_type("ChatMessage", vec![]);
    let request_type = make_type(
        "ChatRequest",
        vec![make_field(
            "messages",
            TypeRef::Vec(Box::new(TypeRef::Named("ChatMessage".to_string()))),
        )],
    );
    let type_defs = vec![message_type, request_type];

    let derived = derive_nested_types_for_wasm("WasmChatRequest", &type_defs, "Wasm");
    assert_eq!(derived.get("messages"), Some(&"WasmChatMessage".to_string()));
}

#[test]
fn derive_nested_types_maps_optional_named_field() {
    let config_type = make_type("ParseConfig", vec![]);
    let request_type = make_type(
        "ParseRequest",
        vec![make_field(
            "config",
            TypeRef::Optional(Box::new(TypeRef::Named("ParseConfig".to_string()))),
        )],
    );
    let type_defs = vec![config_type, request_type];

    let derived = derive_nested_types_for_wasm("WasmParseRequest", &type_defs, "Wasm");
    assert_eq!(derived.get("config"), Some(&"WasmParseConfig".to_string()));
}

#[test]
fn derive_nested_types_skips_primitive_fields() {
    let request_type = make_type(
        "SimpleRequest",
        vec![
            make_field("count", TypeRef::Primitive(PrimitiveType::U32)),
            make_field("name", TypeRef::String),
        ],
    );
    let derived = derive_nested_types_for_wasm("WasmSimpleRequest", &[request_type], "Wasm");
    assert!(derived.is_empty(), "primitives must not produce nested_types entries");
}

#[test]
fn derive_nested_types_explicit_overrides_derived() {
    let inner_type = make_type("Message", vec![]);
    let outer_type = make_type(
        "Request",
        vec![make_field("message", TypeRef::Named("Message".to_string()))],
    );
    let type_defs = vec![inner_type, outer_type];

    // Explicit override provides a different class name.
    let explicit: std::collections::HashMap<String, String> = [("message".to_string(), "CustomMessage".to_string())]
        .into_iter()
        .collect();

    let derived = derive_nested_types_for_wasm("WasmRequest", &type_defs, "Wasm");
    // Merge: explicit wins on collision.
    let mut effective = derived;
    for (k, v) in &explicit {
        effective.insert(k.clone(), v.clone());
    }
    assert_eq!(effective.get("message"), Some(&"CustomMessage".to_string()));
}

#[test]
fn cache_isolation_setup_uses_generic_alef_names() {
    let mut rendered = String::new();
    emit_cache_isolation_setup(&mut rendered);

    assert!(
        rendered.contains("_alefTestCacheDir"),
        "missing generic cache var: {rendered}"
    );
    assert!(
        rendered.contains("mkdtempSync(join(tmpdir(), \"alef-e2e-\"))"),
        "missing generic cache prefix: {rendered}"
    );
    assert!(
        !rendered.contains("sample_language_pack"),
        "TypeScript cache isolation setup must not contain project-specific names: {rendered}"
    );
}

#[test]
fn derive_nested_types_returns_empty_for_unknown_type() {
    let derived = derive_nested_types_for_wasm("WasmUnknownType", &[], "Wasm");
    assert!(derived.is_empty());
}

#[test]
fn collect_transitive_nested_types_walks_two_levels_deep() {
    // FunctionDefinition is nested inside ChatTool, which is nested inside ChatRequest.
    // Single-level derivation only catches WasmChatTool; transitive must also catch
    // WasmFunctionDefinition so the test-body `new WasmFunctionDefinition()` resolves.
    let function_def = make_type("FunctionDefinition", vec![]);
    let chat_tool = make_type(
        "ChatTool",
        vec![make_field("function", TypeRef::Named("FunctionDefinition".to_string()))],
    );
    let chat_request = make_type(
        "ChatRequest",
        vec![make_field(
            "tools",
            TypeRef::Vec(Box::new(TypeRef::Named("ChatTool".to_string()))),
        )],
    );
    let type_defs = vec![function_def, chat_tool, chat_request];

    let mut seeds = std::collections::BTreeSet::new();
    seeds.insert("WasmChatRequest".to_string());
    let derived = collect_transitive_nested_types_for_wasm(&seeds, &type_defs, "Wasm");

    let class_names: std::collections::HashSet<&String> = derived.values().collect();
    assert!(
        class_names.contains(&"WasmChatTool".to_string()),
        "first-level WasmChatTool missing; got {:?}",
        derived
    );
    assert!(
        class_names.contains(&"WasmFunctionDefinition".to_string()),
        "second-level WasmFunctionDefinition missing; got {:?}",
        derived
    );
}

#[test]
fn ts_builder_uses_default_factory_for_all_wasm_classes_not_just_config() {
    // WasmChatCompletionTool has required (non-Optional) fields, so
    // wasm-bindgen's `(constructor)` requires positional args. The codegen
    // must emit `WasmChatCompletionTool.default()` (the synthetic factory)
    // instead of `new WasmChatCompletionTool()`, which would throw at JS
    // runtime. Previously only `*Config` types used the factory.
    let mut obj = serde_json::Map::new();
    obj.insert("type".to_string(), serde_json::Value::String("function".to_string()));
    let result = ts_builder_expression_inner(
        &obj,
        "WasmChatCompletionTool",
        &std::collections::HashMap::new(),
        "wasm",
        &std::collections::HashMap::new(),
        &std::collections::BTreeSet::new(),
        &[],
        &[],
        "Wasm",
        0,
    );
    assert!(
        result.contains("const _u0 = WasmChatCompletionTool.default();"),
        "wasm builder must instantiate via `.default()` for non-Config classes;\n\
             actual:\n{result}",
    );
    assert!(
        !result.contains("new WasmChatCompletionTool()"),
        "wasm builder must NOT use no-arg `new` for non-Config classes;\n\
             actual:\n{result}",
    );
}

#[test]
fn ts_builder_uses_new_for_non_wasm_targets() {
    // Node target keeps object-literal style — only WASM uses the
    // factory pattern. Sanity check that our condition didn't widen.
    let mut obj = serde_json::Map::new();
    obj.insert("model".to_string(), serde_json::Value::String("gpt-4".to_string()));
    let result = ts_builder_expression_inner(
        &obj,
        "ChatCompletionRequest",
        &std::collections::HashMap::new(),
        "node",
        &std::collections::HashMap::new(),
        &std::collections::BTreeSet::new(),
        &[],
        &[],
        "",
        0,
    );
    // Node path returns an object literal cast — no `default()` call.
    assert!(
        !result.contains(".default()"),
        "non-wasm target must not use the wasm-only default factory pattern;\n\
             actual:\n{result}",
    );
}

#[test]
fn collect_transitive_nested_types_terminates_on_cycles() {
    // Self-referential type A -> A. BFS must terminate via the seen set.
    let recursive = make_type(
        "Recursive",
        vec![make_field(
            "child",
            TypeRef::Optional(Box::new(TypeRef::Named("Recursive".to_string()))),
        )],
    );
    let mut seeds = std::collections::BTreeSet::new();
    seeds.insert("WasmRecursive".to_string());
    let derived = collect_transitive_nested_types_for_wasm(&seeds, &[recursive], "Wasm");
    assert_eq!(derived.get("child"), Some(&"WasmRecursive".to_string()));
}

#[test]
fn wasm_imports_nested_types_from_json_object_element_types() {
    let mut e2e_config = crate::e2e::config::E2eConfig::default();
    e2e_config.call.function = "extract".to_string();
    e2e_config.call.args = vec![crate::e2e::config::ArgMapping {
        name: "input".to_string(),
        field: "input".to_string(),
        arg_type: "json_object".to_string(),
        optional: false,
        owned: false,
        element_type: Some("ExtractInput".to_string()),
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    }];

    let fixture = Fixture {
        id: "extract_input_with_nested_config".to_string(),
        category: Some("extract".to_string()),
        description: "extract input with nested config".to_string(),
        input: serde_json::json!({
            "kind": "bytes",
            "config": {
                "force_ocr": true
            }
        }),
        assertions: vec![crate::e2e::fixture::Assertion {
            assertion_type: "not_error".to_string(),
            field: None,
            value: None,
            values: None,
            method: None,
            check: None,
            args: None,
            return_type: None,
        }],
        ..Default::default()
    };
    let extract_input = make_type(
        "ExtractInput",
        vec![make_field(
            "config",
            TypeRef::Optional(Box::new(TypeRef::Named("FileExtractionConfig".to_string()))),
        )],
    );
    let file_config = make_type("FileExtractionConfig", vec![]);
    let config = crate::core::config::ResolvedCrateConfig::default();

    let output = render_test_file(
        "wasm",
        "extract",
        &[&fixture],
        "",
        "@test/wasm",
        "extract",
        &[],
        Some("WasmExtractionConfig"),
        None,
        &e2e_config,
        &[extract_input, file_config],
        &[],
        "Wasm",
        &config,
    );

    assert!(
        output.contains("WasmFileExtractionConfig"),
        "WASM imports must include nested DTOs reached through json_object element types;\n{output}"
    );
}

#[test]
fn wasm_class_name_prepends_wasm_prefix() {
    assert_eq!(wasm_class_name("ChatMessage", "Wasm"), "WasmChatMessage");
    assert_eq!(wasm_class_name("EmbeddingRequest", "Wasm"), "WasmEmbeddingRequest");
}

#[test]
fn strip_setup_metadata_removes_harness_setup_from_runtime_input() {
    let input = serde_json::json!({
        "setup": { "register": true },
        "text": "hello"
    });
    let cleaned = strip_setup_metadata(&input);
    assert_eq!(cleaned, serde_json::json!({ "text": "hello" }));
}

#[test]
fn node_type_imports_strip_configured_js_prefix() {
    use crate::core::config::NewAlefConfig;
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["node"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.node]
type_prefix = "Js"
"#,
    )
    .unwrap();
    let resolved = cfg.resolve().unwrap().remove(0);
    assert_eq!(
        canonical_ts_type_name("node", "JsParseConfig", &resolved),
        "ParseConfig"
    );
    assert_eq!(
        canonical_ts_type_name("wasm", "WasmParseConfig", &resolved),
        "WasmParseConfig"
    );
}

#[test]
fn wasm_visitor_binding_uses_trait_bridge_options_metadata() {
    use crate::core::config::{BridgeBinding, ResolvedCrateConfig, TraitBridgeConfig};

    let config = ResolvedCrateConfig {
        trait_bridges: vec![TraitBridgeConfig {
            trait_name: "Renderer".to_string(),
            type_alias: Some("RenderHandle".to_string()),
            param_name: Some("renderer".to_string()),
            bind_via: BridgeBinding::OptionsField,
            options_type: Some("RenderOptions".to_string()),
            options_field: Some("callback".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    };

    let binding = wasm_visitor_binding(&config, None).expect("visitor binding");
    assert_eq!(binding.options_type, "WasmRenderOptions");
    assert_eq!(binding.options_field, "callback");
    assert_eq!(binding.handle_type, "WasmRenderHandle");
}

#[test]
fn wasm_visitor_arg_uses_configured_field_and_types() {
    let binding = WasmVisitorBinding {
        options_type: "WasmRenderOptions".to_string(),
        options_field: "callback".to_string(),
        handle_type: "WasmRenderHandle".to_string(),
    };

    let args = apply_wasm_visitor_arg("html, undefined", "_visitor", &binding);
    assert!(
        args.contains("WasmRenderOptions.default()"),
        "options type must come from metadata, got:\n{args}"
    );
    assert!(
        args.contains("_u.callback = new WasmRenderHandle(_visitor);"),
        "visitor field and handle type must come from metadata, got:\n{args}"
    );
    assert!(
        !args.contains("WasmConversionOptions") && !args.contains("WasmVisitorHandle"),
        "must not hard-code conversion visitor names, got:\n{args}"
    );
}

#[test]
fn resolve_node_function_name_converts_snake_to_camel() {
    use crate::e2e::config::CallConfig;
    let cc = CallConfig {
        function: "process_text".to_string(),
        ..Default::default()
    };
    assert_eq!(resolve_node_function_name(&cc), "processText");
}

#[test]
fn ts_method_helper_import_recognizes_has_error_nodes() {
    assert_eq!(
        ts_method_helper_import("has_error_nodes"),
        Some("treeHasErrorNodes".to_string())
    );
}

#[test]
fn ts_method_helper_import_returns_none_for_unknown() {
    assert!(ts_method_helper_import("some_unknown_method").is_none());
}

#[test]
fn sanitize_filename_produces_expected_names() {
    let groups = [
        FixtureGroup {
            category: "basic tests".to_string(),
            fixtures: vec![],
        },
        FixtureGroup {
            category: "edge cases".to_string(),
            fixtures: vec![],
        },
    ];
    let names: Vec<String> = groups
        .iter()
        .map(|g| format!("{}.test.ts", sanitize_filename(&g.category)))
        .collect();
    assert_eq!(names, vec!["basic_tests.test.ts", "edge_cases.test.ts"]);
}

/// An HTTP-only test file whose fixture has a JSON body assertion must still emit
/// `_alefE2eDecompressAndParseJson` in the helper_functions block.  The previous
/// implementation only emitted the helper when `has_non_http_fixtures` was true,
/// causing "cannot find function" compile errors for HTTP-only categories with
/// JSON response bodies, partial bodies, or validation-error assertions.
#[test]
fn http_only_test_file_with_json_body_emits_decompress_helper() {
    use crate::e2e::config::E2eConfig;
    use crate::e2e::fixture::{Fixture, HttpExpectedResponse, HttpFixture, HttpHandler, HttpRequest};

    let fixture = Fixture {
        id: "get_user_returns_json".to_string(),
        category: Some("users".to_string()),
        description: "GET /user returns JSON object".to_string(),
        tags: vec![],
        skip: None,
        env: None,
        setup: Vec::new(),
        call: None,
        input: serde_json::Value::Null,
        mock_response: None,
        visitor: None,
        args: vec![],
        assertion_recipes: vec![],
        assertions: vec![],
        source: String::new(),
        http: Some(HttpFixture {
            handler: HttpHandler {
                route: "/user".to_string(),
                method: "GET".to_string(),
                body_schema: None,
                parameters: Default::default(),
                middleware: None,
            },
            request: HttpRequest {
                method: "GET".to_string(),
                path: "/user".to_string(),
                headers: Default::default(),
                query_params: Default::default(),
                cookies: Default::default(),
                body: None,
                form_data: None,
                content_type: None,
            },
            expected_response: HttpExpectedResponse {
                status_code: 200,
                body: Some(serde_json::json!({"id": 1, "name": "Alice"})),
                body_partial: None,
                headers: Default::default(),
                validation_errors: None,
            },
        }),
    };

    let fixtures = vec![&fixture];
    let e2e_config = E2eConfig::default();
    let config = crate::core::config::ResolvedCrateConfig::default();

    let output = render_test_file(
        "node",
        "users",
        &fixtures,
        "",
        "my-lib",
        "processText",
        &[],
        None,
        None,
        &e2e_config,
        &[],
        &[],
        "",
        &config,
    );

    assert!(
        output.contains("_alefE2eDecompressAndParseJson"),
        "HTTP-only test file with JSON body must emit _alefE2eDecompressAndParseJson helper;\n\
             actual output:\n{output}"
    );
}

#[test]
fn render_env_setup_empty_env_returns_empty_string() {
    use crate::e2e::codegen::typescript::test_file::render::render_env_setup;
    let env = std::collections::HashMap::new();
    let output = render_env_setup(&env);
    assert_eq!(output, "", "empty env must return empty string");
}

#[test]
fn render_env_setup_single_var() {
    use crate::e2e::codegen::typescript::test_file::render::render_env_setup;
    let mut env = std::collections::HashMap::new();
    env.insert("TEST_VAR".to_string(), "test_value".to_string());
    let output = render_env_setup(&env);
    assert!(
        output.contains("process.env.TEST_VAR ??= \"test_value\";"),
        "output must contain process.env assignment; got: {output}"
    );
}

#[test]
fn render_env_setup_multiple_vars_sorted_alphabetically() {
    use crate::e2e::codegen::typescript::test_file::render::render_env_setup;
    let mut env = std::collections::HashMap::new();
    env.insert("ZEBRA".to_string(), "value1".to_string());
    env.insert("APPLE".to_string(), "value2".to_string());
    env.insert("BANANA".to_string(), "value3".to_string());
    let output = render_env_setup(&env);

    let apple_idx = output.find("APPLE").expect("must contain APPLE");
    let banana_idx = output.find("BANANA").expect("must contain BANANA");
    let zebra_idx = output.find("ZEBRA").expect("must contain ZEBRA");

    assert!(
        apple_idx < banana_idx && banana_idx < zebra_idx,
        "env vars must be sorted alphabetically; got: {output}"
    );
}

#[test]
fn render_env_setup_uses_defaultassign_semantics() {
    use crate::e2e::codegen::typescript::test_file::render::render_env_setup;
    let mut env = std::collections::HashMap::new();
    env.insert("E2E_ALLOW_PRIVATE_NETWORK".to_string(), "true".to_string());
    let output = render_env_setup(&env);

    assert!(
        output.contains("??="),
        "must use ??= operator for setdefault semantics; got: {output}"
    );
}
