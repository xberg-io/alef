use super::test_case::escape_js_regex_literal;
use super::visitor::WasmVisitorBinding;
use super::*;
use crate::core::ir::{FieldDef, PrimitiveType};
use crate::e2e::escape::sanitize_filename;
use crate::e2e::fixture::FixtureGroup;

pub(super) fn make_field(name: &str, ty: TypeRef) -> FieldDef {
    FieldDef {
        version: Default::default(),
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
        serde_with: None,
        serde_skip_serializing_if: false,
        serde_skip: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    }
}

pub(super) fn make_type(name: &str, fields: Vec<FieldDef>) -> TypeDef {
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
        serde_container_default: false,
        serde_container_conversion: Default::default(),
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
    let derived = collect_transitive_nested_types_for_wasm(&seeds, &type_defs, "Wasm", &Default::default());

    // `derived` is a `BTreeSet<String>` of wasm class names (not a field-name
    // keyed map) — see the doc comment on `collect_transitive_nested_types_for_wasm`
    // for why keying by field name was dropped (colliding same-named fields on
    // different classes made the old map non-deterministic).
    assert!(
        derived.contains("WasmChatTool"),
        "first-level WasmChatTool missing; got {:?}",
        derived
    );
    assert!(
        derived.contains("WasmFunctionDefinition"),
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
        &[],
        "",
        0,
        &mut Default::default(),
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
        &[],
        "",
        0,
        &mut Default::default(),
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
    let derived = collect_transitive_nested_types_for_wasm(&seeds, &[recursive], "Wasm", &Default::default());
    // `derived` is a set of class names; the self-referential field's class
    // (WasmRecursive) must appear exactly once despite the cycle.
    assert_eq!(derived.len(), 1);
    assert!(derived.contains("WasmRecursive"));
}

/// Regression for the field-name-collision bug: two distinct classes
/// (`TesseractConfig` and `ConversionOptions`) each expose a field literally
/// named `preprocessing`, but with different nested class types
/// (`ImagePreprocessingConfig` vs `PreprocessingOptions`). The old
/// `HashMap<field_name, class_name>` shape let one collide with and silently
/// drop the other depending on non-deterministic `HashMap` iteration order.
/// The `BTreeSet<class_name>` shape must retain both.
#[test]
fn collect_transitive_nested_types_keeps_both_classes_on_field_name_collision() {
    let image_preprocessing_config = make_type("ImagePreprocessingConfig", vec![]);
    let preprocessing_options = make_type("PreprocessingOptions", vec![]);
    let tesseract_config = make_type(
        "TesseractConfig",
        vec![make_field(
            "preprocessing",
            TypeRef::Optional(Box::new(TypeRef::Named("ImagePreprocessingConfig".to_string()))),
        )],
    );
    let conversion_options = make_type(
        "ConversionOptions",
        vec![make_field(
            "preprocessing",
            TypeRef::Named("PreprocessingOptions".to_string()),
        )],
    );
    let type_defs = vec![
        image_preprocessing_config,
        preprocessing_options,
        tesseract_config,
        conversion_options,
    ];

    let mut seeds = std::collections::BTreeSet::new();
    seeds.insert("WasmTesseractConfig".to_string());
    seeds.insert("WasmConversionOptions".to_string());
    let derived = collect_transitive_nested_types_for_wasm(&seeds, &type_defs, "Wasm", &Default::default());

    assert!(
        derived.contains("WasmImagePreprocessingConfig"),
        "colliding field name must not drop WasmImagePreprocessingConfig; got {:?}",
        derived
    );
    assert!(
        derived.contains("WasmPreprocessingOptions"),
        "colliding field name must not drop WasmPreprocessingOptions; got {:?}",
        derived
    );
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
    e2e_config.call.overrides.insert(
        "wasm".into(),
        crate::e2e::config::CallOverride {
            enum_fields: [("kind".into(), "ExtractInputKind".into())].into_iter().collect(),
            ..Default::default()
        },
    );

    let fixture = Fixture {
        docs: None,
        requirements: Vec::new(),
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
            skip: None,
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
    let enums = [EnumDef {
        name: "ExtractInputKind".into(),
        ..Default::default()
    }];
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
        &enums,
        &[],
        "Wasm",
        &config,
        &[],
    );

    assert!(
        output.contains("WasmFileExtractionConfig"),
        "WASM imports must include nested DTOs reached through json_object element types;\n{output}"
    );

    // The bare `element_type = "ExtractInput"` names a wasm-wrapped struct, so
    // the constructor reference is prefixed (`WasmExtractInput.default()`). The
    // import statement must reference the same prefixed name or the test throws
    // `ReferenceError: WasmExtractInput is not defined` at runtime.
    let import_line = output
        .lines()
        .find(|l| l.starts_with("import") && l.contains("@test/wasm"))
        .expect("wasm test file must have a binding import line");
    assert!(
        import_line.contains("WasmExtractInput"),
        "import line must reference the prefixed input class;\n{import_line}"
    );
    assert!(
        import_line.contains("WasmExtractInputKind"),
        "import line must prefix enum classes referenced by the input;\n{import_line}"
    );
    assert!(
        !import_line.split([',', '{', '}', ' ']).any(|tok| tok == "ExtractInput"),
        "import line must NOT reference the bare, unprefixed input class;\n{import_line}"
    );
    assert!(
        output.contains("WasmExtractInput.default()"),
        "constructor reference must use the prefixed input class;\n{output}"
    );
}

#[test]
fn wasm_prefixed_wrapped_type_prefixes_known_structs_and_enums() {
    let struct_def = make_type("ExtractInput", vec![]);
    let enum_def = crate::core::ir::EnumDef {
        name: "OutputFormat".to_string(),
        ..Default::default()
    };
    let type_defs = [struct_def];
    let enums = [enum_def];

    // Known wrapped struct → prefixed.
    assert_eq!(
        wasm_prefixed_wrapped_type("wasm", "ExtractInput", &type_defs, &enums, "Wasm"),
        "WasmExtractInput"
    );
    // Known wrapped enum → prefixed.
    assert_eq!(
        wasm_prefixed_wrapped_type("wasm", "OutputFormat", &type_defs, &enums, "Wasm"),
        "WasmOutputFormat"
    );
    // Already prefixed → unchanged (no double prefix).
    assert_eq!(
        wasm_prefixed_wrapped_type("wasm", "WasmExtractInput", &type_defs, &enums, "Wasm"),
        "WasmExtractInput"
    );
    // Unknown / host type → unchanged.
    assert_eq!(
        wasm_prefixed_wrapped_type("wasm", "Uint8Array", &type_defs, &enums, "Wasm"),
        "Uint8Array"
    );
    // Non-wasm language → never prefixed, even for a known struct.
    assert_eq!(
        wasm_prefixed_wrapped_type("node", "ExtractInput", &type_defs, &enums, "Wasm"),
        "ExtractInput"
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
fn resolve_node_function_name_converts_to_lower_camel_case() {
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
        docs: None,
        requirements: Vec::new(),
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
        asyncapi: None,
        websocket: None,
        preserve_input_urls: false,
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
        &[],
        "",
        &config,
        &[],
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

/// Regression: a fixture whose request declares `multipart/form-data` but carries a
/// plain JSON *object* body (a multipart param with no synthesized body) must be sent
/// as a JSON.stringify'd body with `application/json`, NOT with the synthesized
/// multipart boundary Content-Type — that header on a JSON body makes the server's
/// multipart parser reject the request with 400 before the handler runs. Mirrors the
/// Python generator's else-branch.
#[test]
fn multipart_param_with_json_object_body_does_not_emit_boundary_content_type() {
    use crate::e2e::fixture::{Fixture, HttpExpectedResponse, HttpFixture, HttpHandler, HttpRequest};

    let fixture = Fixture {
        docs: None,
        requirements: Vec::new(),
        id: "upload_file_basic".to_string(),
        category: Some("upload".to_string()),
        description: "upload a file".to_string(),
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
                route: "/upload".to_string(),
                method: "POST".to_string(),
                body_schema: None,
                parameters: Default::default(),
                middleware: None,
            },
            request: HttpRequest {
                method: "POST".to_string(),
                path: "/upload".to_string(),
                headers: Default::default(),
                query_params: Default::default(),
                cookies: Default::default(),
                body: Some(serde_json::json!({"file": {"content": "hi", "filename": "a.txt"}})),
                form_data: None,
                content_type: Some("multipart/form-data".to_string()),
            },
            expected_response: HttpExpectedResponse {
                status_code: 200,
                body: Some(serde_json::json!({"filename": "a.txt"})),
                body_partial: None,
                headers: Default::default(),
                validation_errors: None,
            },
        }),
        asyncapi: None,
        websocket: None,
        preserve_input_urls: false,
    };

    let mut out = String::new();
    super::http::render_http_test_case(&mut out, &fixture);

    assert!(
        !out.contains("boundary=alef-boundary"),
        "a JSON object body must NOT get the multipart boundary Content-Type; got:\n{out}"
    );
    assert!(
        out.contains("JSON.stringify"),
        "a JSON object body must be JSON.stringify'd; got:\n{out}"
    );
    assert!(
        out.contains("application/json"),
        "a JSON object body must declare application/json; got:\n{out}"
    );
}

/// Regression: a multipart fixture with no explicit body but a `body_schema` still
/// synthesizes a real multipart string body and MUST carry the boundary Content-Type
/// (sent via Buffer.from as raw bytes).
#[test]
fn multipart_synthesized_body_emits_boundary_content_type() {
    use crate::e2e::fixture::{Fixture, HttpExpectedResponse, HttpFixture, HttpHandler, HttpRequest};

    let fixture = Fixture {
        docs: None,
        requirements: Vec::new(),
        id: "upload_synth".to_string(),
        category: Some("upload".to_string()),
        description: "synthesized multipart".to_string(),
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
                route: "/upload".to_string(),
                method: "POST".to_string(),
                body_schema: Some(serde_json::json!({
                    "type": "object",
                    "properties": {"file": {"type": "string", "format": "binary"}}
                })),
                parameters: Default::default(),
                middleware: None,
            },
            request: HttpRequest {
                method: "POST".to_string(),
                path: "/upload".to_string(),
                headers: Default::default(),
                query_params: Default::default(),
                cookies: Default::default(),
                body: None,
                form_data: None,
                content_type: Some("multipart/form-data".to_string()),
            },
            expected_response: HttpExpectedResponse {
                status_code: 200,
                body: None,
                body_partial: None,
                headers: Default::default(),
                validation_errors: None,
            },
        }),
        asyncapi: None,
        websocket: None,
        preserve_input_urls: false,
    };

    let mut out = String::new();
    super::http::render_http_test_case(&mut out, &fixture);

    assert!(
        out.contains("boundary=alef-boundary"),
        "a synthesized multipart body must carry the boundary Content-Type; got:\n{out}"
    );
    assert!(
        out.contains("Buffer.from"),
        "a synthesized multipart body must be sent as raw bytes via Buffer.from; got:\n{out}"
    );
}

