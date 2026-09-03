//! Kotlin codegen unit tests.
//!
//! ~keep This file is already over the repo's 1,000-line file-modularization cap. The
//! `not_error_may_assert_presence` unification added one parameter to `render_assertion`,
//! required at every call site here — 17 pre-existing, unrelated (non-`not_error`) tests each
//! needed one added `true` argument to keep compiling. That mechanical churn, not new
//! functionality, is the entire growth in this file.

use super::args::{KotlinArgsContext, build_args_and_setup};
use super::assertions::render_assertion;
use super::project::render_build_gradle;
use super::test_file::{is_enum_typed, render_test_file_inner};
use crate::core::config::ResolvedCrateConfig;
use crate::e2e::config::ArgMapping;
use crate::e2e::config::E2eConfig;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::{Assertion, Fixture};
use std::collections::{HashMap, HashSet};

/// IR-oracle wiring regression (alef task #64): a field that is IR-reachable
/// (present, non-`binding_excluded`, on some IR type) but missing from the
/// hand-maintained `result_fields` config must still render a real assertion,
/// not a "skipped: field not available" comment — `kotlin/test_method.rs`
/// (shared by both `kotlin` and `kotlin_android`) now threads
/// `FieldResolver::ir_field_sets(type_defs)` into `with_ir_fields`. ~keep
#[test]
fn kotlin_ir_reachable_field_absent_from_result_fields_is_not_skipped() {
    let reachable: HashSet<String> = ["data".to_string()].into_iter().collect();
    let resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    )
    .with_ir_fields(reachable, HashSet::new(), HashSet::new());
    let assertion = Assertion {
        skip: None,
        assertion_type: "equals".to_string(),
        field: Some("data".to_string()),
        value: Some(serde_json::Value::String("hello".to_string())),
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
        &HashSet::new(),
        &HashSet::new(),
        &HashMap::new(),
        false,
        false,
        true,
    );
    assert!(!out.contains("skipped"), "got: {out}");
}

/// The negative-control half of the same regression: `internal_diagnostics`
/// represents a field carrying `#[doc(hidden)]` or `#[cfg_attr(alef,
/// alef(skip))]` in the real struct (a genuine `binding_excluded` field) —
/// NOT `#[serde(skip)]`, which alone does not exclude a field from the
/// binding surface. Even though it is listed in `result_fields` (a stale/
/// wrong config entry), the IR must still win and reject it. ~keep
#[test]
fn kotlin_ir_excluded_field_present_in_result_fields_is_still_skipped() {
    let result_fields: HashSet<String> = ["internal_diagnostics".to_string()].into_iter().collect();
    let excluded: HashSet<String> = ["internal_diagnostics".to_string()].into_iter().collect();
    let resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &result_fields,
        &HashSet::new(),
        &HashSet::new(),
    )
    .with_ir_fields(HashSet::new(), excluded, HashSet::new());
    let assertion = Assertion {
        skip: None,
        assertion_type: "equals".to_string(),
        field: Some("internal_diagnostics".to_string()),
        value: Some(serde_json::Value::String("hello".to_string())),
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
        &HashSet::new(),
        &HashSet::new(),
        &HashMap::new(),
        false,
        false,
        true,
    );
    assert!(out.contains("skipped"), "got: {out}");
}

/// Regression: enum-typed optional fields must route through `?.getValue()`
/// before falling back via `.orEmpty()`. Emitting `.orEmpty().getValue()`
/// is invalid Kotlin because `T?.orEmpty()` is only defined for `String?`.
#[test]
fn assertion_enum_optional_uses_safe_get_value_then_or_empty() {
    let resolver = resolver_helpers::make_resolver_for_finish_reason();
    let mut enum_fields = HashSet::new();
    enum_fields.insert("choices.finish_reason".to_string());
    let assertion = Assertion {
        skip: None,
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
        &HashSet::new(),
        &HashMap::new(),
        false,
        false,
        true,
    );
    assert!(
        out.contains("result.choices().first().finishReason()?.getValue().orEmpty()"),
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
        docs: None,
        requirements: Vec::new(),
        id: "session_fixture".to_string(),
        category: None,
        description: "test fixture".to_string(),
        tags: vec![],
        skip: None,
        env: None,
        setup: Vec::new(),
        call: None,
        input: serde_json::json!({ "config": { "limit": 3 } }),
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
            owner_handle_is_receiver: false,
            enums: &[],
            target_params: crate::e2e::codegen::call_ir::TargetParams::IrAbsent,
        },
    )
    .expect("args build succeeds");

    let rendered = setup.join("\n");
    assert_eq!(args_str, "session");
    assert!(rendered.contains("MAPPER.readValue(\"{\\\"limit\\\":3}\", SessionConfig::class.java)"));
    assert!(rendered.contains("Sample.createSession(sessionConfig)"));
    assert!(!rendered.contains("CrawlConfig"));
}

