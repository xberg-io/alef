//! Kotlin Android-style e2e codegen tests, split out of `tests.rs`.

use super::super::args::{KotlinArgsContext, build_args_and_setup};
use super::super::test_file::render_test_file_inner;
use crate::core::config::ResolvedCrateConfig;
use crate::e2e::config::ArgMapping;
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::Fixture;
use std::collections::{BTreeMap, HashMap};

/// Regression: kotlin_android test files that contain streaming fixtures must
/// emit `import kotlinx.coroutines.flow.toList`.  Non-android style files must
/// NOT emit it, because `Flow<T>.toList()` is not in scope on JVM targets.
#[test]
fn kotlin_android_streaming_fixture_emits_flow_to_list_import() {
    use crate::core::config::e2e::CallConfig;
    use crate::core::config::extras::{AdapterConfig, AdapterParam, AdapterPattern};
    use crate::e2e::fixture::MockResponse;

    // A fixture with a streaming mock response triggers is_streaming_mock().
    let streaming_fixture = Fixture {
        docs: None,
        requirements: Vec::new(),
        id: "smoke_stream".to_string(),
        category: None,
        description: "streaming test".to_string(),
        tags: vec![],
        skip: None,
        env: None,
        setup: Vec::new(),
        call: None,
        input: serde_json::json!({}),
        mock_response: Some(MockResponse {
            status: 200,
            body: None,
            stream_chunks: Some(vec![serde_json::json!({"delta": "hi"})]),
            headers: BTreeMap::new(),
        }),
        visitor: None,
        args: vec![],
        assertion_recipes: vec![],
        assertions: vec![],
        source: String::new(),
        http: None,
        asyncapi: None,
        websocket: None,
        preserve_input_urls: false,
    };

    let e2e_config = E2eConfig {
        call: CallConfig {
            function: "stream_items".to_string(),
            ..CallConfig::default()
        },
        ..E2eConfig::default()
    };
    // kotlin_android_style=true must emit the import.
    let config = crate::core::config::ResolvedCrateConfig {
        adapters: vec![AdapterConfig {
            name: "stream_items".to_string(),
            pattern: AdapterPattern::Streaming,
            core_path: "sample::Engine::stream_items".to_string(),
            params: vec![AdapterParam {
                name: "request".to_string(),
                ty: "sample::StreamRequest".to_string(),
                optional: false,
            }],
            returns: None,
            error_type: None,
            owner_type: Some("Engine".to_string()),
            item_type: Some("StreamItem".to_string()),
            gil_release: false,
            trait_name: None,
            trait_method: None,
            detect_async: false,
            request_type: None,
            skip_languages: Vec::new(),
        }],
        ..crate::core::config::ResolvedCrateConfig::default()
    };
    let type_defs: Vec<crate::core::ir::TypeDef> = Vec::new();
    let out_android = render_test_file_inner(
        "streaming",
        &[&streaming_fixture],
        "LlmClient",
        "chatStream",
        "dev.sample_crate.sampleapp.android",
        "result",
        &[],
        None,
        false,
        &e2e_config,
        &HashMap::new(),
        true,
        &config,
        &type_defs,
        &[],
        &[],
    )
    .expect("android streaming test file renders");
    assert!(
        out_android.contains("import kotlinx.coroutines.flow.toList"),
        "kotlin_android streaming file must import flow.toList, got:\n{out_android}"
    );
    assert!(
        out_android.contains("import dev.sample_crate.sampleapp.android.StreamRequest"),
        "streaming request DTO must be imported, got:\n{out_android}"
    );

    // kotlin_android_style=false must NOT emit the import.
    let out_jvm = render_test_file_inner(
        "streaming",
        &[&streaming_fixture],
        "LlmClient",
        "chatStream",
        "dev.sample_crate.sampleapp.android",
        "result",
        &[],
        None,
        false,
        &e2e_config,
        &HashMap::new(),
        false,
        &config,
        &type_defs,
        &[],
        &[],
    )
    .expect("jvm streaming test file renders");
    assert!(
        !out_jvm.contains("import kotlinx.coroutines.flow.toList"),
        "non-android streaming file must NOT import flow.toList, got:\n{out_jvm}"
    );
}

