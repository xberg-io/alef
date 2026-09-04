//! Swift e2e codegen unit tests.

use super::accessors::{swift_build_accessor, swift_stringy_aggregator_contains_assert};
use super::assertions::render_assertion;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;
use std::collections::{HashMap, HashSet};

fn make_resolver_tool_calls() -> FieldResolver {
    // Resolver for `choices[0].message.tool_calls[0].function.name`:
    //   - `choices` is a registered array field
    //   - `choices.message.tool_calls` is optional (Optional<RustVec<ToolCall>>)
    let mut optional = HashSet::new();
    optional.insert("choices.message.tool_calls".to_string());
    let mut arrays = HashSet::new();
    arrays.insert("choices".to_string());
    FieldResolver::new(&HashMap::new(), &optional, &HashSet::new(), &arrays, &HashSet::new())
}

/// IR-oracle wiring regression (alef task #64): a field that is IR-reachable
/// (present, non-`binding_excluded`, on some IR type) but missing from the
/// hand-maintained `result_fields` config must still render a real assertion,
/// not a "skipped: field not available" comment — `swift/test_method.rs` now
/// threads `FieldResolver::ir_field_sets(type_defs)` into `with_ir_fields`. ~keep
#[test]
fn swift_ir_reachable_field_absent_from_result_fields_is_not_skipped() {
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
        &resolver,
        false,
        false,
        false,
        false,
        &HashMap::new(),
        false,
        false,
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
fn swift_ir_excluded_field_present_in_result_fields_is_still_skipped() {
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
        &resolver,
        false,
        false,
        false,
        false,
        &HashMap::new(),
        false,
        false,
    );
    assert!(out.contains("skipped"), "got: {out}");
}

#[test]
fn not_empty_is_type_aware_for_optional_values() {
    let cases = [
        ("quality_score", false, "result.qualityScore() != nil"),
        ("keywords", true, "result.keywords()?.isEmpty == false"),
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
            &resolver,
            false,
            false,
            false,
            false,
            &HashMap::new(),
            false,
            false,
        );
        assert!(out.contains(expected), "field {field}: {out}");
        assert!(!out.contains("toString"), "field {field}: {out}");
    }
}

fn not_error_assertion() -> Assertion {
    Assertion {
        skip: None,
        assertion_type: "not_error".to_string(),
        field: None,
        value: None,
        values: None,
        method: None,
        check: None,
        args: None,
        return_type: None,
    }
}

/// Regression test for the not_error vacuous-test defect: before an earlier fix, `not_error`
/// rendered nothing at all, and `test_method.rs` used a separate `has_not_error_assertion` flag
/// only to decide whether to bind `let result = ...` — never to assert on it. That fix over-shot
/// and replaced the silent no-op with a tautological `XCTAssertNotNil(result)`: the binding
/// declares `result` non-optional, so Swift promotes it to `Optional` at the call site and the
/// assertion can never fail regardless of what the call returned. `not_error`'s only real
/// contribution is the `try` propagation above it, which a comment now documents instead.
#[test]
fn not_error_on_a_non_void_result_emits_no_tautological_assertion() {
    let resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    );
    let assertion = not_error_assertion();
    let mut out = String::new();
    render_assertion(
        &mut out,
        &assertion,
        "result",
        &resolver,
        false,
        false,
        false,
        false,
        &HashMap::new(),
        false,
        false,
    );
    assert!(!out.contains("XCTAssertNotNil"), "got: {out}");
    assert_eq!(out, "        // not_error: covered by try propagation\n");
}

#[test]
fn not_error_on_a_streaming_fixture_emits_no_tautological_assertion() {
    let resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    );
    let assertion = not_error_assertion();
    let mut out = String::new();
    render_assertion(
        &mut out,
        &assertion,
        "result",
        &resolver,
        false,
        false,
        false,
        false,
        &HashMap::new(),
        true,
        false,
    );
    assert!(!out.contains("XCTAssertNotNil(chunks)"), "got: {out}");
}

/// A `returns_void` call binds no `result` at all — asserting on it would not
/// compile. The exception path already covers `not_error` there.
#[test]
fn not_error_on_a_returns_void_call_emits_nothing() {
    let resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    );
    let assertion = not_error_assertion();
    let mut out = String::new();
    render_assertion(
        &mut out,
        &assertion,
        "result",
        &resolver,
        false,
        false,
        false,
        false,
        &HashMap::new(),
        false,
        true,
    );
    assert!(
        out.is_empty(),
        "a returns_void call must not reference an unbound result, got: {out}"
    );
}