/// Regression guard for the alef #219 fix: `build_args_and_setup`'s
/// `json_object` `Some(opts_type)` branch previously had no mock-URL-placeholder
/// handling at all — that case was covered solely by test_method.rs's now-removed
/// `deser_lines` mechanism, which duplicated (and has been deleted as) the binding
/// this function emits. Making this function the sole emitter without porting the
/// mock-URL capability over would have silently dropped runtime URL substitution
/// (tests would call a literal `$mock_url` placeholder instead of the mock
/// server) — a silent coverage hole, not a compile error. This pins that the
/// capability now lives here. ~keep
#[test]
fn json_object_arg_with_mock_url_placeholder_binds_once_at_runtime() {
    let args = vec![ArgMapping {
        name: "request".to_string(),
        field: "input.request".to_string(),
        arg_type: "json_object".to_string(),
        optional: false,
        owned: false,
        element_type: None,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    }];
    let fixture = Fixture {
        id: "request_fixture".to_string(),
        description: "test fixture".to_string(),
        input: serde_json::json!({ "request": { "url": "$mock_url/upload" } }),
        ..Fixture::default()
    };

    let (setup, args_str) = build_args_and_setup(
        &fixture.input,
        &args,
        KotlinArgsContext {
            fixture: &fixture,
            class_name: "Sample",
            options_type: Some("UploadRequest"),
            fixture_id: &fixture.id,
            kotlin_android_style: false,
            config: &ResolvedCrateConfig::default(),
            type_defs: &[],
            owner_handle_is_receiver: false,
            enums: &[],
            target_params: crate::e2e::codegen::call_ir::TargetParams::IrAbsent,
        },
    )
    .expect("args build succeeds");

    let rendered = setup.join("\n");
    assert_eq!(args_str, "request");
    // Property 1: a binding must actually be produced before counting it.
    assert!(
        rendered.contains("val request = MAPPER.readValue(requestJson, UploadRequest::class.java)"),
        "expected a runtime-bound readValue call, got:\n{rendered}"
    );
    let binding_count = rendered.matches("val request =").count();
    assert_eq!(
        binding_count, 1,
        "expected exactly one `val request =` binding, got {binding_count}:\n{rendered}"
    );
    assert!(
        rendered.contains(".replace(\"\\$mock_url\", requestMockBaseUrl)"),
        "expected the mock URL placeholder to be swapped in at runtime, got:\n{rendered}"
    );
}

