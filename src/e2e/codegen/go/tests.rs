#![allow(clippy::print_stdout, clippy::print_stderr)]

use crate::e2e::config::{ArgMapping, CallConfig, E2eConfig};
use crate::e2e::fixture::{Assertion, Fixture};

use super::setup::build_args_and_setup;
use super::test_file::{GoTestFileContext, render_test_file};
use super::test_function::{GoTestFunctionContext, render_test_function};

fn make_fixture(id: &str) -> Fixture {
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
        input: serde_json::Value::Null,
        mock_response: Some(crate::e2e::fixture::MockResponse {
            status: 200,
            body: Some(serde_json::Value::Null),
            stream_chunks: None,
            headers: std::collections::BTreeMap::new(),
        }),
        source: String::new(),
        http: None,
        asyncapi: None,
        websocket: None,
        preserve_input_urls: false,
        assertions: vec![Assertion {
            assertion_type: "not_error".to_string(),
            ..Default::default()
        }],
        visitor: None,
        args: vec![],
        assertion_recipes: vec![],
    }
}

// Regression for a bug where the snippet template hardcoded `var typedError *pkg.Error`
// (a pointer). Alef's Go error emitter generates `Error() string` on a value receiver, so
// the concrete error is never a `*Error` and `errors.As` against a pointer target silently
// never matches. Asserting `!body.contains("*pkg.Error")` alone would pass on a body missing
// `typedError` entirely, so this also pins the exact non-pointer declaration. ~keep
#[test]
fn snippet_body_declares_typed_error_by_value_not_by_pointer() {
    let mut fixture = make_fixture("invalid_input");
    fixture.assertions = vec![Assertion {
        assertion_type: "error".to_string(),
        ..Default::default()
    }];
    let e2e_config = E2eConfig {
        call: CallConfig {
            function: "parse".to_string(),
            module: "example.com/sample".to_string(),
            returns_result: true,
            ..CallConfig::default()
        },
        ..E2eConfig::default()
    };
    let config = crate::core::config::ResolvedCrateConfig::default();

    let body =
        super::snippet::render_snippet_body(&fixture, &e2e_config, &config, &[], &[], &[]).expect("snippet renders");

    assert!(body.contains("var typedError pkg.Error"), "{body}");
    assert!(!body.contains("var typedError *pkg.Error"), "{body}");
}

/// snake_case function names in `[e2e.call]` must be routed through `to_go_name`
/// so the emitted Go call uses the idiomatic CamelCase (e.g. `CleanExtractedText`
/// instead of `clean_extracted_text`).
#[test]
fn test_go_method_name_uses_go_casing() {
    let e2e_config = E2eConfig {
        call: CallConfig {
            function: "clean_extracted_text".to_string(),
            module: "github.com/example/mylib".to_string(),
            result_var: "result".to_string(),
            returns_result: true,
            ..CallConfig::default()
        },
        ..E2eConfig::default()
    };

    let fixture = make_fixture("basic_text");
    let mut out = String::new();
    let config = crate::core::config::ResolvedCrateConfig::default();
    let type_defs: Vec<crate::core::ir::TypeDef> = Vec::new();
    let enums: Vec<crate::core::ir::EnumDef> = Vec::new();
    render_test_function(
        &mut out,
        &fixture,
        GoTestFunctionContext {
            import_alias: "sample_crate",
            e2e_config: &e2e_config,
            adapters: &[],
            data_enum_names: &std::collections::HashSet::new(),
            config: &config,
            type_defs: &type_defs,
            enums: &enums,
            errors: &[],
            functions: &[],
        },
    );

    assert!(
        out.contains("sample_crate.CleanExtractedText("),
        "expected Go-cased method name 'CleanExtractedText', got:\n{out}"
    );
    assert!(
        !out.contains("sample_crate.clean_extracted_text("),
        "must not emit raw snake_case method name, got:\n{out}"
    );
}