/// Regression: kotlin_android test files that instantiate an ObjectMapper must
/// emit `import com.fasterxml.jackson.module.kotlin.registerKotlinModule` and
/// call `.registerKotlinModule()` on the mapper.  Non-android files use plain
/// Java records/builders and must NOT emit either.
#[test]
fn kotlin_android_object_mapper_emits_register_kotlin_module() {
    use crate::core::config::e2e::CallConfig;
    use crate::e2e::fixture::{HttpExpectedResponse, HttpFixture, HttpHandler, HttpRequest};

    // An HTTP fixture forces `needs_object_mapper = true` regardless of args.
    let http_fixture = Fixture {
        docs: None,
        requirements: Vec::new(),
        id: "http_test".to_string(),
        category: None,
        description: "http test".to_string(),
        tags: vec![],
        skip: None,
        env: None,
        setup: Vec::new(),
        call: None,
        input: serde_json::json!({}),
        mock_response: None,
        visitor: None,
        args: vec![],
        assertion_recipes: vec![],
        assertions: vec![],
        source: String::new(),
        http: Some(HttpFixture {
            handler: HttpHandler {
                route: "/v1/test".to_string(),
                method: "POST".to_string(),
                body_schema: None,
                parameters: BTreeMap::new(),
                middleware: None,
            },
            request: HttpRequest {
                method: "POST".to_string(),
                path: "/v1/test".to_string(),
                headers: BTreeMap::new(),
                query_params: BTreeMap::new(),
                cookies: BTreeMap::new(),
                body: None,
                form_data: None,
                content_type: None,
            },
            expected_response: HttpExpectedResponse {
                status_code: 200,
                body: None,
                body_partial: None,
                headers: BTreeMap::new(),
                validation_errors: None,
            },
        }),
        asyncapi: None,
        websocket: None,
        preserve_input_urls: false,
    };

    let e2e_config = E2eConfig {
        call: CallConfig::default(),
        ..E2eConfig::default()
    };
    // kotlin_android_style=true must emit registerKotlinModule import and call.
    let config = crate::core::config::ResolvedCrateConfig::default();
    let type_defs: Vec<crate::core::ir::TypeDef> = Vec::new();
    let out_android = render_test_file_inner(
        "configuration",
        &[&http_fixture],
        "",
        "",
        "dev.sample_crate.sampleapp.android",
        "result",
        &[],
        None,
        false,
        &e2e_config,
        &HashMap::new(),
        true,
        &config,
        &type_defs,
        &[],
        &[],
    )
    .expect("android configuration test file renders");
    assert!(
        out_android.contains("import com.fasterxml.jackson.module.kotlin.registerKotlinModule"),
        "kotlin_android with ObjectMapper must import registerKotlinModule, got:\n{out_android}"
    );
    assert!(
        out_android.contains(".registerKotlinModule()"),
        "kotlin_android MAPPER must call .registerKotlinModule(), got:\n{out_android}"
    );

    // kotlin_android_style=false must NOT emit registerKotlinModule.
    let out_jvm = render_test_file_inner(
        "configuration",
        &[&http_fixture],
        "",
        "",
        "dev.sample_crate.sampleapp.android",
        "result",
        &[],
        None,
        false,
        &e2e_config,
        &HashMap::new(),
        false,
        &config,
        &type_defs,
        &[],
        &[],
    )
    .expect("jvm configuration test file renders");
    assert!(
        !out_jvm.contains("registerKotlinModule"),
        "non-android MAPPER must NOT reference registerKotlinModule, got:\n{out_jvm}"
    );
}

