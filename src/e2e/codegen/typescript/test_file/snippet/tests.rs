use super::*;
use crate::e2e::config::CallConfig;

fn fixture() -> Fixture {
    Fixture {
        docs: None,
        requirements: Vec::new(),
        id: "quick_start".to_string(),
        category: None,
        description: "Quick start".to_string(),
        tags: Vec::new(),
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
        assertions: Vec::new(),
        visitor: None,
        args: Vec::new(),
        assertion_recipes: Vec::new(),
    }
}

#[test]
fn same_named_field_in_unrelated_struct_is_not_inferred_as_enum() {
    // Regression for #578: `infer_enum_fields` used to key its result on the
    // bare field name, so a genuinely enum-typed field on one struct
    // (`TranscriptionConfig.model: WhisperModel`) poisoned an unrelated
    // same-named `String` field on a different struct (`LlmConfig.model`)
    // reachable from the same call's type graph.
    let type_defs = [
        TypeDef {
            name: "TranscriptionConfig".into(),
            fields: vec![crate::core::ir::FieldDef {
                name: "model".into(),
                ty: crate::core::ir::TypeRef::Named("WhisperModel".into()),
                ..Default::default()
            }],
            ..Default::default()
        },
        TypeDef {
            name: "LlmConfig".into(),
            fields: vec![crate::core::ir::FieldDef {
                name: "model".into(),
                ty: crate::core::ir::TypeRef::String,
                ..Default::default()
            }],
            ..Default::default()
        },
    ];
    let enums = [EnumDef {
        name: "WhisperModel".into(),
        variants: vec![crate::core::ir::EnumVariant {
            name: "Base".into(),
            serde_rename: Some("base".into()),
            ..Default::default()
        }],
        ..Default::default()
    }];

    // Mirrors `render_snippet_body`, which accumulates `infer_enum_fields`
    // results from multiple type roots (the call's options type and each
    // argument's element type) into one shared map.
    let mut enum_fields = std::collections::HashMap::new();
    infer_enum_fields(Some("TranscriptionConfig"), &type_defs, &enums, &mut enum_fields);
    infer_enum_fields(Some("LlmConfig"), &type_defs, &enums, &mut enum_fields);

    let llm_object = serde_json::json!({"model": "openai/gpt-4o-mini"});
    let llm_expression = ts_builder_expression(
        llm_object.as_object().expect("object"),
        "LlmConfig",
        &Default::default(),
        "node",
        &enum_fields,
        &Default::default(),
        &type_defs,
        &enums,
        "",
        &[],
        &mut Default::default(),
    );
    assert_eq!(llm_expression, "{ model: \"openai/gpt-4o-mini\" } as LlmConfig");

    let transcription_object = serde_json::json!({"model": "base"});
    let transcription_expression = ts_builder_expression(
        transcription_object.as_object().expect("object"),
        "TranscriptionConfig",
        &Default::default(),
        "node",
        &enum_fields,
        &Default::default(),
        &type_defs,
        &enums,
        "",
        &[],
        &mut Default::default(),
    );
    assert_eq!(
        transcription_expression,
        "{ model: WhisperModel.Base } as TranscriptionConfig"
    );
}

#[test]
fn async_snippet_reuses_the_test_call_shape_without_test_harness() {
    let mut e2e = E2eConfig {
        call: CallConfig {
            function: "load_document".to_string(),
            module: "@example/library".to_string(),
            result_var: "document".to_string(),
            r#async: true,
            ..CallConfig::default()
        },
        ..E2eConfig::default()
    };
    e2e.call
        .overrides
        .entry("node".into())
        .or_default()
        .enum_fields
        .insert("mode".into(), "UnusedMode".into());
    let fixture = fixture();
    let config = crate::core::config::ResolvedCrateConfig::default();
    let body = render_snippet_body(SnippetContext {
        lang: "node",
        fixture: &fixture,
        module: "@example/library",
        client_factory: None,
        e2e_config: &e2e,
        type_defs: &[],
        enums: &[],
        functions: &[],
        wasm_type_prefix: "",
        config: &config,
    });

    assert!(body.contains("import { loadDocument } from \"@example/library\";"));
    assert!(body.contains("const document = await loadDocument();"));
    assert!(!body.contains("vitest"));
    assert!(!body.contains("expect("));
    assert!(!body.contains("UnusedMode"));
}

