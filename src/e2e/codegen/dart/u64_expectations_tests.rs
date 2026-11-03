use super::DartE2eCodegen;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{ApiSurface, PrimitiveType, TypeRef};
use crate::e2e::codegen::E2eCodegen;
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::{Assertion, Fixture, FixtureGroup};

fn api() -> ApiSurface {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("lib.rs");
    std::fs::write(&source, include_str!("fixtures/u64_expectations.rs")).unwrap();
    let api = crate::extract::extractor::extract(&[&source], "wide-expectations", "1.0.0", None).unwrap();
    assert_eq!(api.functions.len(), 4);
    assert_eq!(api.types.len(), 5);
    assert_eq!(api.enums.len(), 3);
    let total = api.functions.iter().find(|function| function.name == "total").unwrap();
    assert_eq!(total.return_type, TypeRef::Primitive(PrimitiveType::U64));
    api
}

fn emitted(function: &str, simple: bool, assertion: Assertion) -> String {
    let api = api();
    let fixture = Fixture {
        id: "wide_expectation".into(),
        description: "Declared numeric expectation".into(),
        assertions: vec![assertion],
        ..Default::default()
    };
    let mut config = E2eConfig::default();
    config.call.function = function.into();
    config.call.result_is_simple = simple;
    let files = DartE2eCodegen
        .generate(
            &[FixtureGroup {
                category: "wide_expectations".into(),
                fixtures: vec![fixture],
            }],
            &config,
            &ResolvedCrateConfig {
                name: "wide-expectations".into(),
                ..Default::default()
            },
            &api.types,
            &api.enums,
            &api.functions,
            &api.errors,
        )
        .unwrap();
    let selected: Vec<_> = files
        .iter()
        .filter(|file| file.path.ends_with("test/wide_expectations_test.dart"))
        .collect();
    assert_eq!(selected.len(), 1, "full generator must emit exactly one test artifact");
    let source = &selected[0].content;
    assert!(source.contains(&format!("{function}(")), "missing real call: {source}");
    source.clone()
}

fn assertion(kind: &str, field: Option<&str>, value: serde_json::Value) -> Assertion {
    Assertion {
        assertion_type: kind.into(),
        field: field.map(str::to_owned),
        value: Some(value),
        ..Default::default()
    }
}

fn expect_line(source: &str, expected: &str) {
    expect_lines(source, &[expected]);
}

fn expect_lines(source: &str, expected: &[&str]) {
    let lines: Vec<_> = source
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("expect("))
        .collect();
    assert_eq!(lines, expected, "generated assertions: {source}");
}

#[test]
fn should_compare_tagged_union_payload_u64_values_as_bigints() {
    for (field, expression) in [
        ("format.stats.count", "(result.format as Format_Stats).field0.count"),
        ("format.stats.length", "(result.format as Format_Stats).field0.length"),
        ("format.stats.size", "(result.format as Format_Stats).field0.size"),
        (
            "format.stats.optional_count",
            "(result.format as Format_Stats).field0.optionalCount",
        ),
    ] {
        for (kind, matcher) in [("equals", "equals"), ("greater_than_or_equal", "greaterThanOrEqualTo")] {
            let source = emitted("stats", false, assertion(kind, Some(field), u64::MAX.into()));
            expect_line(
                &source,
                &format!("expect({expression}, {matcher}(BigInt.parse('18446744073709551615')));"),
            );
        }
    }
}

#[test]
fn should_compare_whole_u64_results_as_bigints_with_or_without_simple_override() {
    for simple in [false, true] {
        for field in [None, Some("")] {
            let source = emitted("total", simple, assertion("equals", field, u64::MAX.into()));
            expect_line(&source, "expect(result, equals(BigInt.parse('18446744073709551615')));");
        }
    }
}

#[test]
fn should_preserve_plain_int_expectations_for_usize_and_signed_results() {
    for (function, value) in [("word", 23), ("signed", -1)] {
        let source = emitted(function, true, assertion("equals", None, value.into()));
        expect_line(&source, &format!("expect(result, equals({value}));"));
        assert!(!source.contains("BigInt."), "{source}");
    }
}

#[test]
fn should_preserve_plain_int_payload_expectations_and_collection_counts() {
    for (field, kind, expression, matcher, value) in [
        ("format.stats.word", "equals", "word", "equals", 23),
        ("format.stats.signed", "equals", "signed", "equals", -1),
        ("format.stats.counts", "count_equals", "counts.length", "equals", 2),
        ("format.stats.counts.length", "equals", "counts.length", "equals", 2),
    ] {
        let source = emitted("stats", false, assertion(kind, Some(field), value.into()));
        expect_line(
            &source,
            &format!("expect((result.format as Format_Stats).field0.{expression}, {matcher}({value}));"),
        );
        assert!(!source.contains("BigInt."), "{source}");
    }
}

#[test]
fn should_compare_tagged_union_u64_collection_members_as_bigints() {
    for kind in ["contains", "contains_all"] {
        let mut check = assertion(kind, Some("format.stats.counts"), u64::MAX.into());
        if kind == "contains_all" {
            check.value = None;
            check.values = Some(vec![u64::MAX.into()]);
        }
        let source = emitted("stats", false, check);
        expect_line(
            &source,
            "expect((result.format as Format_Stats).field0.counts, contains(BigInt.parse('18446744073709551615')));",
        );
    }
}

#[test]
fn should_preserve_null_optional_u64_payload_expectations() {
    let source = emitted(
        "stats",
        false,
        assertion("equals", Some("format.stats.optional_count"), serde_json::Value::Null),
    );
    expect_line(
        &source,
        "expect((result.format as Format_Stats).field0.optionalCount, equals(null));",
    );
}

#[test]
fn should_compare_indexed_variant_list_u64_fields_as_bigints() {
    for (variant, expression) in [
        (
            "Segments",
            "(result.entries[0].body.payload as Payload_Segments).field0[0].total",
        ),
        (
            "Kinds",
            "((result.entries[0].body.payload as Payload_Kinds).field0[0] as PieceKind_Text).total",
        ),
    ] {
        let field = format!("entries[0].body.payload.{}[0].total", variant.to_lowercase());
        let source = emitted("stats", false, assertion("equals", Some(&field), u64::MAX.into()));
        expect_lines(
            &source,
            &[
                &format!("expect(result.entries[0].body.payload, isA<Payload_{variant}>());"),
                &format!("expect({expression}, equals(BigInt.parse('18446744073709551615')));"),
            ],
        );
    }
}

#[test]
fn should_preserve_indexed_variant_list_int_fields_and_collection_counts() {
    for (field, kind, expression, value) in [
        ("entries[0].body.payload.segments[0].word", "equals", "[0].word", 23),
        ("entries[0].body.payload.segments", "count_equals", ".length", 2),
    ] {
        let source = emitted("stats", false, assertion(kind, Some(field), value.into()));
        expect_lines(
            &source,
            &[
                "expect(result.entries[0].body.payload, isA<Payload_Segments>());",
                &format!(
                    "expect((result.entries[0].body.payload as Payload_Segments).field0{expression}, equals({value}));"
                ),
            ],
        );
        assert!(!source.contains("BigInt."), "{source}");
    }
}