/// Regression: after the optional `[0]` subscript, the codegen must NOT
/// append a trailing `?`. The Swift compiler sees `?[0]` as consuming the
/// optional chain, yielding the non-optional element type, so a subsequent
/// `?.member` would trigger "cannot use optional chaining on non-optional
/// value".
///
/// With no `SwiftFirstClassMap` configured (default in this test), every
/// accessor is emitted as a swift-bridge method call, so accessors are
/// `result.choices()[0].message().toolCalls()?[0].function().name()`.
#[test]
fn optional_vec_subscript_does_not_emit_trailing_question_mark_before_next_segment() {
    let resolver = make_resolver_tool_calls();
    let (accessor, has_optional) =
        swift_build_accessor("choices[0].message.tool_calls[0].function.name", "result", &resolver);
    // `?` before `[0]` is correct (tool_calls is optional). Method-call
    // syntax (with `()`) is the default when no SwiftFirstClassMap is
    // supplied.
    assert!(
        accessor.contains("toolCalls()?[0]"),
        "expected `toolCalls()?[0]` for optional tool_calls, got: {accessor}"
    );
    // There must NOT be `?[0]?` (trailing `?` after the index).
    assert!(
        !accessor.contains("?[0]?"),
        "must not emit trailing `?` after subscript index: {accessor}"
    );
    // The expression IS optional overall (tool_calls may be nil).
    assert!(has_optional, "expected has_optional=true for optional field chain");
    // Subsequent member access uses `.` (non-optional chain) not `?.`.
    assert!(
        accessor.contains("[0].function"),
        "expected `.function` (non-optional) after subscript: {accessor}"
    );
}

/// `contains` against an array of opaque DTOs must aggregate every
/// text-bearing accessor of the element type and substring-match the
/// expected value, mirroring python's `_alef_e2e_item_texts`. This
/// avoids the brittle "primary accessor" guess (e.g. ImportInfo ->
/// source) that misses values surfaced through sibling fields like
/// `items` or `alias`.
#[test]
fn contains_against_vec_dto_aggregates_stringy_accessors() {
    use crate::e2e::field_access::{StringyField, StringyFieldKind, SwiftFirstClassMap};

    // Simulate the ImportInfo element type with its three text-bearing
    // accessors: source (plain), items (vec), alias (optional).
    let mut stringy_fields_by_type: HashMap<String, Vec<StringyField>> = HashMap::new();
    stringy_fields_by_type.insert(
        "ImportInfo".to_string(),
        vec![
            StringyField {
                name: "source".to_string(),
                kind: StringyFieldKind::Plain,
            },
            StringyField {
                name: "items".to_string(),
                kind: StringyFieldKind::Vec,
            },
            StringyField {
                name: "alias".to_string(),
                kind: StringyFieldKind::Optional,
            },
        ],
    );
    let mut field_types: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut process_fields = HashMap::new();
    process_fields.insert("imports".to_string(), "ImportInfo".to_string());
    field_types.insert("ProcessResult".to_string(), process_fields);

    let mut arrays = HashSet::new();
    arrays.insert("imports".to_string());

    let map = SwiftFirstClassMap {
        field_types,
        stringy_fields_by_type,
        ..SwiftFirstClassMap::default()
    };
    let resolver = FieldResolver::new_with_swift_first_class(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &arrays,
        &HashSet::new(),
        &HashMap::new(),
        map,
    )
    .with_swift_root_type(Some("ProcessResult".to_string()));

    let line = swift_stringy_aggregator_contains_assert(Some("imports"), "result", &resolver, "\"os\"")
        .expect("aggregator should fire for Vec<ImportInfo> contains");
    assert!(
        line.contains("result.imports().contains(where: { item in"),
        "expected contains(where:) over result.imports(): {line}"
    );
    assert!(
        line.contains("texts.append(item.source().toString())"),
        "expected plain source() accessor: {line}"
    );
    assert!(
        line.contains("texts.append(contentsOf: item.items().map { $0.as_str().toString() })"),
        "expected vec items() flattened via .map as_str(): {line}"
    );
    assert!(
        line.contains("if let v = item.alias()"),
        "expected optional alias() unwrap: {line}"
    );
    // Substring match, NOT exact equality.
    assert!(
        line.contains("$0.contains(\"os\")"),
        "expected substring contains over expected value: {line}"
    );
    assert!(!line.contains("$0 == \"os\""), "must not use exact equality: {line}");
}