/// Regression test for alef task #81: go had no fallback at all for a dropped
/// field assertion — `is_valid_for_result` rejects the field, `render_assertion`
/// emits a skip comment, and (until this fix) nothing else ever consulted that
/// comment. This pins that the skip comment carries the exact marker text the
/// shared `fail_on_unavailable_field_markers` mechanism (src/e2e/codegen/mod.rs)
/// matches on, and — because `out` accumulates every fixture's function in the
/// same buffer (see `test_file.rs`) — that a PRECEDING fixture with no issues does
/// not get misattributed the following fixture's dropped field.
#[test]
fn dropped_field_assertion_carries_the_marker_and_is_correctly_attributed_per_fixture() {
    let e2e_config = E2eConfig {
        result_fields: std::collections::HashSet::from(["content".to_string()]),
        call: CallConfig {
            function: "process".to_string(),
            module: "github.com/example/mylib".to_string(),
            result_var: "result".to_string(),
            returns_result: true,
            ..CallConfig::default()
        },
        ..E2eConfig::default()
    };
    let config = crate::core::config::ResolvedCrateConfig::default();
    let type_defs: Vec<crate::core::ir::TypeDef> = Vec::new();
    let enums: Vec<crate::core::ir::EnumDef> = Vec::new();
    let mut out = String::new();

    // First fixture: no field assertions at all — must not pick up anything
    // appended by the second fixture's render.
    render_test_function(
        &mut out,
        &make_fixture("clean_smoke"),
        GoTestFunctionContext {
            import_alias: "sample_crate",
            e2e_config: &e2e_config,
            adapters: &[],
            data_enum_names: &std::collections::HashSet::new(),
            config: &config,
            type_defs: &type_defs,
            enums: &enums,
            errors: &[],
            functions: &[],
        },
    );
    let clean_len = out.len();

    // Second fixture: asserts on a field absent from `result_fields`.
    let mut dirty_fixture = make_fixture("dirty_smoke");
    dirty_fixture.assertions = vec![Assertion {
        assertion_type: "equals".to_string(),
        field: Some("nonexistent_field".to_string()),
        value: Some(serde_json::json!("x")),
        ..Default::default()
    }];
    render_test_function(
        &mut out,
        &dirty_fixture,
        GoTestFunctionContext {
            import_alias: "sample_crate",
            e2e_config: &e2e_config,
            adapters: &[],
            data_enum_names: &std::collections::HashSet::new(),
            config: &config,
            type_defs: &type_defs,
            enums: &enums,
            errors: &[],
            functions: &[],
        },
    );

    assert!(
        !out[..clean_len].contains("not available"),
        "the first fixture's own render must carry no skip marker, got:\n{}",
        &out[..clean_len]
    );
    assert!(
        out[clean_len..].contains("field 'nonexistent_field' not available on result type"),
        "the second fixture's own render must carry the skip marker, got:\n{}",
        &out[clean_len..]
    );
}

/// Regression test for alef task #81: a `!effective_returns_result && result_is_simple`
/// function (no error return to check, plain non-error-returning signature) whose
/// fixture's only declared assertion is `not_error` used to discard the call result
/// to `_` and assert literally nothing — the one Go shape with no fallback of any
/// kind, since the sibling branches (`returns_void`, `returns_result`) both still
/// have a real `if err != nil { t.Fatalf(...) }` check to fall back on. It must now
/// bind the result and assert non-nil instead of silently discarding it.
#[test]
fn declared_not_error_only_fixture_on_a_simple_errorless_call_still_gets_a_real_assertion() {
    let e2e_config = E2eConfig {
        call: CallConfig {
            function: "normalize".to_string(),
            module: "github.com/example/mylib".to_string(),
            result_var: "result".to_string(),
            returns_result: false,
            result_is_simple: true,
            ..CallConfig::default()
        },
        ..E2eConfig::default()
    };
    // `make_fixture` declares exactly one `not_error` assertion by default — the
    // "declared but unusable here" case this fix targets.
    let fixture = make_fixture("normalize_smoke");
    let mut out = String::new();
    let config = crate::core::config::ResolvedCrateConfig::default();
    let type_defs: Vec<crate::core::ir::TypeDef> = Vec::new();
    let enums: Vec<crate::core::ir::EnumDef> = Vec::new();
    render_test_function(
        &mut out,
        &fixture,
        GoTestFunctionContext {
            import_alias: "sample_crate",
            e2e_config: &e2e_config,
            adapters: &[],
            data_enum_names: &std::collections::HashSet::new(),
            config: &config,
            type_defs: &type_defs,
            enums: &enums,
            errors: &[],
            functions: &[],
        },
    );

    assert!(
        !out.contains("_ = sample_crate.Normalize("),
        "must bind the result instead of discarding it, got:\n{out}"
    );
    assert!(out.contains("result := sample_crate.Normalize("), "got:\n{out}");
    assert!(
        out.contains("if result == nil {") && out.contains("t.Fatalf(\"expected non-nil result\")"),
        "expected a real non-nil fallback assertion, got:\n{out}"
    );
}