/// Regression (issue #309, third instance this session): `alef` fixed three of four "generated
/// data-class constructor param has no Kotlin default" cases (0.82.2, commit `e47a5bade`) by
/// materialising a JSON stub through `KotlinFillContext` on the `handle` arg path. This is the
/// fourth case: a `json_object` arg with no fixture value, whose target actually requires it,
/// previously spliced a bare `TypeName()` unconditionally — which does not compile when
/// `TypeName` is not in `default_constructible_type_names` (e.g. `ExtractionConfig`, bare only
/// because `url: UrlExtractionConfig` is bare, itself bare only because `crawl: CrawlConfig` is
/// bare, itself bare only because `crawl.ssrf: SsrfPolicy` has no Kotlin default — the same
/// nested shape `fill_missing_required_kotlin_fields`'s own doc comment already worked through
/// for the `handle` path). This pins that the `json_object` fallback now goes through the exact
/// same recursive stub machinery, all the way down: `{"url":{"crawl":{"ssrf":{}}}}`, not a
/// blind `{}` at any level. ~keep
#[test]
fn json_object_arg_without_default_constructor_falls_back_to_a_json_stub() {
    use crate::core::ir::{DefaultValue, FieldDef, PrimitiveType, TypeDef, TypeRef};

    let ssrf_policy = TypeDef {
        name: "SsrfPolicy".to_string(),
        rust_path: "crawlberg::SsrfPolicy".to_string(),
        has_default: true,
        fields: vec![FieldDef {
            name: "deny_private".to_string(),
            ty: TypeRef::Primitive(PrimitiveType::Bool),
            typed_default: Some(DefaultValue::BoolLiteral(true)),
            ..Default::default()
        }],
        ..Default::default()
    };
    let crawl_config = TypeDef {
        name: "CrawlConfig".to_string(),
        rust_path: "crawlberg::CrawlConfig".to_string(),
        has_default: true,
        fields: vec![
            FieldDef {
                name: "respect_robots_txt".to_string(),
                ty: TypeRef::Primitive(PrimitiveType::Bool),
                typed_default: Some(DefaultValue::BoolLiteral(false)),
                ..Default::default()
            },
            FieldDef {
                name: "ssrf".to_string(),
                ty: TypeRef::Named("SsrfPolicy".to_string()),
                // Required, no Kotlin default -- mirrors the real `crawlberg::CrawlConfig::ssrf`
                // field the `fill_missing_required_kotlin_fields` doc comment already names.
                typed_default: None,
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let url_extraction_config = TypeDef {
        name: "UrlExtractionConfig".to_string(),
        rust_path: "xberg::UrlExtractionConfig".to_string(),
        has_default: true,
        fields: vec![FieldDef {
            name: "crawl".to_string(),
            ty: TypeRef::Named("CrawlConfig".to_string()),
            // The real field's default is `UrlExtractionConfig::default_xberg_crawl_config()`,
            // a `PublicFunctionCall` alef cannot fold across the `..Default::default()` spread
            // in its body -- `kotlin_field_default` correctly renders no Kotlin default for it,
            // and this stays permanent: the value depends on a foreign crate's `impl Default`
            // alef's constant-folder cannot read across a crate boundary. ~keep
            typed_default: Some(DefaultValue::PublicFunctionCall(
                "UrlExtractionConfig::default_xberg_crawl_config".to_string(),
            )),
            ..Default::default()
        }],
        ..Default::default()
    };
    let extraction_config = TypeDef {
        name: "ExtractionConfig".to_string(),
        rust_path: "xberg::ExtractionConfig".to_string(),
        has_default: true,
        fields: vec![FieldDef {
            name: "url".to_string(),
            ty: TypeRef::Named("UrlExtractionConfig".to_string()),
            typed_default: Some(DefaultValue::Empty),
            ..Default::default()
        }],
        ..Default::default()
    };
    let type_defs = [ssrf_policy, crawl_config, url_extraction_config, extraction_config];

    let args = vec![ArgMapping {
        name: "options".to_string(),
        field: "input.options".to_string(),
        arg_type: "json_object".to_string(),
        optional: true,
        owned: false,
        element_type: None,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    }];
    let fixture = Fixture {
        id: "extraction_fixture".to_string(),
        description: "test fixture".to_string(),
        input: serde_json::json!({}),
        ..Fixture::default()
    };

    let (setup, args_str) = build_args_and_setup(
        &fixture.input,
        &args,
        KotlinArgsContext {
            fixture: &fixture,
            class_name: "Sample",
            options_type: Some("ExtractionConfig"),
            fixture_id: &fixture.id,
            kotlin_android_style: true,
            config: &ResolvedCrateConfig::default(),
            type_defs: &type_defs,
            owner_handle_is_receiver: false,
            enums: &[],
            target_params: crate::e2e::codegen::call_ir::TargetParams::IrAbsent,
        },
    )
    .expect("args build succeeds");

    let rendered = setup.join("\n");
    assert_eq!(args_str, "optionsDefault");
    assert_eq!(
        rendered,
        "val optionsDefault = MAPPER.readValue(\"{\\\"url\\\":{\\\"crawl\\\":{\\\"ssrf\\\":{}}}}\", \
         ExtractionConfig::class.java)",
        "expected the fully recursive JSON stub down to `ssrf`, got:\n{rendered}"
    );
    assert!(
        !rendered.contains("ExtractionConfig()"),
        "ExtractionConfig is not zero-arg constructible (`url` has no Kotlin default); \
         the bare constructor must never be emitted, got:\n{rendered}"
    );
}

/// Resolver for an optional field on a non-array result, e.g. `data` directly
/// on the result (mirrors `action_results[0].data` after array-index resolution
/// down to the leaf field on the accessed element).
fn make_resolver_for_optional_field(field: &str) -> FieldResolver {
    let mut optional = HashSet::new();
    optional.insert(field.to_string());
    FieldResolver::new(
        &HashMap::new(),
        &optional,
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    )
}

#[test]
fn not_empty_is_type_aware_for_nullable_values() {
    let cases = [
        ("quality_score", false, "result.qualityScore() != null"),
        ("keywords", true, "result.keywords()?.isNotEmpty() == true"),
    ];

    for (field, is_collection, expected) in cases {
        let mut optional = HashSet::new();
        optional.insert(field.to_string());
        let mut arrays = HashSet::new();
        if is_collection {
            arrays.insert(field.to_string());
        }
        let resolver = FieldResolver::new(&HashMap::new(), &optional, &HashSet::new(), &arrays, &HashSet::new());
        let assertion = Assertion {
            skip: None,
            assertion_type: "not_empty".to_string(),
            field: Some(field.to_string()),
            value: None,
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
            &HashSet::new(),
            &HashSet::new(),
            &HashMap::new(),
            false,
            false,
            true,
        );
        assert!(out.contains(expected), "field {field}: {out}");
        assert!(!out.contains("toString"), "field {field}: {out}");
    }
}

/// Regression: an optional field whose Kotlin type is `Any?` (mapped from Rust
/// `Option<serde_json::Value>`) must not render a bare `.orEmpty()` — that's a
/// `String?`/`CharSequence?` extension and does not resolve on `Any?`, so the
/// generated Kotlin fails with "Unresolved reference 'orEmpty'". It must instead
/// stringify through a null-safe call first.
#[test]
fn assertion_json_scalar_optional_field_stringifies_before_or_empty() {
    let resolver = make_resolver_for_optional_field("data");
    let mut json_scalar_fields = HashSet::new();
    json_scalar_fields.insert("data".to_string());
    let assertion = Assertion {
        skip: None,
        assertion_type: "contains".to_string(),
        field: Some("data".to_string()),
        value: Some(serde_json::Value::String("JS Test Page".to_string())),
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
        &HashSet::new(),
        &json_scalar_fields,
        &HashMap::new(),
        false,
        true,
        true,
    );
    assert!(
        out.contains("result.data?.toString().orEmpty().contains(\"JS Test Page\")"),
        "expected null-safe stringify before orEmpty() for an Any? field, got: {out}"
    );
    assert!(
        !out.contains("result.data.orEmpty()"),
        "must not emit a bare .orEmpty() on Any?, got: {out}"
    );
}

#[test]
fn assertion_json_scalar_matches_configured_array_wildcard() {
    let resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::from(["action_results".to_string()]),
        &HashSet::from(["action_results".to_string()]),
        &HashSet::new(),
    );
    let assertion = Assertion {
        skip: None,
        assertion_type: "contains".to_string(),
        field: Some("action_results[0].data".to_string()),
        value: Some(serde_json::json!("JS Test Page")),
        values: None,
        method: None,
        check: None,
        args: None,
        return_type: None,
    };
    let mut output = String::new();
    render_assertion(
        &mut output,
        &assertion,
        "result",
        "",
        &resolver,
        false,
        false,
        &HashSet::new(),
        &HashSet::from(["action_results[].data".to_string()]),
        &HashMap::new(),
        false,
        true,
        true,
    );
    assert!(output.contains("data?.toString().orEmpty().contains"), "got: {output}");
    assert!(!output.contains("data.orEmpty()"), "got: {output}");
}

#[test]
fn assertion_json_scalar_and_nullable_root_are_stringified_for_contains() {
    let resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    );
    let assertion = Assertion {
        skip: None,
        assertion_type: "contains".to_string(),
        field: Some("data".to_string()),
        value: Some(serde_json::json!("needle")),
        values: None,
        method: None,
        check: None,
        args: None,
        return_type: None,
    };
    let mut field_output = String::new();
    render_assertion(
        &mut field_output,
        &assertion,
        "result",
        "",
        &resolver,
        true,
        false,
        &HashSet::new(),
        &HashSet::from(["data".to_string()]),
        &HashMap::new(),
        false,
        true,
        true,
    );
    assert!(
        field_output.contains("result?.toString().orEmpty().contains(\"needle\")"),
        "got: {field_output}"
    );

    let mut root_output = String::new();
    let mut root_assertion = assertion;
    root_assertion.field = None;
    render_assertion(
        &mut root_output,
        &root_assertion,
        "result",
        "",
        &resolver,
        true,
        true,
        &HashSet::new(),
        &HashSet::new(),
        &HashMap::new(),
        false,
        true,
        true,
    );
    assert!(
        root_output.contains("result?.toString().orEmpty().contains(\"needle\")"),
        "got: {root_output}"
    );
}

/// Regression (negative direction): a genuinely `String?` optional field must
/// keep rendering the plain `.orEmpty()` fallback — `fields_json_scalar` is
/// opt-in per field, so fields absent from it are unaffected.
#[test]
fn assertion_string_optional_field_still_uses_plain_or_empty() {
    let resolver = make_resolver_for_optional_field("title");
    let assertion = Assertion {
        skip: None,
        assertion_type: "contains".to_string(),
        field: Some("title".to_string()),
        value: Some(serde_json::Value::String("Example".to_string())),
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
        &HashSet::new(),
        &HashSet::new(),
        &HashMap::new(),
        false,
        true,
        true,
    );
    assert!(
        out.contains("result.title.orEmpty().contains(\"Example\")"),
        "expected plain .orEmpty() for a String? field, got: {out}"
    );
    assert!(
        !out.contains("?.toString().orEmpty()"),
        "must not stringify a genuine String? field, got: {out}"
    );
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
        skip: None,
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
        &HashSet::new(),
        &HashMap::new(),
        false,
        false,
        true,
    );
    assert!(
        out.contains("result.choices().first().finishReason().getValue()"),
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
        skip: None,
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
        &HashSet::new(),
        &HashMap::new(),
        false,
        false,
        true,
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
        &HashSet::new(),
        &HashMap::new(),
        false,
        false,
        true,
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
                version: Default::default(),
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
                serde_with: None,
                serde_skip_serializing_if: false,
                serde_skip: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
                original_type: None,
            },
            FieldDef {
                version: Default::default(),
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
                serde_with: None,
                serde_skip_serializing_if: false,
                serde_skip: false,
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
        serde_container_default: false,
        serde_container_conversion: Default::default(),
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
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
        skip: None,
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
        &HashSet::new(),
        &HashMap::new(),
        false,
        false,
        true,
    );
    assert!(
        out.contains(".getValue()"),
        "auto-detected enum field must route through .getValue(), got: {out}"
    );
}