#[test]
fn docs_argument_override_imports_its_referenced_input_type() {
    let mut fixture = fixture();
    fixture.docs = Some(crate::e2e::fixture::FixtureDocs {
        sample_url: None,
        topic: "guides".into(),
        stem: None,
        paths: Default::default(),
        title: None,
        description: None,
        input: None,
        shows: Vec::new(),
        error: None,
        presentation: Some(crate::e2e::fixture::FixtureDocsPresentation {
            call: None,
            input: Some(serde_json::json!({"source": {"kind": "uri", "uri": "guide.txt"}})),
            args: Some(vec![crate::e2e::config::ArgMapping {
                name: "source".into(),
                field: "source".into(),
                arg_type: "json_object".into(),
                optional: false,
                owned: true,
                element_type: Some("DocumentInput".into()),
                go_type: None,
                vec_inner_is_ref: false,
                trait_name: None,
            }]),
            files: Vec::new(),
            operations: Vec::new(),
        }),
        client: None,
        side_effects: crate::e2e::fixture::SideEffectClass::Safe,
        coverage_exceptions: Default::default(),
        sample_url_vars: Default::default(),
        body_file: None,
    });
    let e2e = E2eConfig {
        call: CallConfig {
            function: "load_document".into(),
            module: "@example/library".into(),
            result_var: "document".into(),
            r#async: true,
            ..CallConfig::default()
        },
        ..E2eConfig::default()
    };
    let config = crate::core::config::ResolvedCrateConfig::default();

    let body = render_snippet_body(SnippetContext {
        lang: "node",
        fixture: &fixture,
        module: "@example/library",
        client_factory: None,
        e2e_config: &e2e,
        type_defs: &[],
        enums: &[],
        functions: &[],
        wasm_type_prefix: "",
        config: &config,
    });

    assert!(body.contains("import { DocumentInput, loadDocument }"), "{body}");
    assert!(body.contains("const source: DocumentInput ="), "{body}");
    assert!(!body.contains("as DocumentInput"), "{body}");
}

#[test]
fn docs_bytes_read_the_presented_relative_path() {
    let mut fixture = fixture();
    fixture.input = serde_json::json!({"content": "document.pdf"});
    fixture.args = vec![crate::e2e::config::ArgMapping {
        name: "content".into(),
        field: "content".into(),
        arg_type: "bytes".into(),
        optional: false,
        owned: true,
        element_type: None,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    }];
    let e2e = E2eConfig {
        call: CallConfig {
            function: "load_document".into(),
            module: "@example/library".into(),
            result_var: "document".into(),
            r#async: true,
            ..CallConfig::default()
        },
        ..E2eConfig::default()
    };
    let config = crate::core::config::ResolvedCrateConfig::default();

    let body = render_snippet_body(SnippetContext {
        lang: "node",
        fixture: &fixture,
        module: "@example/library",
        client_factory: None,
        e2e_config: &e2e,
        type_defs: &[],
        enums: &[],
        functions: &[],
        wasm_type_prefix: "",
        config: &config,
    });

    assert!(body.contains("readFile(\"document.pdf\")"), "{body}");
    assert!(body.contains("await loadDocument(_content_content)"), "{body}");
}

#[test]
fn expected_error_snippet_handles_the_rejected_call() {
    let mut fixture = fixture();
    fixture.assertions.push(crate::e2e::fixture::Assertion {
        assertion_type: "error".into(),
        ..Default::default()
    });
    let mut e2e = E2eConfig::default();
    e2e.call.function = "parse".into();
    e2e.call.r#async = true;
    let config = crate::core::config::ResolvedCrateConfig::default();
    let body = render_snippet_body(SnippetContext {
        lang: "node",
        fixture: &fixture,
        module: "@example/library",
        client_factory: None,
        e2e_config: &e2e,
        type_defs: &[],
        enums: &[],
        functions: &[],
        wasm_type_prefix: "",
        config: &config,
    });
    assert!(body.contains("try {"));
    assert!(body.contains("error instanceof Error"));
    assert!(!body.contains("expected call to fail"));
    assert!(!body.contains("const result = await"));
}