/// When the element type has fewer than 2 stringy accessors, the
/// aggregator should bow out and let the simpler single-accessor path
/// emit code, keeping diff churn minimal on fixtures that already pass.
#[test]
fn contains_aggregator_skips_when_only_one_stringy_field() {
    use crate::e2e::field_access::{StringyField, StringyFieldKind, SwiftFirstClassMap};

    let mut stringy_fields_by_type: HashMap<String, Vec<StringyField>> = HashMap::new();
    stringy_fields_by_type.insert(
        "TagInfo".to_string(),
        vec![StringyField {
            name: "name".to_string(),
            kind: StringyFieldKind::Plain,
        }],
    );
    let mut field_types: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut root_fields = HashMap::new();
    root_fields.insert("tags".to_string(), "TagInfo".to_string());
    field_types.insert("Root".to_string(), root_fields);
    let mut arrays = HashSet::new();
    arrays.insert("tags".to_string());
    let map = SwiftFirstClassMap {
        field_types,
        stringy_fields_by_type,
        ..SwiftFirstClassMap::default()
    };
    let resolver = FieldResolver::new_with_swift_first_class(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &arrays,
        &HashSet::new(),
        &HashMap::new(),
        map,
    )
    .with_swift_root_type(Some("Root".to_string()));
    assert!(
        swift_stringy_aggregator_contains_assert(Some("tags"), "result", &resolver, "\"x\"").is_none(),
        "single-stringy-field types must not trigger the aggregator"
    );
}

/// Regression: when a chain has multiple optional fields, only the FIRST
/// optional should emit a `?`. Once we unwrap with one `?`, Swift treats
/// the result as concrete, so subsequent non-leaf optional fields must NOT
/// emit additional `?` operators.
///
/// Example: `summary()` returns `Optional<SummaryResult>`, then `strategy()`
/// on SummaryResult returns non-Optional RustString. The emitted accessor
/// should be `result.summary()?.strategy()` (NOT `summary()?.strategy()?`).
#[test]
fn chained_optional_only_emits_question_mark_on_first_optional() {
    let mut optional = HashSet::new();
    optional.insert("summary".to_string());
    let resolver = FieldResolver::new(
        &HashMap::new(),
        &optional,
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    );

    let (accessor, has_optional) = swift_build_accessor("summary.strategy", "result", &resolver);
    // `summary()` is optional, so `?` is correct.
    assert!(
        accessor.contains("summary()?"),
        "expected `summary()?` for optional summary field: {accessor}"
    );
    // `strategy()` comes after unwrapping, so it must NOT have `?`.
    assert!(
        !accessor.contains("strategy()?"),
        "must not emit `?` after already-unwrapped optional field: {accessor}"
    );
    // Verify the full accessor shape.
    assert_eq!(
        accessor, "result.summary()?.strategy()",
        "expected `result.summary()?.strategy()`, got: {accessor}"
    );
    // The expression IS optional overall.
    assert!(has_optional, "expected has_optional=true for chain with optional root");
}

/// Env var injection in setUp() produces sorted setenv() calls with proper string escaping.
#[test]
fn test_file_renders_env_vars_in_class_setup() {
    use crate::core::config::ResolvedCrateConfig;
    use crate::e2e::config::E2eConfig;

    let mut e2e_config = E2eConfig::default();
    e2e_config.env.insert("ZEBRA".to_string(), "z_value".to_string());
    e2e_config.env.insert("APPLE".to_string(), "a_value".to_string());
    e2e_config.env.insert("BANANA".to_string(), "b_value".to_string());

    let output = super::test_file::render_test_file(
        "smoke",
        &[],
        &e2e_config,
        "TestModule",
        "TestCase",
        "testFunction",
        "result",
        &[],
        false,
        None,
        &Default::default(),
        &ResolvedCrateConfig::default(),
        &[],
        false,
        &[],
        &[],
        &[],
    );

    // Verify env vars appear in sorted order: APPLE, BANANA, ZEBRA.
    assert!(output.contains("APPLE"), "expected APPLE env var in output");
    assert!(output.contains("BANANA"), "expected BANANA env var in output");
    assert!(output.contains("ZEBRA"), "expected ZEBRA env var in output");

    // Verify sorting: APPLE must come before BANANA, BANANA before ZEBRA.
    let apple_pos = output.find("APPLE").unwrap();
    let banana_pos = output.find("BANANA").unwrap();
    let zebra_pos = output.find("ZEBRA").unwrap();
    assert!(
        apple_pos < banana_pos && banana_pos < zebra_pos,
        "env vars must be sorted alphabetically, got positions APPLE={}, BANANA={}, ZEBRA={}",
        apple_pos,
        banana_pos,
        zebra_pos
    );

    // Verify setenv signature: should have setenv(key, val, 0) calls.
    assert!(
        output.contains("setenv(key, val, 0)"),
        "expected setenv(key, val, 0) calls in output"
    );
}