/// Two IR types extracted from different modules can share a bare `name`
/// (e.g. two distinct `Config` structs). `derive_nested_types_for_wasm` must
/// resolve such a collision the same way regardless of where each same-named
/// entry sits in the `type_defs` slice, otherwise the generated wasm import
/// line silently swaps one imported class for another across regen runs
/// whenever the upstream type registry's order shifts (see the doc comment
/// on `derive_nested_types_for_wasm`).
fn make_type_at_path(name: &str, rust_path: &str, fields: Vec<FieldDef>) -> TypeDef {
    let mut type_def = make_type(name, fields);
    type_def.rust_path = rust_path.to_string();
    type_def
}

#[test]
fn derive_nested_types_resolves_duplicate_names_deterministically_by_rust_path() {
    let field = make_field("nested", TypeRef::Named("Config".to_string()));
    let request_type = make_type("Request", vec![field]);

    let config_a = make_type_at_path("Config", "crate::module_a::Config", vec![]);
    let config_b = make_type_at_path("Config", "crate::module_b::Config", vec![]);

    let forward_order = vec![request_type.clone(), config_a.clone(), config_b.clone()];
    let reverse_order = vec![request_type, config_b, config_a];

    let derived_forward = derive_nested_types_for_wasm("WasmRequest", &forward_order, "Wasm");
    let derived_reverse = derive_nested_types_for_wasm("WasmRequest", &reverse_order, "Wasm");

    assert_eq!(
        derived_forward, derived_reverse,
        "duplicate-name resolution must not depend on type_defs slice order"
    );
    // `crate::module_a::Config` sorts before `crate::module_b::Config`, so the
    // tie always breaks toward module_a regardless of input order.
    assert_eq!(derived_forward.get("nested"), Some(&"WasmConfig".to_string()));
}