/// napi-rs (the node target's FFI boundary) converts every Rust error into a
/// plain JS `Error` -- it never generates a named error class. A crate's
/// `error_type` config (e.g. `error_type = "XbergError"` in alef.toml, used
/// by every other language's docs snippets to build an idiomatic
/// `import`/`instanceof` pair) does not apply to node: importing and
/// `instanceof`-checking a class that the node package never exports fails
/// with `TS2305: Module has no exported member 'XbergError'`.
///
/// Before this fix, node snippets used `config.error_type_name()`
/// unconditionally, so any crate with a custom `error_type` broke every
/// generated node docs snippet that expects an error (see
/// error_empty_mime.md, error_unsupported_mime.md, and others).
#[test]
fn node_error_snippet_uses_builtin_error_not_the_crate_error_type() {
    let mut fixture = fixture();
    fixture.assertions.push(crate::e2e::fixture::Assertion {
        assertion_type: "error".into(),
        ..Default::default()
    });
    let mut e2e = E2eConfig::default();
    e2e.call.function = "parse".into();
    e2e.call.r#async = true;
    let config = crate::core::config::ResolvedCrateConfig {
        error_type: Some("XbergError".into()),
        ..Default::default()
    };

    let node_body = render_snippet_body(SnippetContext {
        lang: "node",
        fixture: &fixture,
        module: "@example/library",
        client_factory: None,
        e2e_config: &e2e,
        type_defs: &[],
        enums: &[],
        functions: &[],
        wasm_type_prefix: "",
        config: &config,
    });
    assert!(
        node_body.contains("error instanceof Error"),
        "node must fall back to the built-in Error, got: {node_body}"
    );
    assert!(
        !node_body.contains("XbergError"),
        "node must never reference the crate error type, got: {node_body}"
    );
    assert!(
        !node_body.contains("import { Error"),
        "Error is a global -- it must not be imported, got: {node_body}"
    );
}

/// `crates/xberg-wasm` throws a bare JS string from every fallible export
/// (`.map_err(|e| JsValue::from_str(&e.to_string()))`) -- there is no
/// `#[wasm_bindgen]` `XbergError`/`WasmXbergError` export to `instanceof`
/// against, and `error.name`/`error.message` are always `undefined` on a
/// thrown string. A crate's `error_type` config must not leak into the
/// wasm catch block: the only value that reads correctly off a thrown
/// wasm error is `String(error)`.
///
/// Before this fix, wasm snippets emitted `if (error instanceof
/// XbergError) { console.error(...) }` and imported `XbergError` from a
/// module that never exports it, so every generated wasm docs snippet
/// that expects an error failed at both import resolution and runtime
/// (the check is always false, so the catch block is silently a no-op).
#[test]
fn wasm_error_snippet_reads_the_thrown_value_as_a_string() {
    let mut fixture = fixture();
    fixture.assertions.push(crate::e2e::fixture::Assertion {
        assertion_type: "error".into(),
        ..Default::default()
    });
    let mut e2e = E2eConfig::default();
    e2e.call.function = "parse".into();
    e2e.call.r#async = true;
    let config = crate::core::config::ResolvedCrateConfig {
        error_type: Some("XbergError".into()),
        ..Default::default()
    };

    let wasm_body = render_snippet_body(SnippetContext {
        lang: "wasm",
        fixture: &fixture,
        module: "@example/library",
        client_factory: None,
        e2e_config: &e2e,
        type_defs: &[],
        enums: &[],
        functions: &[],
        wasm_type_prefix: "",
        config: &config,
    });
    assert!(
        wasm_body.contains("console.error(String(error));"),
        "wasm must read the thrown value as a string, got: {wasm_body}"
    );
    assert!(
        !wasm_body.contains("instanceof"),
        "wasm has no named error class to instanceof-check, got: {wasm_body}"
    );
    assert!(
        !wasm_body.contains("XbergError"),
        "wasm must never reference the crate error type -- it isn't exported, got: {wasm_body}"
    );
    assert!(
        !wasm_body.contains("import { XbergError"),
        "wasm has nothing to import for errors, got: {wasm_body}"
    );
}

#[test]
fn wasm_visitor_snippet_builds_and_attaches_the_real_bridge() {
    let mut fixture = fixture();
    fixture.visitor = Some(crate::e2e::fixture::VisitorSpec {
        callbacks: [("visit_text".into(), crate::e2e::fixture::CallbackAction::Continue)].into(),
    });
    let mut e2e = E2eConfig::default();
    e2e.call.function = "render_document".into();
    e2e.call.overrides.entry("wasm".into()).or_default().options_type = Some("RenderOptions".into());
    let config = crate::core::config::ResolvedCrateConfig {
        trait_bridges: vec![crate::core::config::TraitBridgeConfig {
            trait_name: "DocumentVisitor".into(),
            type_alias: Some("VisitorHandle".into()),
            options_type: Some("RenderOptions".into()),
            options_field: Some("visitor".into()),
            ..Default::default()
        }],
        ..Default::default()
    };

    let body = render_snippet_body(SnippetContext {
        lang: "wasm",
        fixture: &fixture,
        module: "@example/wasm",
        client_factory: None,
        e2e_config: &e2e,
        type_defs: &[],
        enums: &[],
        functions: &[],
        wasm_type_prefix: "",
        config: &config,
    });

    assert!(body.contains("visitText(ctx: any, text: any)"));
    assert!(body.contains("new WasmVisitorHandle(_testVisitor)"));
    assert!(body.contains("RenderOptions.default()"));
    assert!(body.contains("VisitorHandle"));
}