fn make_not_error_fixture_test_file(id: &str, assertions: Vec<Assertion>) -> String {
    let fixture = Fixture {
        docs: None,
        requirements: Vec::new(),
        id: id.to_string(),
        category: None,
        description: "not_error import test".to_string(),
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
        assertions,
        source: String::new(),
        http: None,
        asyncapi: None,
        websocket: None,
        preserve_input_urls: false,
    };
    let e2e_config = E2eConfig::default();
    let config = ResolvedCrateConfig::default();
    let type_defs: Vec<crate::core::ir::TypeDef> = Vec::new();
    render_test_file_inner(
        "not_error",
        &[&fixture],
        "SampleClient",
        "getItem",
        "dev.sample_crate",
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
    .expect("not_error test file renders")
}

/// Regression: `import kotlin.test.assertNotNull` used to be written into every generated
/// Kotlin test file unconditionally (`test_file.rs`), whether or not any fixture in that file
/// renders `not_error::render_not_error`'s non-streaming branch -- the only call site that
/// spells `assertNotNull`. A file with no `not_error` assertion at all never calls that branch,
/// so the import was dead: Kotlin's unused-import lint flags it the same way checkstyle flags
/// an unused `java.util.List` import in the java backend.
#[test]
fn kotlin_test_file_without_not_error_fixture_omits_assert_not_null_import() {
    let out = make_not_error_fixture_test_file(
        "equals_only",
        vec![Assertion {
            assertion_type: "equals".to_string(),
            field: Some("id".to_string()),
            value: Some(serde_json::Value::String("abc".to_string())),
            ..Assertion::default()
        }],
    );
    assert!(
        !out.contains("import kotlin.test.assertNotNull"),
        "a file with no `not_error` assertion must not import assertNotNull, got:\n{out}"
    );
}

/// Companion to the test above: a fixture that DOES declare a (non-streaming) `not_error`
/// assertion drives `render_not_error`'s `assertNotNull(...)` branch, so the import must be
/// present.
#[test]
fn kotlin_test_file_with_not_error_fixture_imports_assert_not_null() {
    let out = make_not_error_fixture_test_file(
        "not_error_only",
        vec![Assertion {
            assertion_type: "not_error".to_string(),
            ..Assertion::default()
        }],
    );
    assert!(
        out.contains("import kotlin.test.assertNotNull"),
        "a file with a non-streaming `not_error` assertion must import assertNotNull, got:\n{out}"
    );
    assert!(
        out.contains("assertNotNull(result, \"expected non-null result\")"),
        "the not_error assertion itself must still render, got:\n{out}"
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
        "../../test_documents",
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
        "../../test_documents",
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
        "../../test_documents",
    );
    assert!(
        out.contains("packages/kotlin/build/libs/sample_router-0.15.6-rc.3.jar"),
        "expected local jar reference, got:\n{out}"
    );
}

/// Regression: the test-documents directory name in the generated `workingDir` must
/// come from `E2eConfig::test_documents_dir` (via `test_documents_relative_from`), not
/// a hard-coded `"test_documents"` literal -- see CLAUDE.md's `project-agnostic-codegen`
/// rule. A consumer that configures a non-default `test_documents_dir` must see that
/// name reflected in the generated build.gradle.kts. Mirrors the kotlin_android
/// regression (`kotlin_android::project::build_gradle_local_mode_working_dir_uses_configured_test_documents_dir`).
#[test]
fn build_gradle_working_dir_uses_configured_test_documents_dir() {
    use crate::e2e::codegen::E2eCodegen;

    let config = ResolvedCrateConfig::default();
    let e2e_config = E2eConfig {
        test_documents_dir: "fixture_files".to_string(),
        ..E2eConfig::default()
    };

    let files = super::KotlinE2eCodegen
        .generate(&[], &e2e_config, &config, &[], &[], &[], &[])
        .expect("kotlin e2e generation succeeds on an empty fixture set");

    let build_gradle = files
        .iter()
        .find(|f| f.path.ends_with("build.gradle.kts"))
        .expect("build.gradle.kts must be generated");
    assert!(
        build_gradle.content.contains("../../fixture_files"),
        "workingDir must resolve the configured test_documents_dir, got:\n{}",
        build_gradle.content
    );
    assert!(
        !build_gradle.content.contains("../../test_documents"),
        "must not hard-code the literal `test_documents`, got:\n{}",
        build_gradle.content
    );
}

/// Regression: when HTTP fixtures are present the generated `MockServerListener`
/// implements `LauncherSessionListener`, referencing
/// `org.junit.platform.launcher.{LauncherSession, LauncherSessionListener}` as
/// compile-time symbols. Without `junit-platform-launcher` on the test
/// classpath, Kotlin compilation fails with "Unresolved reference 'launcher'".
#[test]
fn build_gradle_kotlin_declares_junit_platform_launcher_when_http_fixtures_present() {
    let out = render_build_gradle(
        "sample_project-kotlin",
        "dev.sample_project",
        "0.1.0",
        crate::e2e::config::DependencyMode::Local,
        true,
        "../../test_documents",
    );
    assert!(
        out.contains(r#"testImplementation("org.junit.platform:junit-platform-launcher:"#),
        "build.gradle.kts must declare junit-platform-launcher when HTTP fixtures are present, got:\n{out}"
    );
}

/// Regression: without HTTP fixtures, `MockServerListener` is never emitted, so
/// the launcher dependency is unnecessary weight and must be omitted.
#[test]
fn build_gradle_kotlin_omits_junit_platform_launcher_without_http_fixtures() {
    let out = render_build_gradle(
        "sample_project-kotlin",
        "dev.sample_project",
        "0.1.0",
        crate::e2e::config::DependencyMode::Local,
        false,
        "../../test_documents",
    );
    assert!(
        !out.contains("junit-platform-launcher"),
        "build.gradle.kts must not declare junit-platform-launcher without HTTP fixtures, got:\n{out}"
    );
}

/// Regression for the InteractionTest.kt compile break: a fixture field path
/// carrying a virtual namespace prefix (`interaction.`, stripped by
/// `accessor()` via `namespace_stripped_path` when it builds `field_expr`)
/// must still be recognized as a `fields_json_scalar` field when the
/// consumer's `alef.toml` configures the *stripped* struct path
/// (`action_results[].data`, not `interaction.action_results[].data`).
/// Before this fix `field_is_json_scalar` compared against the unstripped
/// fixture path and always missed, so the field fell through to the plain
/// `.orEmpty()` fallback — which does not resolve on `Any?` (a JSON-scalar
/// field's real Kotlin type) and left the generated e2e module uncompilable.
#[test]
fn assertion_json_scalar_matches_namespace_stripped_path() {
    let resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::from(["action_results[].data".to_string()]),
        &HashSet::from(["action_results".to_string()]),
        &HashSet::from(["action_results".to_string()]),
        &HashSet::new(),
    );
    let assertion = Assertion {
        skip: None,
        assertion_type: "contains".to_string(),
        field: Some("interaction.action_results[0].data".to_string()),
        value: Some(serde_json::json!("JS Test Page")),
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
        &HashSet::new(),
        &HashSet::from(["action_results[].data".to_string()]),
        &HashMap::new(),
        false,
        true,
        true,
    );
    assert!(
        !out.contains(".data.orEmpty()"),
        "must not emit a bare `.orEmpty()` on the Any? field behind a namespace-prefixed path, got:\n{out}"
    );
    assert!(
        out.contains(".data?.toString().orEmpty().contains(\"JS Test Page\")"),
        "expected the null-safe stringify path for the namespace-prefixed json-scalar field, got:\n{out}"
    );
}

/// Companion negative case for the namespace-stripped lookup above: a field
/// behind the same `interaction.` prefix that is only in `fields_optional`
/// (a genuine `String?`, not a JSON scalar) must keep emitting the plain
/// `.orEmpty()` fallback. This is the test that would catch an over-broad fix
/// that treats every namespace-stripped optional field as a JSON scalar.
#[test]
fn assertion_namespace_stripped_string_field_still_uses_plain_or_empty() {
    let resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::from(["action_results[].action_type".to_string()]),
        &HashSet::from(["action_results".to_string()]),
        &HashSet::from(["action_results".to_string()]),
        &HashSet::new(),
    );
    let assertion = Assertion {
        skip: None,
        assertion_type: "contains".to_string(),
        field: Some("interaction.action_results[0].action_type".to_string()),
        value: Some(serde_json::json!("executeJs")),
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
        &HashSet::new(),
        // No `fields_json_scalar` entry for `action_type` — it is a real `String?`.
        &HashSet::from(["action_results[].data".to_string()]),
        &HashMap::new(),
        false,
        true,
        true,
    );
    assert!(
        out.contains(".actionType.orEmpty().contains(\"executeJs\")"),
        "expected the plain .orEmpty() fallback for a genuinely nullable String field, got:\n{out}"
    );
    assert!(
        !out.contains("?.toString().orEmpty()"),
        "a non-json-scalar field must not be routed through the Any? stringify path, got:\n{out}"
    );
}