/// Empty env produces no env injection block.
#[test]
fn test_file_renders_no_env_block_when_env_empty() {
    use crate::core::config::ResolvedCrateConfig;
    use crate::e2e::config::E2eConfig;

    let e2e_config = E2eConfig::default();

    let output = super::test_file::render_test_file(
        "smoke",
        &[],
        &e2e_config,
        "TestModule",
        "TestCase",
        "testFunction",
        "result",
        &[],
        false,
        None,
        &Default::default(),
        &ResolvedCrateConfig::default(),
        &[],
        false,
        &[],
        &[],
        &[],
    );

    // No env vars means no setenv calls.
    assert!(
        !output.contains("setenv"),
        "empty env should not produce any setenv calls"
    );
}

/// An `error` assertion with a declared `value` must check both the caught
/// error's description and its dynamic type name, since fixture authors use
/// either convention (a message-only field name or a type-name prefix).
#[test]
fn test_file_error_assertion_with_declared_value_checks_message_and_type() {
    use crate::core::config::ResolvedCrateConfig;
    use crate::e2e::config::E2eConfig;
    use crate::e2e::fixture::{Assertion, Fixture};

    let mut e2e_config = E2eConfig::default();
    e2e_config.call.function = "parseThing".into();

    let mut fixture = Fixture {
        id: "invalid_thing".into(),
        description: "Invalid thing raises".into(),
        ..Fixture::default()
    };
    fixture.assertions.push(Assertion {
        assertion_type: "error".into(),
        value: Some(serde_json::json!("ThingNotFound")),
        ..Default::default()
    });

    let output = super::test_file::render_test_file(
        "smoke",
        &[&fixture],
        &e2e_config,
        "TestModule",
        "TestCase",
        "parseThing",
        "result",
        &[],
        false,
        None,
        &Default::default(),
        &ResolvedCrateConfig::default(),
        &[],
        false,
        &[],
        &[],
        &[],
    );

    assert!(
        output.contains("String(describing: error)"),
        "expected error description capture, got:\n{output}"
    );
    assert!(
        output.contains("String(describing: type(of: error))"),
        "expected error type-name capture, got:\n{output}"
    );
    assert!(
        output.contains(
            "XCTAssertTrue(_errorMessage.contains(\"ThingNotFound\") || _errorType.contains(\"ThingNotFound\")"
        ),
        "expected a disjunctive message-or-type check against the declared value, got:\n{output}"
    );
    // The success-path failure call is untouched by this feature.
    assert!(output.contains("XCTFail(\"expected to throw\")"));
}

/// With no declared `value` on the `error` assertion, output must be
/// byte-identical to the pre-existing "catch anything" behavior.
#[test]
fn test_file_error_assertion_without_declared_value_is_byte_identical() {
    use crate::core::config::ResolvedCrateConfig;
    use crate::e2e::config::E2eConfig;
    use crate::e2e::fixture::{Assertion, Fixture};

    let mut e2e_config = E2eConfig::default();
    e2e_config.call.function = "parseThing".into();

    let mut fixture = Fixture {
        id: "invalid_thing".into(),
        description: "Invalid thing raises".into(),
        ..Fixture::default()
    };
    fixture.assertions.push(Assertion {
        assertion_type: "error".into(),
        ..Default::default()
    });

    let output = super::test_file::render_test_file(
        "smoke",
        &[&fixture],
        &e2e_config,
        "TestModule",
        "TestCase",
        "parseThing",
        "result",
        &[],
        false,
        None,
        &Default::default(),
        &ResolvedCrateConfig::default(),
        &[],
        false,
        &[],
        &[],
        &[],
    );

    assert!(!output.contains("String(describing: error)"));
    assert!(output.contains("        } catch {\n            // success\n        }"));
}