/// Regression: kotlin_android bytes args must be coerced to ByteArray by reading
/// the file path, not passed as plain String literals.
#[test]
fn kotlin_android_bytes_arg_emits_files_read_all_bytes() {
    let args = vec![ArgMapping {
        name: "content".to_string(),
        field: "input.path".to_string(),
        arg_type: "bytes".to_string(),
        optional: false,
        owned: false,
        element_type: None,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    }];
    let fixture = Fixture {
        docs: None,
        requirements: Vec::new(),
        id: "extract_bytes_fixture".to_string(),
        category: None,
        description: "test bytes extraction".to_string(),
        tags: vec![],
        skip: None,
        env: None,
        setup: Vec::new(),
        call: None,
        input: serde_json::json!({ "path": "pdf/test.pdf" }),
        mock_response: None,
        visitor: None,
        args: vec![],
        assertion_recipes: vec![],
        assertions: vec![],
        source: String::new(),
        http: None,
        asyncapi: None,
        websocket: None,
        preserve_input_urls: false,
    };

    // JVM style: should emit plain string
    let (_, args_jvm) = build_args_and_setup(
        &fixture.input,
        &args,
        KotlinArgsContext {
            fixture: &fixture,
            class_name: "SampleBinding",
            options_type: None,
            fixture_id: "extract_bytes_fixture",
            kotlin_android_style: false,
            config: &ResolvedCrateConfig::default(),
            type_defs: &[],
            owner_handle_is_receiver: false,
            enums: &[],
            target_params: crate::e2e::codegen::call_ir::TargetParams::IrAbsent,
        },
    )
    .expect("args build succeeds");
    assert!(
        args_jvm.contains("\"pdf/test.pdf\""),
        "JVM style must emit string literal, got: {args_jvm}"
    );

    // Android style: should emit Files.readAllBytes(Paths.get(...))
    let (_, args_android) = build_args_and_setup(
        &fixture.input,
        &args,
        KotlinArgsContext {
            fixture: &fixture,
            class_name: "SampleBinding",
            options_type: None,
            fixture_id: "extract_bytes_fixture",
            kotlin_android_style: true,
            config: &ResolvedCrateConfig::default(),
            type_defs: &[],
            owner_handle_is_receiver: false,
            enums: &[],
            target_params: crate::e2e::codegen::call_ir::TargetParams::IrAbsent,
        },
    )
    .expect("args build succeeds");
    assert!(
        args_android.contains("java.nio.file.Files.readAllBytes"),
        "kotlin_android bytes arg must use Files.readAllBytes, got: {args_android}"
    );
    assert!(
        args_android.contains("Paths.get("),
        "kotlin_android bytes arg must use Paths.get, got: {args_android}"
    );
}

/// Regression: kotlin_android batch bytes args must wrap each path string in
/// the configured item type with file contents as ByteArray.
#[test]
fn kotlin_android_batch_bytes_item_wraps_paths() {
    let args = vec![ArgMapping {
        name: "items".to_string(),
        field: "input.paths".to_string(),
        arg_type: "json_object".to_string(),
        optional: false,
        owned: false,
        element_type: Some("FileBytesItem".to_string()),
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    }];
    let fixture = Fixture {
        docs: None,
        requirements: Vec::new(),
        id: "batch_extract_fixture".to_string(),
        category: None,
        description: "test batch extraction".to_string(),
        tags: vec![],
        skip: None,
        env: None,
        setup: Vec::new(),
        call: None,
        input: serde_json::json!({ "paths": ["pdf/test1.pdf", "pdf/test2.pdf"] }),
        mock_response: None,
        visitor: None,
        args: vec![],
        assertion_recipes: vec![],
        assertions: vec![],
        source: String::new(),
        http: None,
        asyncapi: None,
        websocket: None,
        preserve_input_urls: false,
    };

    let (_, args_android) = build_args_and_setup(
        &fixture.input,
        &args,
        KotlinArgsContext {
            fixture: &fixture,
            class_name: "SampleBinding",
            options_type: None,
            fixture_id: "batch_extract_fixture",
            kotlin_android_style: true,
            config: &ResolvedCrateConfig::default(),
            type_defs: &[],
            owner_handle_is_receiver: false,
            enums: &[],
            target_params: crate::e2e::codegen::call_ir::TargetParams::IrAbsent,
        },
    )
    .expect("args build succeeds");
    assert!(
        args_android.contains("FileBytesItem"),
        "kotlin_android batch must wrap items in the configured item type, got: {args_android}"
    );
    assert!(
        args_android.contains("java.nio.file.Files.readAllBytes"),
        "kotlin_android batch items must read file bytes, got: {args_android}"
    );
    assert!(
        args_android.contains("listOf("),
        "kotlin_android batch must emit listOf(...), got: {args_android}"
    );
}