fn error_fixture(id: &str, error_value: Option<serde_json::Value>) -> Fixture {
    let assertion = crate::e2e::fixture::Assertion {
        assertion_type: "error".to_string(),
        value: error_value,
        ..Default::default()
    };
    Fixture {
        id: id.to_string(),
        category: Some("thing".to_string()),
        description: "declared-error fixture".to_string(),
        input: serde_json::json!({}),
        assertions: vec![assertion],
        ..Default::default()
    }
}

fn render_error_fixture(fixture: &Fixture) -> String {
    render_error_fixture_with_errors(fixture, &[])
}

fn render_error_fixture_with_errors(fixture: &Fixture, errors: &[crate::core::ir::ErrorDef]) -> String {
    let mut e2e_config = E2eConfig::default();
    e2e_config.call.function = "doThing".to_string();
    let config = crate::core::config::ResolvedCrateConfig::default();
    let fixtures = vec![fixture];

    render_test_file(
        "node",
        "thing",
        &fixtures,
        "",
        "my-lib",
        "doThing",
        &[],
        None,
        None,
        &e2e_config,
        &[],
        &[],
        &[],
        "",
        &config,
        errors,
    )
}

#[test]
fn error_assertion_without_declared_value_keeps_plain_reject_matcher() {
    let fixture = error_fixture("thing_fails", None);
    let output = render_error_fixture(&fixture);

    assert!(
        output.contains("\t\t}).rejects.toThrow();\n"),
        "no declared error value must keep the unchanged `.rejects.toThrow()` form;\n{output}"
    );
    assert!(
        !output.contains("toSatisfy"),
        "no declared error value must not emit a toSatisfy matcher;\n{output}"
    );
}

