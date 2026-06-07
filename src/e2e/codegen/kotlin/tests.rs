use super::args::{KotlinArgsContext, build_args_and_setup};
use super::assertions::render_assertion;
use super::project::render_build_gradle;
use super::test_file::{is_enum_typed, render_test_file_inner};
use crate::core::config::ResolvedCrateConfig;
use crate::e2e::config::ArgMapping;
use crate::e2e::config::E2eConfig;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::{Assertion, Fixture};
use std::collections::{BTreeMap, HashMap, HashSet};

fn make_resolver_for_finish_reason() -> FieldResolver {
    // Resolver for `choices[0].finish_reason` where:
    //   - `choices` is a registered array field (default index 0)
    //   - `choices.finish_reason` is optional (`@Nullable`)
    let mut optional = HashSet::new();
    optional.insert("choices.finish_reason".to_string());
    let mut arrays = HashSet::new();
    arrays.insert("choices".to_string());
    FieldResolver::new(&HashMap::new(), &optional, &HashSet::new(), &arrays, &HashSet::new())
}

/// Regression: enum-typed optional fields must route through `?.getValue()`
/// before falling back via `.orEmpty()`. Emitting `.orEmpty().getValue()`
/// is invalid Kotlin because `T?.orEmpty()` is only defined for `String?`.
#[test]
fn assertion_enum_optional_uses_safe_get_value_then_or_empty() {
    let resolver = make_resolver_for_finish_reason();
    let mut enum_fields = HashSet::new();
    enum_fields.insert("choices.finish_reason".to_string());
    let assertion = Assertion {
        assertion_type: "equals".to_string(),
        field: Some("choices.finish_reason".to_string()),
        value: Some(serde_json::Value::String("stop".to_string())),
        values: None,
        method: None,
        check: None,
        args: None,
        return_type: None,
    };
    let mut out = String::new();
    render_assertion(
        &mut out,
        &assertion,
        "result",
        "",
        &resolver,
        false,
        false,
        &enum_fields,
        &HashMap::new(),
        false,
        false,
    );
    assert!(
        out.contains("result.choices().first().finishReason()?.getValue().orEmpty().trim()"),
        "expected enum-optional safe-call pattern, got: {out}"
    );
    assert!(
        !out.contains(".finishReason().orEmpty().getValue()"),
        "must not emit .orEmpty().getValue() on a nullable enum: {out}"
    );
}

#[test]
fn handle_config_deserialization_uses_resolved_options_type() {
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
    let fixture = Fixture {
        id: "session_fixture".to_string(),
        category: None,
        description: "test fixture".to_string(),
        tags: vec![],
        skip: None,
        env: None,
        call: None,
        input: serde_json::json!({ "config": { "limit": 3 } }),
        mock_response: None,
        visitor: None,
        args: vec![],
        assertion_recipes: vec![],
        assertions: vec![],
        source: String::new(),
        http: None,
    };

    let (setup, args_str) = build_args_and_setup(
        &fixture.input,
        &args,
        KotlinArgsContext {
            fixture: &fixture,
            class_name: "Sample",
            options_type: Some("SessionConfig"),
            fixture_id: &fixture.id,
            kotlin_android_style: false,
            config: &ResolvedCrateConfig::default(),
            type_defs: &[],
        },
    );

    let rendered = setup.join("\n");
    assert_eq!(args_str, "session");
    assert!(rendered.contains("MAPPER.readValue(\"{\\\"limit\\\":3}\", SessionConfig::class.java)"));
    assert!(rendered.contains("Sample.createSession(sessionConfig)"));
    assert!(!rendered.contains("CrawlConfig"));
}

/// Non-optional enum field should call `.getValue()` directly without
/// safe-call or fallback (no need to handle null).
#[test]
fn assertion_enum_non_optional_uses_plain_get_value() {
    let mut arrays = HashSet::new();
    arrays.insert("choices".to_string());
    let resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &arrays,
        &HashSet::new(),
    );
    let mut enum_fields = HashSet::new();
    enum_fields.insert("choices.finish_reason".to_string());
    let assertion = Assertion {
        assertion_type: "equals".to_string(),
        field: Some("choices.finish_reason".to_string()),
        value: Some(serde_json::Value::String("stop".to_string())),
        values: None,
        method: None,
        check: None,
        args: None,
        return_type: None,
    };
    let mut out = String::new();
    render_assertion(
        &mut out,
        &assertion,
        "result",
        "",
        &resolver,
        false,
        false,
        &enum_fields,
        &HashMap::new(),
        false,
        false,
    );
    assert!(
        out.contains("result.choices().first().finishReason().getValue().trim()"),
        "expected plain .getValue() for non-optional enum, got: {out}"
    );
}