/// Declared error values containing Swift string-interpolation and escape
/// characters (`"`, `\`, backslash-escapes) must be escaped via the shared
/// `escape_swift` helper, not hand-rolled, so the emitted literal stays valid.
#[test]
fn test_file_error_assertion_escapes_declared_value_for_swift_string_literal() {
    use crate::core::config::ResolvedCrateConfig;
    use crate::e2e::config::E2eConfig;
    use crate::e2e::fixture::{Assertion, Fixture};

    let mut e2e_config = E2eConfig::default();
    e2e_config.call.function = "parseThing".into();

    let mut fixture = Fixture {
        id: "invalid_thing".into(),
        description: "Invalid thing raises".into(),
        ..Fixture::default()
    };
    fixture.assertions.push(Assertion {
        assertion_type: "error".into(),
        value: Some(serde_json::json!("bad \"field\" \\ value")),
        ..Default::default()
    });

    let output = super::test_file::render_test_file(
        "smoke",
        &[&fixture],
        &e2e_config,
        "TestModule",
        "TestCase",
        "parseThing",
        "result",
        &[],
        false,
        None,
        &Default::default(),
        &ResolvedCrateConfig::default(),
        &[],
        false,
        &[],
        &[],
        &[],
    );

    let expected_escaped = crate::e2e::codegen::swift::values::escape_swift("bad \"field\" \\ value");
    let expected_snippet = format!("_errorMessage.contains(\"{expected_escaped}\")");
    assert!(
        output.contains(&expected_snippet),
        "expected escaped literal snippet `{expected_snippet}` in:\n{output}"
    );
}

/// Regression test: verify that app harness generates valid Swift multi-line
/// string literals. The bug was that template trim settings ate the newline
/// between `"""` and the first JSON chunk, producing invalid syntax like
/// `let _FIXTURES_JSON = """{...` instead of `let _FIXTURES_JSON = [...].joined()`.
///
/// The fix moves chunking to Rust and uses raw string literals that Swift
/// compiles directly without multiline-string issues.
#[test]
fn app_harness_renders_fixtures_json_chunks_without_multiline_string_syntax_error() {
    use crate::e2e::config::E2eConfig;
    use crate::e2e::fixture::FixtureGroup;

    // Test with an empty fixture group first to check basic structure.
    let group = FixtureGroup {
        category: "test".to_string(),
        fixtures: vec![],
    };

    let e2e_config = E2eConfig::default();
    let output = super::project::render_app_harness(&e2e_config, &[group], "TestModule");

    // Verify the output does NOT have the bug signature: `"""` followed immediately by `{`.
    assert!(
        !output.contains("\"\"\"{{"),
        "output must not have multiline string opening followed by JSON object on same line"
    );
    assert!(
        !output.contains("\"\"\" {"),
        "output must not have multiline string opening followed by space and JSON on same line"
    );

    // Verify the array-based approach is used.
    assert!(
        output.contains("let _FIXTURES_JSON: String = ["),
        "expected array literal pattern: let _FIXTURES_JSON: String = ["
    );

    // Verify `.joined()` is present (arrays are concatenated).
    assert!(
        output.contains("].joined()"),
        "expected .joined() call to concatenate chunks"
    );

    // Verify the output is not empty and contains valid Swift structure.
    assert!(!output.is_empty(), "rendered output should not be empty");
}

/// Regression test: when has_http_fixtures is false, the `let _existing` binding
/// should not be emitted to avoid "initialization of immutable value '_existing' was never used"
/// warning. The binding is only used inside the `if has_http_fixtures {` block.
#[test]
fn test_file_does_not_emit_existing_binding_when_no_http_fixtures() {
    use crate::core::config::ResolvedCrateConfig;
    use crate::e2e::config::E2eConfig;

    let e2e_config = E2eConfig::default();

    // Render with has_http_fixtures=false
    let output = super::test_file::render_test_file(
        "smoke",
        &[],
        &e2e_config,
        "TestModule",
        "TestCase",
        "testFunction",
        "result",
        &[],
        false,
        None,
        &Default::default(),
        &ResolvedCrateConfig::default(),
        &[],
        false, // has_http_fixtures = false
        &[],
        &[],
        &[],
    );

    // The `let _existing` binding should NOT be present when there are no HTTP fixtures.
    assert!(
        !output.contains("let _existing"),
        "should not emit `let _existing` binding when has_http_fixtures=false"
    );
}