#[test]
fn error_assertion_with_declared_value_checks_message_or_name() {
    let fixture = error_fixture("thing_fails_with_bad_request", Some(serde_json::json!("BadRequest")));
    let output = render_error_fixture(&fixture);

    assert!(
        !output.contains("\t\t}).rejects.toThrow();\n"),
        "a declared error value must replace the unconditional `.rejects.toThrow()`;\n{output}"
    );
    assert!(
        output.contains("}).rejects.toSatisfy((error) => {"),
        "a declared error value must switch to the toSatisfy matcher;\n{output}"
    );
    assert!(
        output.contains("return /BadRequest/.test(_message) || /BadRequest/.test(_name);"),
        "the matcher must check the declared value against EITHER the message OR the name;\n{output}"
    );
}

#[test]
fn error_assertion_value_with_regex_metacharacters_is_escaped() {
    let fixture = error_fixture("thing_fails_with_metachars", Some(serde_json::json!("field(a.b)+")));
    let output = render_error_fixture(&fixture);

    assert!(
        output.contains(r"return /field\(a\.b\)\+/.test(_message) || /field\(a\.b\)\+/.test(_name);"),
        "regex metacharacters in the declared value must be escaped so the pattern \
         matches the value literally rather than as a regex;\n{output}"
    );
}

/// The defect this fix closes: a declared value naming a real `ErrorVariant` — every NAPI
/// throw site is `napi::Error::new(Status::GenericFailure, e.to_string())`, generic status and
/// name, message only — must render the registered skip, not a `toSatisfy` matcher that can
/// never pass.
#[test]
fn error_assertion_with_a_known_variant_node_cannot_substantiate_is_skipped() {
    let fixture = error_fixture("thing_fails_with_auth", Some(serde_json::json!("Authentication")));
    let errors = vec![crate::core::ir::ErrorDef {
        name: "ApiError".to_string(),
        rust_path: "lib::ApiError".to_string(),
        original_rust_path: String::new(),
        variants: vec![crate::core::ir::ErrorVariant {
            name: "Authentication".to_string(),
            error_code: Some(100),
            is_unit: true,
            ..crate::core::ir::ErrorVariant::default()
        }],
        doc: String::new(),
        methods: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }];
    let output = render_error_fixture_with_errors(&fixture, &errors);

    assert!(
        output.contains("\t\t}).rejects.toThrow();\n"),
        "the call must still be proven to fail (the unconditional toThrow form), got:\n{output}"
    );
    assert!(
        !output.contains("toSatisfy"),
        "must not render a matcher that can never pass, got:\n{output}"
    );
    assert!(
        output.contains(
            "\t\t// skipped: declared error variant 'Authentication' not yet preserved as a distinct identity by \
             this backend's generator"
        ),
        "got:\n{output}"
    );
}