/// Regression: emitted `System.loadLibrary(...)` must use the resolved
/// `jni_lib_name()` (`{ffi_prefix}_jni`) so it stays in sync with the cdylib
/// emitted by the generated JNI `Cargo.toml` `[lib] name`. Hard-coding
/// `{crate_name}_jni` breaks for crates that override `[crates.ffi] prefix`
/// (for example: name `custom-runtime-crate`, prefix `custom_runtime`, cdylib
/// `custom_runtime_jni`).
#[test]
fn kotlin_android_test_file_loads_resolved_jni_lib_name_not_crate_name() {
    use crate::core::config::e2e::CallConfig;
    use crate::core::config::{FfiConfig, ResolvedCrateConfig};
    use crate::e2e::fixture::MockResponse;

    let fixture = Fixture {
        docs: None,
        requirements: Vec::new(),
        id: "smoke_one".to_string(),
        category: None,
        description: "smoke".to_string(),
        tags: vec![],
        skip: None,
        env: None,
        setup: Vec::new(),
        call: None,
        input: serde_json::json!({}),
        mock_response: Some(MockResponse {
            status: 200,
            body: None,
            stream_chunks: None,
            headers: BTreeMap::new(),
        }),
        visitor: None,
        args: vec![],
        assertion_recipes: vec![],
        assertions: vec![],
        source: String::new(),
        http: None,
        asyncapi: None,
        websocket: None,
        preserve_input_urls: false,
    };
    let e2e_config = E2eConfig {
        call: CallConfig::default(),
        ..E2eConfig::default()
    };
    let mut config = ResolvedCrateConfig {
        name: "custom-runtime-crate".to_string(),
        ..ResolvedCrateConfig::default()
    };
    config.ffi = Some(FfiConfig {
        prefix: Some("custom_runtime".to_string()),
        error_style: "last_error".to_string(),
        header_name: None,
        lib_name: None,
        visitor_callbacks: false,
        features: None,
        extra_features: Vec::new(),
        serde_rename_all: None,
        exclude_functions: Vec::new(),
        exclude_types: Vec::new(),
        capsule_types: HashMap::new(),
        rename_fields: HashMap::new(),
        plugin_error_constructor: None,
        target_dep_overrides: Vec::new(),
        excluded_default_features: Vec::new(),
    });
    let type_defs: Vec<crate::core::ir::TypeDef> = Vec::new();
    let out = render_test_file_inner(
        "smoke",
        &[&fixture],
        "Bridge",
        "doThing",
        "dev.sample_crate.sampleapp.android",
        "result",
        &[],
        None,
        false,
        &e2e_config,
        &HashMap::new(),
        true,
        &config,
        &type_defs,
        &[],
        &[],
    )
    .expect("kotlin_android test file renders");
    assert!(
        out.contains("System.loadLibrary(\"custom_runtime_jni\")"),
        "kotlin_android test must loadLibrary the resolved jni_lib_name (`custom_runtime_jni`), got:\n{out}"
    );
    assert!(
        !out.contains("custom-runtime-crate_jni"),
        "kotlin_android test must NOT loadLibrary the raw crate name, got:\n{out}"
    );
}

#[test]
fn empty_mock_url_lists_have_explicit_string_element_types() {
    for preserve in [false, true] {
        let fixture: Fixture = serde_json::from_value(serde_json::json!({
            "id": "empty_batch", "input": { "urls": [] }, "preserve_input_urls": preserve
        }))
        .expect("valid fixture");
        let args: Vec<ArgMapping> = serde_json::from_value(serde_json::json!([{
            "name": "urls", "field": "input.urls", "type": "mock_url_list"
        }]))
        .expect("valid URL argument");
        let (setup, call) = build_args_and_setup(
            &fixture.input,
            &args,
            KotlinArgsContext {
                fixture: &fixture,
                class_name: "SampleBinding",
                options_type: None,
                fixture_id: "empty_batch",
                kotlin_android_style: true,
                config: &ResolvedCrateConfig::default(),
                type_defs: &[],
                owner_handle_is_receiver: false,
                enums: &[],
                target_params: crate::e2e::codegen::call_ir::TargetParams::IrAbsent,
            },
        )
        .expect("URL argument generation succeeds");
        assert_eq!(call, "urls");
        assert!(
            setup.iter().any(|line| line.contains("listOf<String>()")),
            "preserve={preserve}: {setup:?}"
        );
    }
}