/// A doc-only WASM snippet whose options type has a class-typed field that is reached
/// only through the IR (no `nested_types` entry configured on the call override, which
/// this fixture deliberately leaves unset). The builder expression still constructs the
/// nested class (`ts_builder_expression_inner` derives it from `type_defs` on its own),
/// so the import statement must name it too, or the emitted snippet references an
/// undeclared symbol and fails to typecheck. `render_test_file`'s import builder already
/// walks the IR transitively for this reason; this pins the same behaviour for the
/// standalone-snippet path.
#[test]
fn wasm_snippet_imports_a_nested_class_reached_only_through_the_ir() {
    let mut fixture = fixture();
    fixture.input = serde_json::json!({"config": {}});
    fixture.args = vec![crate::e2e::config::ArgMapping {
        name: "input".into(),
        field: "input".into(),
        arg_type: "json_object".into(),
        optional: false,
        owned: true,
        element_type: None,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    }];
    let mut e2e = E2eConfig::default();
    e2e.call.function = "extract".into();
    e2e.call.overrides.entry("wasm".into()).or_default().options_type = Some("ExtractInput".into());
    let type_defs = [
        TypeDef {
            name: "ExtractInput".into(),
            fields: vec![crate::core::ir::FieldDef {
                name: "config".into(),
                ty: crate::core::ir::TypeRef::Optional(Box::new(crate::core::ir::TypeRef::Named(
                    "FileExtractionConfig".into(),
                ))),
                ..Default::default()
            }],
            ..Default::default()
        },
        TypeDef {
            name: "FileExtractionConfig".into(),
            ..Default::default()
        },
    ];
    let config = crate::core::config::ResolvedCrateConfig::default();

    let body = render_snippet_body(SnippetContext {
        lang: "wasm",
        fixture: &fixture,
        module: "@example/wasm",
        client_factory: None,
        e2e_config: &e2e,
        type_defs: &type_defs,
        enums: &[],
        functions: &[],
        wasm_type_prefix: "Wasm",
        config: &config,
    });

    assert!(
        body.contains("WasmFileExtractionConfig.default()"),
        "test setup did not reach the nested-builder path, got: {body}"
    );
    assert!(
        body.contains("import { WasmExtractInput, WasmFileExtractionConfig, extract }"),
        "nested class reached only through the IR must still be imported, got: {body}"
    );
}

/// Pins that a `client_factory` documentation snippet never points the reader at the
/// mock server (`MOCK_SERVER*` env vars, the `/fixtures/<id>` route, or the literal
/// `"test-key"` credential) and does construct a client for the reader.
///
/// TypeScript diverges from java/csharp/zig on the credential: those read it from
/// the environment, while `snippet.rs`'s `client_setup` inlines the
/// reader-substitutable placeholder `"your-api-key"`. That is a convention
/// difference, not a harness leak, so this pins the placeholder rather than
/// asserting an environment read the generator does not perform.
#[test]
fn client_factory_snippet_never_points_the_reader_at_the_mock_server() {
    let mut fixture = fixture();
    fixture.id = "rate_limit_429".into();
    fixture.description = "Rate limited".into();
    fixture.mock_response = Some(crate::e2e::fixture::MockResponse {
        status: 429,
        body: None,
        stream_chunks: None,
        headers: Default::default(),
    });
    let mut e2e = E2eConfig::default();
    e2e.call.function = "chat".into();
    e2e.call.result_var = "result".into();
    e2e.call.overrides.insert(
        "node".into(),
        crate::e2e::config::CallOverride {
            client_factory: Some("create_client".into()),
            ..Default::default()
        },
    );
    let config = crate::core::config::ResolvedCrateConfig::default();

    let body = render_snippet_body(SnippetContext {
        lang: "node",
        fixture: &fixture,
        module: "@example/library",
        client_factory: None,
        e2e_config: &e2e,
        type_defs: &[],
        enums: &[],
        functions: &[],
        wasm_type_prefix: "",
        config: &config,
    });

    assert!(!body.contains("MOCK_SERVER"), "mock-server env var leaked:\n{body}");
    assert!(
        !body.contains("/fixtures/rate_limit_429"),
        "mock-server fixture route leaked:\n{body}"
    );
    assert!(!body.contains("\"test-key\""), "literal credential leaked:\n{body}");
    assert!(
        body.contains("const client = create_client(\"your-api-key\");"),
        "client is not constructed with a reader-substitutable credential:\n{body}"
    );
    assert!(
        body.contains("client.chat("),
        "the call must go through the constructed client:\n{body}"
    );
}