/// ~keep `returns_void` means the Go signature's ONLY return is `error`, where `nil`
/// is success. Combined with `result_is_simple` it used to take the errorless-value
/// branch and emit `result := f(); if result == nil { fatal }` -- an assertion that
/// passes only when the call FAILS. This inverted 19 generated tests across the
/// plugin-registry surface. `returns_void` must win over `result_is_simple`.
#[test]
fn returns_void_with_simple_result_asserts_the_error_is_nil_not_non_nil() {
    let e2e_config = E2eConfig {
        call: CallConfig {
            function: "clear_ocr_backends".to_string(),
            module: "github.com/example/mylib".to_string(),
            result_var: "result".to_string(),
            returns_result: false,
            returns_void: true,
            result_is_simple: true,
            ..CallConfig::default()
        },
        ..E2eConfig::default()
    };
    let fixture = make_fixture("clear_ocr_backends_smoke");
    let mut out = String::new();
    let config = crate::core::config::ResolvedCrateConfig::default();
    let type_defs: Vec<crate::core::ir::TypeDef> = Vec::new();
    let enums: Vec<crate::core::ir::EnumDef> = Vec::new();
    render_test_function(
        &mut out,
        &fixture,
        GoTestFunctionContext {
            import_alias: "sample_crate",
            e2e_config: &e2e_config,
            adapters: &[],
            data_enum_names: &std::collections::HashSet::new(),
            config: &config,
            type_defs: &type_defs,
            enums: &enums,
            errors: &[],
            functions: &[],
        },
    );

    assert!(
        !out.contains("expected non-nil result"),
        "an error-only return must never be asserted non-nil -- nil IS success, got:\n{out}"
    );
    assert!(
        out.contains("err := sample_crate.ClearOcrBackends("),
        "expected the error-return call shape, got:\n{out}"
    );
    assert!(
        out.contains("if err != nil {") && out.contains("t.Fatalf(\"call failed: %v\", err)"),
        "expected the standard error check, got:\n{out}"
    );
}

/// Positive control for the same fix: a fixture with genuinely zero declared
/// assertions is left exactly as before (deliberate smoke-test contract) — the
/// result is still discarded, since there is nothing to fall back on behalf of.
#[test]
fn zero_declared_assertions_on_a_simple_errorless_call_still_discards_the_result() {
    let e2e_config = E2eConfig {
        call: CallConfig {
            function: "normalize".to_string(),
            module: "github.com/example/mylib".to_string(),
            result_var: "result".to_string(),
            returns_result: false,
            result_is_simple: true,
            ..CallConfig::default()
        },
        ..E2eConfig::default()
    };
    let mut fixture = make_fixture("normalize_smoke");
    fixture.assertions = Vec::new();
    let mut out = String::new();
    let config = crate::core::config::ResolvedCrateConfig::default();
    let type_defs: Vec<crate::core::ir::TypeDef> = Vec::new();
    let enums: Vec<crate::core::ir::EnumDef> = Vec::new();
    render_test_function(
        &mut out,
        &fixture,
        GoTestFunctionContext {
            import_alias: "sample_crate",
            e2e_config: &e2e_config,
            adapters: &[],
            data_enum_names: &std::collections::HashSet::new(),
            config: &config,
            type_defs: &type_defs,
            enums: &enums,
            errors: &[],
            functions: &[],
        },
    );

    assert!(
        out.contains("_ = sample_crate.Normalize("),
        "a fixture with zero declared assertions is an intentional smoke test and must \
         still discard the result, got:\n{out}"
    );
    assert!(!out.contains("t.Fatalf(\"expected non-nil result\")"), "got:\n{out}");
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
    let data_enum_names = std::collections::HashSet::new();
    let (package_decls, setup, args_str) = build_args_and_setup(
        &fixture.input,
        &args,
        "pkg",
        Some("SessionConfig"),
        &fixture,
        false,
        false,
        &data_enum_names,
        &crate::core::config::ResolvedCrateConfig::default(),
        &[],
        &[],
        false,
        crate::e2e::codegen::call_ir::TargetParams::IrAbsent,
    )
    .expect("args render");

    let rendered = setup.join("\n");
    assert!(package_decls.is_empty());
    assert_eq!(args_str, "session");
    assert!(rendered.contains("var sessionConfig pkg.SessionConfig"));
    assert!(rendered.contains("pkg.CreateSession(&sessionConfig)"));
    assert!(!rendered.contains("CrawlConfig"));
}