#[test]
fn escape_js_regex_literal_escapes_metacharacters_and_delimiter() {
    assert_eq!(escape_js_regex_literal("plain"), "plain");
    assert_eq!(
        escape_js_regex_literal(r"a.b*c+d?e^f$g{h}i(j)k|l[m]n\o/p"),
        r"a\.b\*c\+d\?e\^f\$g\{h\}i\(j\)k\|l\[m\]n\\o\/p"
    );
    assert_eq!(
        escape_js_regex_literal("line1\nline2\ttab\rcr"),
        r"line1\nline2\ttab\rcr"
    );
}

/// Regression test for alef task #81: TypeScript's "skipped: field not available" comment
/// must carry the exact marker text the shared `fail_on_unavailable_field_markers`
/// mechanism (src/e2e/codegen/mod.rs) matches on, so arming
/// `ALEF_E2E_STRICT_FIELD_AVAILABILITY` turns a dropped field assertion into a
/// generation-time failure. The arming behaviour itself is proven in `mod.rs`'s
/// `unavailable_field_marker_tests`; this test only pins the marker text TypeScript emits
/// through the real per-fixture rendering entry point (`render_test_file`), which also
/// covers wasm since `wasm.rs` delegates to this same function. ~keep
#[test]
fn dropped_field_assertion_carries_the_marker_that_arms_the_strict_mode() {
    let mut e2e_config = crate::e2e::config::E2eConfig::default();
    e2e_config.call.function = "process".to_string();
    e2e_config.call.result_var = "result".to_string();
    e2e_config.call.result_fields = std::collections::HashSet::from(["content".to_string()]);
    e2e_config.call.returns_result = true;

    let fixture = Fixture {
        docs: None,
        requirements: Vec::new(),
        id: "process_smoke".to_string(),
        description: "test".to_string(),
        input: serde_json::Value::Null,
        assertions: vec![crate::e2e::fixture::Assertion {
            skip: None,
            assertion_type: "equals".to_string(),
            field: Some("nonexistent_field".to_string()),
            value: Some(serde_json::json!("x")),
            values: None,
            method: None,
            check: None,
            args: None,
            return_type: None,
        }],
        ..Default::default()
    };

    let config = crate::core::config::ResolvedCrateConfig::default();

    let output = render_test_file(
        "typescript",
        "smoke",
        &[&fixture],
        "",
        "@test/pkg",
        "process",
        &[],
        None,
        None,
        &e2e_config,
        &[],
        &[],
        &[],
        "",
        &config,
        &[],
    );

    assert!(
        output.contains("field 'nonexistent_field' not available on result type"),
        "got:\n{output}"
    );
}

/// Regression test for alef task #81, tightened by #233: a fixture whose sole declared assertion
/// resolved to a "skipped" comment (its field is unavailable on the result type) produced an
/// entirely comment-only, vacuously-passing test body. The generic `expect(result).toBeDefined()`
/// fallback that first closed it is not enough for THIS cause: the fixture named a check a config
/// or fixture edit would make run, so a fallback that passes in its place is the green test the
/// refusal exists to stop. It must now emit an expectation that FAILS and names the fixture, with
/// the marker carried alongside it. ~keep
#[test]
fn dropped_field_assertion_is_refused_with_an_expectation_that_fails() {
    let mut e2e_config = crate::e2e::config::E2eConfig::default();
    e2e_config.call.function = "process".to_string();
    e2e_config.call.result_var = "result".to_string();
    e2e_config.call.result_fields = std::collections::HashSet::from(["content".to_string()]);
    e2e_config.call.returns_result = true;

    let fixture = Fixture {
        docs: None,
        requirements: Vec::new(),
        id: "process_smoke".to_string(),
        description: "test".to_string(),
        input: serde_json::Value::Null,
        assertions: vec![crate::e2e::fixture::Assertion {
            skip: None,
            assertion_type: "equals".to_string(),
            field: Some("nonexistent_field".to_string()),
            value: Some(serde_json::json!("x")),
            values: None,
            method: None,
            check: None,
            args: None,
            return_type: None,
        }],
        ..Default::default()
    };

    let config = crate::core::config::ResolvedCrateConfig::default();

    let output = render_test_file(
        "typescript",
        "smoke",
        &[&fixture],
        "",
        "@test/pkg",
        "process",
        &[],
        None,
        None,
        &e2e_config,
        &[],
        &[],
        &[],
        "",
        &config,
        &[],
    );

    assert!(
        output.contains("const unresolvedAssertion = \"alef resolved no assertion for fixture `process_smoke`")
            && output.contains("expect(unresolvedAssertion).toBeNull();"),
        "a consumer-fixable gap must be refused with an expectation that fails, got:\n{output}"
    );
    assert!(
        output.contains("field 'nonexistent_field' not available on result type"),
        "the marker must be carried into the refusal, not replaced by silence, got:\n{output}"
    );
    assert!(
        !output.contains("expect(result).toBeDefined();"),
        "a fallback that always passes must not stand in for the refused assertion, got:\n{output}"
    );
    assert!(
        !output.contains("it.skip("),
        "a consumer-fixable gap must not be parked as skipped, got:\n{output}"
    );
    let refusals = crate::e2e::codegen::inert_example::take_inert_examples();
    assert_eq!(refusals.len(), 1, "the refusal must be recorded once for the summary");
    assert_eq!(refusals[0].fixture_id, "process_smoke");
}