/// When has_http_fixtures is true, the `let _existing` binding SHOULD be emitted
/// inside the harness setup block.
#[test]
fn test_file_emits_existing_binding_when_has_http_fixtures() {
    use crate::core::config::ResolvedCrateConfig;
    use crate::e2e::config::E2eConfig;

    let e2e_config = E2eConfig::default();

    // Render with has_http_fixtures=true
    let output = super::test_file::render_test_file(
        "smoke",
        &[],
        &e2e_config,
        "TestModule",
        "TestCase",
        "testFunction",
        "result",
        &[],
        false,
        None,
        &Default::default(),
        &ResolvedCrateConfig::default(),
        &[],
        true, // has_http_fixtures = true
        &[],
        &[],
        &[],
    );

    // The `let _existing` binding SHOULD be present when has_http_fixtures=true.
    assert!(
        output.contains("let _existing = ProcessInfo.processInfo.environment[\"SUT_URL\"]"),
        "should emit `let _existing` binding when has_http_fixtures=true"
    );

    // Verify it's inside the if block (appears before the nil check).
    assert!(
        output.contains("let _existing = ProcessInfo.processInfo.environment[\"SUT_URL\"]\n")
            || output.contains("let _existing = ProcessInfo.processInfo.environment[\"SUT_URL\"]\r\n"),
        "binding should be followed by the if nil check"
    );
}

/// The harness readiness probe must only report `ready` when the probe request
/// actually received an HTTP response — a connection error (e.g. "connection
/// refused" while the harness is still binding its listener) also completes the
/// data task, so treating "task completed" as "ready" reports the harness ready
/// before it can serve requests, masking boot failures as spurious nil-unwraps
/// downstream instead of a clear timeout error.
#[test]
fn test_file_readiness_probe_requires_actual_http_response() {
    use crate::core::config::ResolvedCrateConfig;
    use crate::e2e::config::E2eConfig;

    let e2e_config = E2eConfig::default();

    let output = super::test_file::render_test_file(
        "smoke",
        &[],
        &e2e_config,
        "TestModule",
        "TestCase",
        "testFunction",
        "result",
        &[],
        false,
        None,
        &Default::default(),
        &ResolvedCrateConfig::default(),
        &[],
        true, // has_http_fixtures = true
        &[],
        &[],
        &[],
    );

    // The completion handler must gate success on both a nil error and an actual
    // HTTPURLResponse, not merely on the data task completing.
    assert!(
        output.contains("error == nil, response is HTTPURLResponse"),
        "probe must require a real HTTP response before treating the harness as ready"
    );
    assert!(
        output.contains("_probeSucceeded"),
        "probe must track response success separately from task completion"
    );

    // The old dishonest form signaled `_probeSema` directly from a `{ _, _, _ in ... }`
    // closure that discarded the response/error, treating any completion as success.
    assert!(
        !output.contains("{ _, _, _ in _probeSema.signal() }"),
        "probe must not discard the response/error and treat any completion as ready"
    );

    // The timeout and boot-failure paths must still surface honestly.
    assert!(
        output.contains("Harness did not become ready within 15s"),
        "must still fatalError with a clear message when the harness never becomes ready"
    );
    assert!(
        output.contains("Failed to start harness"),
        "must still fatalError when the harness process fails to launch"
    );
}

/// Resolver whose only registered array field is `links`, mirroring the rust backend's
/// wildcard tests so the two suites pin the same fixture shape.
fn wildcard_resolver() -> FieldResolver {
    FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::from(["links".to_string()]),
        &HashSet::new(),
    )
}

fn render_field_assertion(
    resolver: &FieldResolver,
    assertion_type: &str,
    field: &str,
    value: Option<serde_json::Value>,
) -> String {
    let assertion = Assertion {
        skip: None,
        assertion_type: assertion_type.to_string(),
        field: Some(field.to_string()),
        value,
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
        resolver,
        false,
        false,
        false,
        false,
        &HashMap::new(),
        false,
        false,
    );
    out
}