/// Regression test: a `result_is_simple` call with a `contains` assertion whose
/// `field` ("result") is not a struct field must still bind the call to the result
/// variable AND emit the `fmt`/`strings` imports.  The assertion renderer ignores
/// the field for `result_is_simple` calls and emits `strings.Contains(fmt.Sprint(result), …)`,
/// so binding to `_` (or omitting the imports) produces uncompilable Go.
#[test]
fn test_result_is_simple_contains_binds_result_and_emits_imports() {
    use crate::core::config::e2e::ArgMapping;

    let e2e_config = E2eConfig {
        call: CallConfig {
            function: "detect_mime_type_from_bytes".to_string(),
            module: "github.com/example/mylib".to_string(),
            result_var: "result".to_string(),
            returns_result: true,
            result_is_simple: true,
            args: vec![ArgMapping {
                name: "content".to_string(),
                field: "input.data".to_string(),
                arg_type: "bytes".to_string(),
                optional: false,
                owned: false,
                element_type: None,
                go_type: None,
                vec_inner_is_ref: false,
                trait_name: None,
            }],
            ..CallConfig::default()
        },
        ..E2eConfig::default()
    };

    let fixture = Fixture {
        docs: None,
        requirements: Vec::new(),
        id: "mime_detect_bytes".to_string(),
        category: None,
        description: "Detect MIME type from file bytes".to_string(),
        tags: vec![],
        skip: None,
        env: None,
        setup: Vec::new(),
        call: None,
        input: serde_json::json!({"data": "pdf/fake_memo.pdf"}),
        mock_response: None,
        source: String::new(),
        http: None,
        asyncapi: None,
        websocket: None,
        preserve_input_urls: false,
        assertions: vec![Assertion {
            assertion_type: "contains".to_string(),
            field: Some("result".to_string()),
            value: Some(serde_json::Value::String("pdf".to_string())),
            ..Default::default()
        }],
        visitor: None,
        args: vec![],
        assertion_recipes: vec![],
    };

    let config = crate::core::config::ResolvedCrateConfig::default();
    let type_defs: Vec<crate::core::ir::TypeDef> = Vec::new();
    let enums: Vec<crate::core::ir::EnumDef> = Vec::new();
    let out = render_test_file(
        "mime_utilities",
        &[&fixture],
        GoTestFileContext {
            go_module_path: "github.com/example/mylib",
            import_alias: "sample_crate",
            e2e_config: &e2e_config,
            adapters: &[],
            data_enum_names: &std::collections::HashSet::new(),
            config: &config,
            type_defs: &type_defs,
            enums: &enums,
            errors: &[],
            functions: &[],
            crate_facts: None,
        },
    );

    assert!(
        out.contains("result, err := sample_crate.DetectMimeTypeFromBytes("),
        "expected the call to bind to `result`, not `_`; got:\n{out}"
    );
    assert!(
        out.contains("strings.Contains(") && out.contains("string("),
        "expected `strings.Contains(string(...))` cast rendering; got:\n{out}"
    );
    assert!(
        !out.contains("\t\"fmt\""),
        "expected fmt import to NOT be emitted (uses string cast not fmt.Sprint); got:\n{out}"
    );
    assert!(
        out.contains("\t\"strings\""),
        "expected the `strings` import to be emitted; got:\n{out}"
    );
}