/// Positive control for the same fix: a fixture with genuinely zero declared
/// assertions is left untouched (deliberate "just call it" smoke test).
#[test]
fn zero_declared_assertions_are_left_untouched() {
    let mut e2e_config = crate::e2e::config::E2eConfig::default();
    e2e_config.call.function = "process".to_string();
    e2e_config.call.result_var = "result".to_string();
    e2e_config.call.returns_result = true;

    let fixture = Fixture {
        docs: None,
        requirements: Vec::new(),
        id: "process_smoke".to_string(),
        description: "test".to_string(),
        input: serde_json::Value::Null,
        assertions: Vec::new(),
        ..Default::default()
    };

    let config = crate::core::config::ResolvedCrateConfig::default();

    let output = render_test_file(
        "typescript",
        "smoke",
        &[&fixture],
        "",
        "@test/pkg",
        "process",
        &[],
        None,
        None,
        &e2e_config,
        &[],
        &[],
        &[],
        "",
        &config,
        &[],
    );

    assert!(
        !output.contains("expect(result).toBeDefined();"),
        "a fixture with zero declared assertions must stay vacuous, got:\n{output}"
    );
}

fn node_client_snippet(expects_error: bool) -> String {
    let mut fixture = Fixture {
        id: "rate_limit_429".to_string(),
        description: "Rate limited".to_string(),
        input: serde_json::Value::Null,
        ..Default::default()
    };
    if expects_error {
        fixture.assertions = serde_json::from_value(serde_json::json!([{"type": "error"}])).expect("assertions");
    }
    let mut e2e_config = crate::e2e::config::E2eConfig::default();
    e2e_config.call.function = "chat".into();
    e2e_config.call.result_var = "result".into();
    e2e_config.call.r#async = true;
    let config = crate::core::config::ResolvedCrateConfig::default();

    render_snippet_body(SnippetContext {
        lang: "node",
        fixture: &fixture,
        module: "@test/node",
        client_factory: Some("createClient"),
        e2e_config: &e2e_config,
        type_defs: &[],
        enums: &[],
        functions: &[],
        wasm_type_prefix: "Wasm",
        config: &config,
    })
}

/// The negative control that keeps the WASM client release off node: both languages share this
/// renderer and `typescript/snippet_body.jinja`, and node has nothing to release — alef's napi
/// wrapper emits an empty `impl`, so the generated `DefaultClient` exposes no `free`, `close`, or
/// `Symbol.dispose`. A node snippet that constructs a client must therefore be byte-for-byte what
/// it was: no release call, no `finally` scope, and the two-space body indentation. This is the
/// pin that a `lang == "wasm"` gate exists at all; without it the fix silently reaches 188 node
/// snippets and emits a call to a method that does not exist. ~keep
#[test]
fn node_client_snippet_gains_no_release() {
    let body = node_client_snippet(false);

    assert!(
        !body.contains(".free()") && !body.contains(".close()") && !body.contains("dispose"),
        "node has no client release surface to call:\n{body}"
    );
    assert!(!body.contains("finally"), "node must not gain a release scope:\n{body}");
    assert!(
        body.contains("  const result = await client.chat("),
        "node must keep its two-space body indentation:\n{body}"
    );
}

