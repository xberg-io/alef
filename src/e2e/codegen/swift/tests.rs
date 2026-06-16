//! Swift e2e codegen unit tests.

use super::accessors::{swift_build_accessor, swift_stringy_aggregator_contains_assert};
use crate::e2e::field_access::FieldResolver;
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
        first_class_types: HashSet::new(),
        field_types,
        vec_field_names: HashSet::new(),
        root_type: None,
        stringy_fields_by_type,
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
        first_class_types: HashSet::new(),
        field_types,
        vec_field_names: HashSet::new(),
        root_type: None,
        stringy_fields_by_type,
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
    );

    // No env vars means no setenv calls.
    assert!(
        !output.contains("setenv"),
        "empty env should not produce any setenv calls"
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