/// Regression: per-call `enum_fields` overrides (e.g. `status = "BatchStatus"`) must be
/// merged into the effective enum-field set before rendering assertions.  Previously the
/// kotlin codegen only consulted the global `fields_enum` set, so `status` on `BatchObject`
/// was treated as a plain `String` and `.trim()` was emitted directly instead of
/// `.getValue().trim()`, causing a Kotlin compile error ("BatchStatus has no method trim").
#[test]
fn per_call_enum_field_override_routes_through_get_value() {
    // Simulate `status` field on a non-optional result with no global enum registration.
    let resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    );
    // `status` is NOT in the global enum_fields set...
    let global_enum_fields: HashSet<String> = HashSet::new();
    // ...but a per-call override registers it.
    let mut per_call_enum_fields: HashSet<String> = global_enum_fields.clone();
    per_call_enum_fields.insert("status".to_string());

    let assertion = Assertion {
        assertion_type: "equals".to_string(),
        field: Some("status".to_string()),
        value: Some(serde_json::Value::String("validating".to_string())),
        values: None,
        method: None,
        check: None,
        args: None,
        return_type: None,
    };

    // Without the merge (global only): must NOT emit .getValue()
    let mut out_no_merge = String::new();
    render_assertion(
        &mut out_no_merge,
        &assertion,
        "result",
        "",
        &resolver,
        false,
        false,
        &global_enum_fields,
        &HashMap::new(),
        false,
        false,
    );
    assert!(
        !out_no_merge.contains(".getValue()"),
        "global-only set must not emit .getValue() for unregistered status: {out_no_merge}"
    );

    // With the merge (per-call included): must emit .getValue()
    let mut out_merged = String::new();
    render_assertion(
        &mut out_merged,
        &assertion,
        "result",
        "",
        &resolver,
        false,
        false,
        &per_call_enum_fields,
        &HashMap::new(),
        false,
        false,
    );
    assert!(
        out_merged.contains(".getValue()"),
        "merged per-call set must emit .getValue() for status: {out_merged}"
    );
}

/// Auto-detection: fields whose Rust type is `Named(T)` where `T` is NOT a
/// known struct should be treated as enum-typed without any explicit per-call
/// `enum_fields` override. The `type_enum_fields` map (built in `generate()`)
/// pre-computes these sets so `render_test_method` can merge them.
#[test]
fn auto_detected_enum_fields_from_type_defs_route_through_get_value() {
    use crate::core::ir::{CoreWrapper, FieldDef, TypeDef, TypeRef};

    // Simulate a `BatchObject` type with `status: BatchStatus` (Named, not a struct).
    let batch_object_def = TypeDef {
        name: "BatchObject".to_string(),
        rust_path: "demo_client::BatchObject".to_string(),
        original_rust_path: String::new(),
        fields: vec![
            FieldDef {
                name: "id".to_string(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                doc: String::new(),
                sanitized: false,
                is_boxed: false,
                type_rust_path: None,
                cfg: None,
                typed_default: None,
                core_wrapper: CoreWrapper::None,
                vec_inner_core_wrapper: CoreWrapper::None,
                newtype_wrapper: None,
                serde_rename: None,
                serde_flatten: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
                original_type: None,
            },
            FieldDef {
                name: "status".to_string(),
                ty: TypeRef::Named("BatchStatus".to_string()),
                optional: false,
                default: None,
                doc: String::new(),
                sanitized: false,
                is_boxed: false,
                type_rust_path: None,
                cfg: None,
                typed_default: None,
                core_wrapper: CoreWrapper::None,
                vec_inner_core_wrapper: CoreWrapper::None,
                newtype_wrapper: None,
                serde_rename: None,
                serde_flatten: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
                original_type: None,
            },
        ],
        methods: vec![],
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        doc: String::new(),
        cfg: None,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: true,
        serde_rename_all: None,
        has_serde: true,
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
    };

    // `BatchObject` is the only struct — `BatchStatus` is not in struct_names.
    let type_defs = [batch_object_def];
    let struct_names: HashSet<&str> = type_defs.iter().map(|td| td.name.as_str()).collect();

    // Verify is_enum_typed correctly identifies `status` as enum-typed.
    let status_ty = TypeRef::Named("BatchStatus".to_string());
    assert!(
        is_enum_typed(&status_ty, &struct_names),
        "BatchStatus (not a known struct) should be detected as enum-typed"
    );
    let id_ty = TypeRef::String;
    assert!(
        !is_enum_typed(&id_ty, &struct_names),
        "String field should NOT be detected as enum-typed"
    );

    // Verify the type_enum_fields map is built correctly.
    let type_enum_fields: std::collections::HashMap<String, HashSet<String>> = type_defs
        .iter()
        .filter_map(|td| {
            let enum_field_names: HashSet<String> = td
                .fields
                .iter()
                .filter(|field| is_enum_typed(&field.ty, &struct_names))
                .map(|field| field.name.clone())
                .collect();
            if enum_field_names.is_empty() {
                None
            } else {
                Some((td.name.clone(), enum_field_names))
            }
        })
        .collect();

    let batch_enum_fields = type_enum_fields
        .get("BatchObject")
        .expect("BatchObject should have enum fields");
    assert!(
        batch_enum_fields.contains("status"),
        "BatchObject.status should be auto-detected as enum-typed, got: {batch_enum_fields:?}"
    );
    assert!(
        !batch_enum_fields.contains("id"),
        "BatchObject.id (String) must not be in enum fields"
    );

    // Verify render_assertion produces `.getValue()` when `status` is in enum_fields.
    let resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    );
    let assertion = Assertion {
        assertion_type: "equals".to_string(),
        field: Some("status".to_string()),
        value: Some(serde_json::Value::String("validating".to_string())),
        values: None,
        method: None,
        check: None,
        args: None,
        return_type: None,
    };
    let mut out = String::new();
    render_assertion(
        &mut out,
        &assertion,
        "result",
        "",
        &resolver,
        false,
        false,
        batch_enum_fields,
        &HashMap::new(),
        false,
        false,
    );
    assert!(
        out.contains(".getValue()"),
        "auto-detected enum field must route through .getValue(), got: {out}"
    );
}