/// The `expects_error` half of `node_client_snippet_gains_no_release`: that arm is where the
/// release scope is attached for WASM, so pin that node's `try`/`catch` gains no `finally`. ~keep
#[test]
fn node_expects_error_snippet_gains_no_release_scope() {
    let body = node_client_snippet(true);

    assert!(body.contains("} catch (error) {"), "{body}");
    assert!(
        !body.contains("finally"),
        "node must not gain a release scope on the error path:\n{body}"
    );
    assert!(
        !body.contains(".free()"),
        "node has no client release surface to call:\n{body}"
    );
}

/// TypeScript's `expects_error` branch renders the `rejects` matcher and returns, so every other
/// assertion on the fixture used to leave no trace in the generated test at all.
#[test]
fn typescript_equals_on_an_error_field_is_named_instead_of_dropped() {
    let mut fixture = error_fixture("rate_limited", Some(serde_json::json!("BadRequest")));
    fixture.assertions.push(crate::e2e::fixture::Assertion {
        assertion_type: "equals".to_string(),
        field: Some("error.status_code".to_string()),
        ..Default::default()
    });

    let _ = crate::e2e::codegen::take_skip_records();
    let output = render_error_fixture(&fixture);

    // Positive first: the error block really rendered.
    assert!(
        output.contains("}).rejects.toSatisfy("),
        "the error block must render: {output}"
    );
    assert!(
        output.contains(
            "// skipped: assertion type 'equals' has no accessor for error field error.status_code in this backend"
        ),
        "{output}"
    );

    let records = crate::e2e::codegen::take_skip_records();
    assert_eq!(records.len(), 1, "got: {records:?}");
    assert_eq!(records[0].language, "node");
    assert_eq!(records[0].field, "equals");
}

/// Negative control: a lone `error` assertion must leave the generated file marker-free.
#[test]
fn typescript_a_lone_error_assertion_renders_no_marker() {
    let fixture = error_fixture("thing_fails", None);
    let output = render_error_fixture(&fixture);

    assert!(
        output.contains("}).rejects.toThrow();"),
        "the error block must render: {output}"
    );
    assert!(!output.contains("has no accessor for error field"), "{output}");
}

/// CONTROL for the refusal wired in #233, asserted before any absence assertion: a field the
/// availability oracle resolves still renders its real check, nothing is refused, and no skip is
/// emitted. An over-broad refusal here would silently delete coverage that runs today — the same
/// defect pointing the other way. ~keep
#[test]
fn a_resolvable_field_assertion_is_published_unchanged_and_never_refused() {
    let _ = crate::e2e::codegen::inert_example::take_inert_examples();
    let mut e2e_config = crate::e2e::config::E2eConfig::default();
    e2e_config.call.function = "process".to_string();
    e2e_config.call.result_var = "result".to_string();
    e2e_config.call.result_fields = std::collections::HashSet::from(["content".to_string()]);
    e2e_config.call.returns_result = true;

    let fixture = Fixture {
        id: "ts_control".to_string(),
        description: "test".to_string(),
        assertions: vec![crate::e2e::fixture::Assertion {
            assertion_type: "equals".to_string(),
            field: Some("content".to_string()),
            value: Some(serde_json::json!("hello")),
            ..Default::default()
        }],
        ..Default::default()
    };
    let config = crate::core::config::ResolvedCrateConfig::default();

    let output = render_test_file(
        "typescript",
        "smoke",
        &[&fixture],
        "",
        "@test/pkg",
        "process",
        &[],
        None,
        None,
        &e2e_config,
        &[],
        &[],
        &[],
        "",
        &config,
        &[],
    );

    assert!(
        output.contains("expect(result.content"),
        "the renderable assertion must still be emitted, got:\n{output}"
    );
    assert!(
        !output.contains("it.skip(") && !output.contains("unresolvedAssertion"),
        "a live example must not be refused, got:\n{output}"
    );
    assert!(
        crate::e2e::codegen::inert_example::take_inert_examples().is_empty(),
        "nothing may be recorded for a live example"
    );
}

