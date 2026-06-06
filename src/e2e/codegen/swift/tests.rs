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