/// Field-path spelling: `fields_json_scalar` (like `fields_optional`) accepts
/// both the array-wildcard spelling (`action_results[].data`) and the
/// dotted/de-indexed spelling (`action_results.data`) that the consumer's
/// `alef.toml` also lists side by side for other keys. Both must resolve to
/// the same JSON-scalar classification for a concrete indexed fixture path.
#[test]
fn assertion_json_scalar_accepts_both_bracket_and_dotted_spellings() {
    let make_resolver = || {
        FieldResolver::new(
            &HashMap::new(),
            &HashSet::from(["action_results[].data".to_string()]),
            &HashSet::from(["action_results".to_string()]),
            &HashSet::from(["action_results".to_string()]),
            &HashSet::new(),
        )
    };
    let assertion = Assertion {
        skip: None,
        assertion_type: "contains".to_string(),
        field: Some("action_results[0].data".to_string()),
        value: Some(serde_json::json!("JS Test Page")),
        values: None,
        method: None,
        check: None,
        args: None,
        return_type: None,
    };

    for spelling in ["action_results[].data", "action_results.data"] {
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            "",
            &make_resolver(),
            false,
            false,
            &HashSet::new(),
            &HashSet::from([spelling.to_string()]),
            &HashMap::new(),
            false,
            true,
            true,
        );
        assert!(
            out.contains(".data?.toString().orEmpty().contains"),
            "spelling `{spelling}` must resolve as a json-scalar field, got:\n{out}"
        );
    }
}

#[cfg(test)]
mod android;
#[cfg(test)]
mod resolver_helpers;
#[cfg(test)]
mod wildcard;