/// Render a single test function over `assertions` against a result whose only array
/// field is `results`, so the indexed-assertion emitter is exercised in isolation.
fn render_indexed_assertion_function(assertions: Vec<Assertion>) -> String {
    let mut array_fields = std::collections::HashSet::new();
    array_fields.insert("results".to_string());

    let e2e_config = E2eConfig {
        call: CallConfig {
            function: "extract".to_string(),
            module: "github.com/example/mylib".to_string(),
            result_var: "result".to_string(),
            returns_result: true,
            ..CallConfig::default()
        },
        fields_array: array_fields,
        ..E2eConfig::default()
    };

    let mut fixture = make_fixture("batch_results");
    fixture.assertions = assertions;

    let mut out = String::new();
    let config = crate::core::config::ResolvedCrateConfig::default();
    let type_defs: Vec<crate::core::ir::TypeDef> = Vec::new();
    let enums: Vec<crate::core::ir::EnumDef> = Vec::new();
    render_test_function(
        &mut out,
        &fixture,
        GoTestFunctionContext {
            import_alias: "pkg",
            e2e_config: &e2e_config,
            adapters: &[],
            data_enum_names: &std::collections::HashSet::new(),
            config: &config,
            type_defs: &type_defs,
            enums: &enums,
            errors: &[],
            functions: &[],
        },
    );
    out
}

/// A fixture asserting on `results[0]` states that `results` has an element, so the
/// emitted Go must abort on an empty slice and then index unconditionally. Wrapping the
/// assertion in `if len(result.Results) > 0` made the whole check vanish for an empty
/// result, which is how 30 generated Go tests passed without ever asserting anything.
#[test]
fn indexed_assertion_fails_the_test_when_the_collection_is_empty() {
    let out = render_indexed_assertion_function(vec![Assertion {
        assertion_type: "equals".to_string(),
        field: Some("results[0].mime_type".to_string()),
        value: Some(serde_json::Value::String("image/png".to_string())),
        ..Default::default()
    }]);

    let expected = "\tif len(result.Results) == 0 {\n\
         \t\tt.Fatalf(\"expected non-empty %s\", `result.Results`)\n\
         \t}\n\
         \tif string(result.Results[0].MimeType) != `image/png` {\n\
         \t\tt.Errorf(\"equals mismatch: got %v\", result.Results[0].MimeType)\n\
         \t}\n";
    assert!(
        out.contains(expected),
        "expected fatal precondition followed by an unguarded assertion:\n{expected}\ngot:\n{out}"
    );
    assert!(
        !out.contains("if len(result.Results) > 0 {"),
        "the emptiness guard that swallows the assertion must be gone; got:\n{out}"
    );
}

/// `t.Fatalf` aborts the function, so one precondition protects every later index into
/// the same collection. Emitting it per assertion would triple the noise for a fixture
/// that checks three fields of `results[0]`.
#[test]
fn repeated_indexed_assertions_share_one_non_empty_precondition() {
    let out = render_indexed_assertion_function(vec![
        Assertion {
            assertion_type: "equals".to_string(),
            field: Some("results[0].mime_type".to_string()),
            value: Some(serde_json::Value::String("image/png".to_string())),
            ..Default::default()
        },
        Assertion {
            assertion_type: "not_empty".to_string(),
            field: Some("results[0].content".to_string()),
            ..Default::default()
        },
    ]);

    assert_eq!(
        out.matches("t.Fatalf(\"expected non-empty %s\", `result.Results`)")
            .count(),
        1,
        "the precondition should be emitted once per collection; got:\n{out}"
    );
    assert!(
        out.contains("\tif len(result.Results[0].Content) == 0 {\n\t\tt.Errorf(\"expected non-empty value\")\n\t}\n"),
        "the second assertion must still be emitted unguarded; got:\n{out}"
    );
}