/// Regression: kotlin_android test files that contain streaming fixtures must
/// emit `import kotlinx.coroutines.flow.toList`.  Non-android style files must
/// NOT emit it, because `Flow<T>.toList()` is not in scope on JVM targets.
#[test]
fn kotlin_android_streaming_fixture_emits_flow_to_list_import() {
    use crate::core::config::e2e::CallConfig;
    use crate::e2e::fixture::MockResponse;

    // A fixture with a streaming mock response triggers is_streaming_mock().
    let streaming_fixture = Fixture {
        id: "smoke_stream".to_string(),
        category: None,
        description: "streaming test".to_string(),
        tags: vec![],
        skip: None,
        env: None,
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
    };

    let e2e_config = E2eConfig {
        call: CallConfig::default(),
        ..E2eConfig::default()
    };
    // kotlin_android_style=true must emit the import.
    let config = crate::core::config::ResolvedCrateConfig::default();
    let type_defs: Vec<crate::core::ir::TypeDef> = Vec::new();
    let out_android = render_test_file_inner(
        "streaming",
        &[&streaming_fixture],
        "LlmClient",
        "chatStream",
        "dev.sample_crate.samplellm.android",
        "result",
        &[],
        None,
        false,
        &e2e_config,
        &HashMap::new(),
        true,
        &config,
        &type_defs,
    );
    assert!(
        out_android.contains("import kotlinx.coroutines.flow.toList"),
        "kotlin_android streaming file must import flow.toList, got:\n{out_android}"
    );

    // kotlin_android_style=false must NOT emit the import.
    let out_jvm = render_test_file_inner(
        "streaming",
        &[&streaming_fixture],
        "LlmClient",
        "chatStream",
        "dev.sample_crate.samplellm.android",
        "result",
        &[],
        None,
        false,
        &e2e_config,
        &HashMap::new(),
        false,
        &config,
        &type_defs,
    );
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
        id: "http_test".to_string(),
        category: None,
        description: "http test".to_string(),
        tags: vec![],
        skip: None,
        env: None,
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
        "dev.sample_crate.samplellm.android",
        "result",
        &[],
        None,
        false,
        &e2e_config,
        &HashMap::new(),
        true,
        &config,
        &type_defs,
    );
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
        "dev.sample_crate.samplellm.android",
        "result",
        &[],
        None,
        false,
        &e2e_config,
        &HashMap::new(),
        false,
        &config,
        &type_defs,
    );
    assert!(
        !out_jvm.contains("registerKotlinModule"),
        "non-android MAPPER must NOT reference registerKotlinModule, got:\n{out_jvm}"
    );
}