/// Canary for the wildcard defect. `equals` had no `[].` branch, so `links[].url` fell through
/// to the generic accessor — which lowers `links[]` to `links[0]` — and emitted
/// `XCTAssertEqual(result.links()[0].url().toString(), "…")`. That is a *different* assertion
/// from the one the fixture wrote: it passes whenever element zero matches while claiming to
/// cover the array, so the suite was green on a property false for every later element. The
/// backend cannot traverse `equals`, so the only honest output is a visible skip. Against the
/// pre-fix generator the emitted text contains `[0]` and every assertion below fails. ~keep
#[test]
fn swift_wildcard_equals_leaves_a_visible_skip_instead_of_asserting_element_zero() {
    let out = render_field_assertion(
        &wildcard_resolver(),
        "equals",
        "links[].url",
        Some(serde_json::Value::String("https://example.com".to_string())),
    );
    assert_eq!(
        out.trim_end(),
        "        // skipped: unsupported traversal assertion 'equals' on 'links[].url'",
        "got: {out}"
    );
    assert!(!out.contains("[0]"), "wildcard must not pin element 0, got: {out}");
    assert!(
        !out.contains("XCTAssert"),
        "a refused traversal must emit no assertion at all, got: {out}"
    );
}

/// The same silent narrowing reached every other arm without a `[].` branch. Each must now
/// refuse visibly rather than assert against element zero. ~keep
#[test]
fn swift_wildcard_scalar_assertions_all_refuse_visibly() {
    let resolver = wildcard_resolver();
    let cases = [
        ("starts_with", Some(serde_json::json!("http"))),
        ("ends_with", Some(serde_json::json!(".com"))),
        ("matches_regex", Some(serde_json::json!("^http"))),
        ("min_length", Some(serde_json::json!(1))),
        ("max_length", Some(serde_json::json!(80))),
        ("greater_than", Some(serde_json::json!(1))),
        ("less_than", Some(serde_json::json!(9))),
        ("greater_than_or_equal", Some(serde_json::json!(1))),
        ("less_than_or_equal", Some(serde_json::json!(9))),
        ("count_min", Some(serde_json::json!(1))),
        ("count_equals", Some(serde_json::json!(2))),
        ("is_true", None),
        ("is_false", None),
        ("is_empty", None),
        ("contains_any", None),
    ];
    for (assertion_type, value) in cases {
        let out = render_field_assertion(&resolver, assertion_type, "links[].url", value);
        assert_eq!(
            out.trim_end(),
            format!("        // skipped: unsupported traversal assertion '{assertion_type}' on 'links[].url'"),
            "assertion type {assertion_type}: {out}"
        );
        assert!(
            !out.contains("[0]"),
            "assertion type {assertion_type} must not pin element 0, got: {out}"
        );
    }
}

/// `is_empty` was listed in the old `traversal_skips_field_expr` suppression set but had no
/// traversal branch to suppress for, so it dropped the `let _vec_… = result.links()` binding
/// while still emitting an expression that referenced that local — Swift naming an undeclared
/// variable. The refusal removes the expression, so no dangling local can survive. ~keep
#[test]
fn swift_wildcard_is_empty_emits_no_reference_to_a_dropped_vec_local() {
    let out = render_field_assertion(&wildcard_resolver(), "is_empty", "links[].url", None);
    assert!(!out.contains("_vec_"), "got: {out}");
}

/// The four arms that *can* traverse must keep doing so — the pre-dispatch moved their code,
/// it did not remove it. `contains` quantifies over the array and names no index. ~keep
#[test]
fn swift_wildcard_contains_still_quantifies_over_every_element() {
    let out = render_field_assertion(
        &wildcard_resolver(),
        "contains",
        "links[].url",
        Some(serde_json::Value::String("example".to_string())),
    );
    assert!(
        out.contains("XCTAssertTrue(result.links().contains(where: { $0.url().toString().contains(\"example\") })"),
        "got: {out}"
    );
    assert!(!out.contains("[0]"), "traversal must be index-free, got: {out}");
}

#[test]
fn swift_wildcard_not_contains_still_quantifies_over_every_element() {
    let out = render_field_assertion(
        &wildcard_resolver(),
        "not_contains",
        "links[].url",
        Some(serde_json::Value::String("example".to_string())),
    );
    assert!(
        out.contains("XCTAssertFalse(result.links().contains(where:"),
        "got: {out}"
    );
    assert!(!out.contains("[0]"), "traversal must be index-free, got: {out}");
}