/// A fixture that never indexes a collection makes no claim about its length, so no
/// precondition may be invented for it — `not_empty` on the slice itself is the fixture's
/// own way of demanding a non-empty result and must stay a plain, non-fatal check.
#[test]
fn assertion_without_an_index_gets_no_non_empty_precondition() {
    let out = render_indexed_assertion_function(vec![Assertion {
        assertion_type: "not_empty".to_string(),
        field: Some("results".to_string()),
        ..Default::default()
    }]);

    assert!(
        !out.contains("t.Fatalf(\"expected non-empty %s\""),
        "a non-indexed assertion must not gain a fatal precondition; got:\n{out}"
    );
    assert!(
        out.contains("\tif len(result.Results) == 0 {\n\t\tt.Errorf(\"expected non-empty value\")\n\t}\n"),
        "the fixture's own not_empty check must be emitted verbatim; got:\n{out}"
    );
}

/// `len()` does not compile against a Go numeric scalar (e.g. `float64`). A sibling
/// `greater_than_or_equal` assertion against a JSON number on the same field proves the
/// field is a scalar number, so `not_empty` must skip the `len()` call entirely rather than
/// emit code that fails to build. Reverting the fix reintroduces
/// `if len(result.Results[0].QualityScore) == 0 {`, which does not compile.
#[test]
fn not_empty_on_a_numeric_scalar_field_emits_no_len_call() {
    let out = render_indexed_assertion_function(vec![
        Assertion {
            assertion_type: "not_empty".to_string(),
            field: Some("results[0].quality_score".to_string()),
            ..Default::default()
        },
        Assertion {
            assertion_type: "greater_than_or_equal".to_string(),
            field: Some("results[0].quality_score".to_string()),
            value: Some(serde_json::json!(0.0)),
            ..Default::default()
        },
    ]);

    assert!(
        !out.contains("len(result.Results[0].QualityScore)"),
        "not_empty on a numeric scalar must not call len(), which does not compile against \
         a scalar Go type; got:\n{out}"
    );
    assert!(
        !out.contains("t.Errorf(\"expected non-empty value\")"),
        "a numeric scalar always carries a value in Go, so not_empty has nothing to check; got:\n{out}"
    );
}

/// A field with no numeric sibling assertion is presumed sized (string/slice/array/map), so
/// `not_empty` must keep using `len()` — the fix narrows only the proven-scalar case, it does
/// not stop measuring collections and strings.
#[test]
fn not_empty_on_a_sized_field_still_uses_len() {
    let out = render_indexed_assertion_function(vec![Assertion {
        assertion_type: "not_empty".to_string(),
        field: Some("results[0].content".to_string()),
        ..Default::default()
    }]);

    assert!(
        out.contains("\tif len(result.Results[0].Content) == 0 {\n\t\tt.Errorf(\"expected non-empty value\")\n\t}\n"),
        "not_empty on a field with no numeric sibling assertion must still use len(); got:\n{out}"
    );
}

/// Regression test for alef task #86: a `visitor` fixture whose options type resolves
/// from neither `[e2e.call]` nor any `[[crates.trait_bridges]]` entry used to emit a
/// `t.Skip("go: visitor fixture requires trait bridge options_type")` body. That reads
/// as an author-intended skip in `go test` output but is really a config failure, so the
/// emitted suite went green while exercising none of the visitor behavior it claimed.
/// It must now fail at generation time, naming the fixture and the missing options type
/// — mirroring `c/assertions.rs` and `kotlin/args.rs`, which already refuse to emit for
/// an unresolvable trait bridge.
#[test]
#[should_panic(expected = "Go e2e generator: fixture `visitor_smoke` declares a `visitor`")]
fn visitor_fixture_without_trait_bridge_options_type_fails_loudly_instead_of_emitting_a_skip() {
    use crate::e2e::fixture::{CallbackAction, VisitorSpec};

    let e2e_config = E2eConfig {
        call: CallConfig {
            function: "convert".to_string(),
            module: "github.com/example/mylib".to_string(),
            result_var: "result".to_string(),
            ..CallConfig::default()
        },
        ..E2eConfig::default()
    };

    let mut fixture = make_fixture("visitor_smoke");
    fixture.visitor = Some(VisitorSpec {
        callbacks: [("visit_element".to_string(), CallbackAction::Skip)]
            .into_iter()
            .collect(),
    });

    let mut out = String::new();
    // No `[[crates.trait_bridges]]` entries declared — nothing supplies an `options_type`.
    let config = crate::core::config::ResolvedCrateConfig::default();
    let type_defs: Vec<crate::core::ir::TypeDef> = Vec::new();
    let enums: Vec<crate::core::ir::EnumDef> = Vec::new();
    render_test_function(
        &mut out,
        &fixture,
        GoTestFunctionContext {
            import_alias: "sample_crate",
            e2e_config: &e2e_config,
            adapters: &[],
            data_enum_names: &std::collections::HashSet::new(),
            config: &config,
            type_defs: &type_defs,
            enums: &enums,
            errors: &[],
            functions: &[],
        },
    );
}