/// A streaming fixture has no honest fallback subject of its own: `chunks` is bound to a freshly
/// drained array immediately above, so `expect(chunks).toBeDefined()` cannot fail. When every
/// declared assertion funnels into a marker that alef itself owns, the example must come out as
/// vitest's own `it.skip` — never as a passing test — and the markers must come with it.
#[test]
fn a_streaming_example_whose_every_assertion_skips_is_refused_as_a_skipped_test() {
    let _ = crate::e2e::codegen::inert_example::take_inert_examples();
    let mut e2e_config = crate::e2e::config::E2eConfig::default();
    e2e_config.call.function = "process".to_string();
    e2e_config.call.result_var = "result".to_string();
    e2e_config.call.result_fields = std::collections::HashSet::from(["content".to_string()]);
    e2e_config.call.returns_result = true;
    e2e_config.call.streaming = Some(crate::core::config::e2e::StreamingConfig::Enabled(true));

    let fixture = Fixture {
        id: "ts_stream_all_skipped".to_string(),
        description: "test".to_string(),
        assertions: vec![crate::e2e::fixture::Assertion {
            assertion_type: "equals".to_string(),
            field: Some("nonexistent_field".to_string()),
            value: Some(serde_json::json!("x")),
            skip: Some(crate::e2e::fixture::AssertionSkip::All(true)),
            ..Default::default()
        }],
        ..Default::default()
    };
    let config = crate::core::config::ResolvedCrateConfig::default();

    let output = render_test_file(
        "typescript",
        "smoke",
        &[&fixture],
        "",
        "@test/pkg",
        "process",
        &[],
        None,
        None,
        &e2e_config,
        &[],
        &[],
        &[],
        "",
        &config,
        &[],
    );

    assert!(
        output.contains("it.skip(\"ts_stream_all_skipped: test\""),
        "the refusal must be vitest's own skip, got:\n{output}"
    );
    assert!(
        output.contains("// skipped: field 'nonexistent_field' not available on result type"),
        "the marker must be carried into the refusal, not replaced by silence, got:\n{output}"
    );
    assert!(
        output.contains("// alef rendered no runnable expectation for fixture `ts_stream_all_skipped`"),
        "the refusal must name why it could not run, got:\n{output}"
    );
    assert!(
        !output.contains("expect(chunks).toBeDefined();"),
        "a guard that cannot fail must not stand in for the refused assertions, got:\n{output}"
    );
    let refusals = crate::e2e::codegen::inert_example::take_inert_examples();
    assert_eq!(refusals.len(), 1, "the refusal must be recorded once for the summary");
    assert_eq!(refusals[0].fixture_id, "ts_stream_all_skipped");
}

/// CONTROL: alef's own acknowledged debt on a NON-streaming call keeps the
/// `expect(result).toBeDefined()` fallback. That check can genuinely fail — a binding returning
/// `undefined` trips it — so refusing it would delete the "the call worked" coverage it carries,
/// and the marker naming what could not run must survive beside it. ~keep
#[test]
fn acknowledged_debt_on_a_non_streaming_call_keeps_its_failable_fallback() {
    let _ = crate::e2e::codegen::inert_example::take_inert_examples();
    let mut e2e_config = crate::e2e::config::E2eConfig::default();
    e2e_config.call.function = "process".to_string();
    e2e_config.call.result_var = "result".to_string();
    e2e_config.call.result_fields = std::collections::HashSet::from(["content".to_string()]);
    e2e_config.call.returns_result = true;

    let fixture = Fixture {
        id: "ts_generator_debt".to_string(),
        description: "test".to_string(),
        assertions: vec![crate::e2e::fixture::Assertion {
            assertion_type: "equals".to_string(),
            field: Some("nonexistent_field".to_string()),
            value: Some(serde_json::json!("x")),
            skip: Some(crate::e2e::fixture::AssertionSkip::All(true)),
            ..Default::default()
        }],
        ..Default::default()
    };
    let config = crate::core::config::ResolvedCrateConfig::default();

    let output = render_test_file(
        "typescript",
        "smoke",
        &[&fixture],
        "",
        "@test/pkg",
        "process",
        &[],
        None,
        None,
        &e2e_config,
        &[],
        &[],
        &[],
        "",
        &config,
        &[],
    );

    assert!(
        output.contains("expect(result).toBeDefined();"),
        "the failable fallback must survive, got:\n{output}"
    );
    assert!(
        output.contains("// skipped: field 'nonexistent_field' not available on result type"),
        "the marker must survive beside the fallback, got:\n{output}"
    );
    assert!(
        crate::e2e::codegen::inert_example::take_inert_examples().is_empty(),
        "an example that still asserts something is not a refusal"
    );
}