/// Registry mode joins the group (`kotlin_pkg_id`) and artifactId (`pkg_name`)
/// into a single `group:artifact:version` coordinate.
#[test]
fn registry_dep_uses_group_artifact_version_coordinate() {
    let out = render_build_gradle(
        "sample_router-kotlin",
        "dev.sample_router",
        "0.15.6-rc.3",
        crate::e2e::config::DependencyMode::Registry,
        false,
    );
    assert!(
        out.contains(r#"testImplementation("dev.sample_router:sample_router-kotlin:0.15.6-rc.3")"#),
        "expected single-group maven coordinate, got:\n{out}"
    );
}

/// Regression: a `pkg_name` that already embeds the group must NOT have the
/// group prepended a second time (previously produced the unresolvable
/// `dev.sample_project:dev.sample_project:sample_project:<version>` coordinate).
#[test]
fn registry_dep_does_not_double_the_group_prefix() {
    let out = render_build_gradle(
        "dev.sample_router:sample_router-kotlin",
        "dev.sample_router",
        "0.15.6-rc.3",
        crate::e2e::config::DependencyMode::Registry,
        false,
    );
    assert!(
        out.contains(r#"testImplementation("dev.sample_router:sample_router-kotlin:0.15.6-rc.3")"#),
        "group must not be doubled, got:\n{out}"
    );
    assert!(
        !out.contains("dev.sample_router:dev.sample_router"),
        "doubled group must never appear, got:\n{out}"
    );
}

/// Local mode resolves the built jar by its filesystem base name (the
/// kotlin binding's `rootProject.name`, passed as `pkg_name` in local mode),
/// independent of the published Maven artifactId.
#[test]
fn local_dep_references_built_jar_by_base_name() {
    let out = render_build_gradle(
        "sample_router",
        "dev.sample_router",
        "0.15.6-rc.3",
        crate::e2e::config::DependencyMode::Local,
        false,
    );
    assert!(
        out.contains("packages/kotlin/build/libs/sample_router-0.15.6-rc.3.jar"),
        "expected local jar reference, got:\n{out}"
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
        id: "extract_bytes_fixture".to_string(),
        category: None,
        description: "test bytes extraction".to_string(),
        tags: vec![],
        skip: None,
        env: None,
        call: None,
        input: serde_json::json!({ "path": "pdf/test.pdf" }),
        mock_response: None,
        visitor: None,
        args: vec![],
        assertion_recipes: vec![],
        assertions: vec![],
        source: String::new(),
        http: None,
    };

    // JVM style: should emit plain string
    let (_, args_jvm) = build_args_and_setup(
        &fixture.input,
        &args,
        KotlinArgsContext {
            fixture: &fixture,
            class_name: "Kreuzberg",
            options_type: None,
            fixture_id: "extract_bytes_fixture",
            kotlin_android_style: false,
            config: &ResolvedCrateConfig::default(),
            type_defs: &[],
        },
    );
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
            class_name: "Kreuzberg",
            options_type: None,
            fixture_id: "extract_bytes_fixture",
            kotlin_android_style: true,
            config: &ResolvedCrateConfig::default(),
            type_defs: &[],
        },
    );
    assert!(
        args_android.contains("java.nio.file.Files.readAllBytes"),
        "kotlin_android bytes arg must use Files.readAllBytes, got: {args_android}"
    );
    assert!(
        args_android.contains("Paths.get("),
        "kotlin_android bytes arg must use Paths.get, got: {args_android}"
    );
}

/// Regression: kotlin_android batch_bytes args must wrap each path string in
/// BatchBytesItem(...) with file contents as ByteArray.
#[test]
fn kotlin_android_batch_bytes_item_wraps_paths() {
    let args = vec![ArgMapping {
        name: "items".to_string(),
        field: "input.paths".to_string(),
        arg_type: "json_object".to_string(),
        optional: false,
        owned: false,
        element_type: Some("BatchBytesItem".to_string()),
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    }];
    let fixture = Fixture {
        id: "batch_extract_fixture".to_string(),
        category: None,
        description: "test batch extraction".to_string(),
        tags: vec![],
        skip: None,
        env: None,
        call: None,
        input: serde_json::json!({ "paths": ["pdf/test1.pdf", "pdf/test2.pdf"] }),
        mock_response: None,
        visitor: None,
        args: vec![],
        assertion_recipes: vec![],
        assertions: vec![],
        source: String::new(),
        http: None,
    };

    let (_, args_android) = build_args_and_setup(
        &fixture.input,
        &args,
        KotlinArgsContext {
            fixture: &fixture,
            class_name: "Kreuzberg",
            options_type: None,
            fixture_id: "batch_extract_fixture",
            kotlin_android_style: true,
            config: &ResolvedCrateConfig::default(),
            type_defs: &[],
        },
    );
    assert!(
        args_android.contains("BatchBytesItem"),
        "kotlin_android batch must wrap items in BatchBytesItem, got: {args_android}"
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