/// Go's `expects_error` branch renders the failure check plus (since the declared-value work) a
/// message-or-type-name comparison, then returns — every other assertion on the fixture used to
/// leave no trace at all in the generated test.
#[test]
fn go_equals_on_an_error_field_is_named_instead_of_dropped() {
    let mut fixture = make_fixture("rate_limited");
    fixture.assertions = vec![
        Assertion {
            assertion_type: "error".to_string(),
            value: Some(serde_json::Value::String("BadRequest".to_string())),
            ..Default::default()
        },
        Assertion {
            assertion_type: "equals".to_string(),
            field: Some("error.status_code".to_string()),
            ..Default::default()
        },
    ];
    let e2e_config = E2eConfig {
        call: CallConfig {
            function: "parse".to_string(),
            module: "example.com/sample".to_string(),
            returns_result: true,
            ..CallConfig::default()
        },
        ..E2eConfig::default()
    };
    let config = crate::core::config::ResolvedCrateConfig::default();
    let type_defs: Vec<crate::core::ir::TypeDef> = Vec::new();
    let enums: Vec<crate::core::ir::EnumDef> = Vec::new();

    let mut out = String::new();
    let _ = crate::e2e::codegen::take_skip_records();
    render_test_function(
        &mut out,
        &fixture,
        GoTestFunctionContext {
            import_alias: "sample_crate",
            e2e_config: &e2e_config,
            adapters: &[],
            data_enum_names: &std::collections::HashSet::new(),
            config: &config,
            type_defs: &type_defs,
            enums: &enums,
            errors: &[],
            functions: &[],
        },
    );

    // Positive first: the error block really did render, so the absence check below is not
    // vacuously satisfied by a backend that emitted nothing.
    assert!(
        out.contains("t.Errorf(\"expected an error, but call succeeded\")"),
        "the error block must render: {out}"
    );
    assert!(
        out.contains(
            "// skipped: assertion type 'equals' has no accessor for error field error.status_code in this backend"
        ),
        "{out}"
    );

    let records = crate::e2e::codegen::take_skip_records();
    assert_eq!(records.len(), 1, "got: {records:?}");
    assert_eq!(records[0].language, "go");
    assert_eq!(records[0].field, "equals");
}

/// Negative control: an error fixture with nothing beyond its one `error` assertion must render
/// no marker at all, so the gate stays informative.
#[test]
fn go_a_lone_error_assertion_renders_no_marker() {
    let mut fixture = make_fixture("rejects");
    fixture.assertions = vec![Assertion {
        assertion_type: "error".to_string(),
        ..Default::default()
    }];
    let e2e_config = E2eConfig {
        call: CallConfig {
            function: "parse".to_string(),
            module: "example.com/sample".to_string(),
            returns_result: true,
            ..CallConfig::default()
        },
        ..E2eConfig::default()
    };
    let config = crate::core::config::ResolvedCrateConfig::default();
    let type_defs: Vec<crate::core::ir::TypeDef> = Vec::new();
    let enums: Vec<crate::core::ir::EnumDef> = Vec::new();

    let mut out = String::new();
    render_test_function(
        &mut out,
        &fixture,
        GoTestFunctionContext {
            import_alias: "sample_crate",
            e2e_config: &e2e_config,
            adapters: &[],
            data_enum_names: &std::collections::HashSet::new(),
            config: &config,
            type_defs: &type_defs,
            enums: &enums,
            errors: &[],
            functions: &[],
        },
    );

    assert!(
        out.contains("t.Errorf(\"expected an error, but call succeeded\")"),
        "the error block must render: {out}"
    );
    assert!(!out.contains("has no accessor for error field"), "{out}");
}