#[test]
fn swift_wildcard_not_empty_still_quantifies_over_every_element() {
    let out = render_field_assertion(&wildcard_resolver(), "not_empty", "links[].url", None);
    assert!(
        out.contains("XCTAssertTrue(result.links().contains(where: { !$0.url().toString().isEmpty })"),
        "got: {out}"
    );
    assert!(!out.contains("[0]"), "traversal must be index-free, got: {out}");
}

/// The guard that the pre-dispatch does not over-capture: an explicitly indexed path is not a
/// wildcard, so it must still emit a real index-0 assertion rather than becoming a skip. This
/// is what stops the fix from silently deleting working coverage. ~keep
#[test]
fn swift_explicit_index_still_asserts_against_that_element() {
    let out = render_field_assertion(
        &wildcard_resolver(),
        "equals",
        "links[0].url",
        Some(serde_json::Value::String("https://example.com".to_string())),
    );
    assert!(out.contains("XCTAssertEqual("), "got: {out}");
    assert!(out.contains("[0].url().toString()"), "got: {out}");
    assert!(
        !out.contains("skipped"),
        "an explicit index is not a traversal, got: {out}"
    );
}

/// A non-wildcard field must be untouched by the pre-dispatch. ~keep
#[test]
fn swift_plain_field_equals_is_unaffected_by_the_wildcard_pre_dispatch() {
    let out = render_field_assertion(
        &wildcard_resolver(),
        "equals",
        "title",
        Some(serde_json::Value::String("hello".to_string())),
    );
    assert_eq!(
        out.trim_end(),
        "        XCTAssertEqual(result.title().toString(), \"hello\")"
    );
}

/// Swift's error path emits the do/catch and returns, so every other assertion on the fixture
/// used to leave no trace in the generated test at all.
#[test]
fn swift_equals_on_an_error_field_is_named_instead_of_dropped() {
    use crate::core::config::ResolvedCrateConfig;
    use crate::e2e::config::E2eConfig;
    use crate::e2e::fixture::{Assertion, Fixture};

    let mut e2e_config = E2eConfig::default();
    e2e_config.call.function = "parseThing".into();

    let mut fixture = Fixture {
        id: "rate_limited".into(),
        description: "Invalid thing raises".into(),
        ..Fixture::default()
    };
    fixture.assertions.push(Assertion {
        assertion_type: "error".into(),
        value: Some(serde_json::json!("ThingNotFound")),
        ..Default::default()
    });
    fixture.assertions.push(Assertion {
        assertion_type: "equals".into(),
        field: Some("error.status_code".into()),
        ..Default::default()
    });

    let _ = crate::e2e::codegen::take_skip_records();
    let output = super::test_file::render_test_file(
        "smoke",
        &[&fixture],
        &e2e_config,
        "TestModule",
        "TestCase",
        "parseThing",
        "result",
        &[],
        false,
        None,
        &Default::default(),
        &ResolvedCrateConfig::default(),
        &[],
        false,
        &[],
        &[],
        &[],
    );

    // Positive first: the error block really rendered.
    assert!(
        output.contains("XCTFail(\"expected to throw\")"),
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
    assert_eq!(records[0].language, "swift");
    assert_eq!(records[0].field, "equals");
}

/// Negative control: a lone `error` assertion must leave the generated file marker-free.
#[test]
fn swift_a_lone_error_assertion_renders_no_marker() {
    use crate::core::config::ResolvedCrateConfig;
    use crate::e2e::config::E2eConfig;
    use crate::e2e::fixture::{Assertion, Fixture};

    let mut e2e_config = E2eConfig::default();
    e2e_config.call.function = "parseThing".into();

    let mut fixture = Fixture {
        id: "invalid_thing".into(),
        description: "Invalid thing raises".into(),
        ..Fixture::default()
    };
    fixture.assertions.push(Assertion {
        assertion_type: "error".into(),
        ..Default::default()
    });

    let output = super::test_file::render_test_file(
        "smoke",
        &[&fixture],
        &e2e_config,
        "TestModule",
        "TestCase",
        "parseThing",
        "result",
        &[],
        false,
        None,
        &Default::default(),
        &ResolvedCrateConfig::default(),
        &[],
        false,
        &[],
        &[],
        &[],
    );

    assert!(
        output.contains("XCTFail(\"expected to throw\")"),
        "the error block must render: {output}"
    );
    assert!(!output.contains("has no accessor for error field"), "{output}");
}
