//! Regression coverage for Swift assertions that step past a JSON-bridged leaf.
//!
//! swift-bridge collapses a JSON-bridged field to a single `RustString` holding the whole field
//! as JSON, so the Swift leaf has neither `.count` nor a subscript. The generator's guard used to
//! be keyed on the trailing accessor's spelling — a `.length`/`.count`/`.size` suffix — so it
//! refused the count and happily emitted an indexed accessor against the very same leaf. The
//! generator therefore wrote the correct "JSON-bridges it to RustString" skip comment for one
//! assertion and a broken assertion for the other, on adjacent lines.
//!
//! These tests drive the real entry point, `render_test_method`, against an IR whose getter
//! shapes are the ones the binding generator actually emits, so the classification has to come
//! from the same predicate the binding uses rather than from a re-derivation.

use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{FieldDef, FunctionDef, PrimitiveType, TypeDef, TypeRef};
use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::fixture::{Assertion, Fixture};

fn field(name: &str, ty: TypeRef, optional: bool) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        optional,
        ..FieldDef::default()
    }
}

/// IR mirroring the real extraction shape: `Option<Vec<T>>` arrives as `ty: Vec<T>` with
/// `optional: true`, because field extraction strips the outer `Option` into the flag.
fn metadata_ir() -> (Vec<TypeDef>, Vec<FunctionDef>) {
    let type_defs = vec![
        TypeDef {
            name: "HeadingInfo".to_string(),
            fields: vec![
                field("level", TypeRef::Primitive(PrimitiveType::U32), false),
                field("text", TypeRef::String, false),
            ],
            has_serde: true,
            ..TypeDef::default()
        },
        TypeDef {
            name: "PageMetadata".to_string(),
            fields: vec![
                field("title", TypeRef::String, false),
                // `Option<Vec<HeadingInfo>>` → `fn headings(&self) -> String`
                field(
                    "headings",
                    TypeRef::Vec(Box::new(TypeRef::Named("HeadingInfo".to_string()))),
                    true,
                ),
                // `Option<Vec<String>>` → `fn og_locale_alternates(&self) -> String`
                field("og_locale_alternates", TypeRef::Vec(Box::new(TypeRef::String)), true),
                // `Vec<HeadingInfo>` → `fn favicons(&self) -> Vec<String>`, genuinely countable
                field(
                    "favicons",
                    TypeRef::Vec(Box::new(TypeRef::Named("HeadingInfo".to_string()))),
                    false,
                ),
            ],
            has_serde: true,
            ..TypeDef::default()
        },
        TypeDef {
            name: "ArticleInfo".to_string(),
            fields: vec![
                // `Option<String>` → `fn published_time(&self) -> Option<String>`
                field("published_time", TypeRef::String, true),
                field("byline", TypeRef::String, false),
            ],
            // Opaque: fields are swift-bridge method calls, so the leaf is `Optional<RustString>`
            // rather than a first-class Swift `String?` property.
            is_opaque: true,
            ..TypeDef::default()
        },
        TypeDef {
            name: "ProcessResult".to_string(),
            fields: vec![
                field("metadata", TypeRef::Named("PageMetadata".to_string()), false),
                // Optional ancestor: the accessor chain gets a `?` before the leaf.
                field("article", TypeRef::Named("ArticleInfo".to_string()), true),
            ],
            has_serde: true,
            ..TypeDef::default()
        },
    ];
    let functions = vec![FunctionDef {
        name: "process".to_string(),
        return_type: TypeRef::Named("ProcessResult".to_string()),
        ..FunctionDef::default()
    }];
    (type_defs, functions)
}

fn render_assertion_on(assertion: Assertion) -> String {
    render_assertion_with_result_fields(assertion, &[])
}

fn render_assertion_with_result_fields(assertion: Assertion, result_fields: &[&str]) -> String {
    let (type_defs, functions) = metadata_ir();
    let call_config = CallConfig {
        function: "process".to_string(),
        result_var: "result".to_string(),
        ..CallConfig::default()
    };
    let mut e2e_config = E2eConfig::default();
    e2e_config.calls.insert("process".to_string(), call_config.clone());
    e2e_config
        .result_fields
        .extend(result_fields.iter().map(|f| (*f).to_string()));
    let fixture = Fixture {
        id: "json_bridged_traversal".to_string(),
        description: "JSON-bridged traversal".to_string(),
        call: Some("process".to_string()),
        assertions: vec![assertion],
        ..Fixture::default()
    };
    let map = super::values::build_swift_first_class_map(&type_defs, &[], &e2e_config, &call_config);
    let config = ResolvedCrateConfig {
        name: "sample".into(),
        ..ResolvedCrateConfig::default()
    };
    let mut out = String::new();
    super::test_method::render_test_method(
        &mut out,
        &fixture,
        &e2e_config,
        "process",
        "result",
        &[],
        false,
        None,
        &map,
        "Sample",
        &config,
        &type_defs,
        &[],
        &functions,
        &[],
    );
    out
}

fn assertion(assertion_type: &str, field_path: &str, value: Option<serde_json::Value>) -> Assertion {
    Assertion {
        assertion_type: assertion_type.to_string(),
        field: Some(field_path.to_string()),
        value,
        ..Assertion::default()
    }
}

const JSON_BRIDGE_SKIP: &str = "swift-bridge JSON-bridges it to RustString";

/// The case the count-suffix guard missed: indexing into a leaf the binding collapsed to one
/// `RustString`. There is no `[0]` subscript on a `RustString`, but `JSONSerialization` can decode
/// the bridged JSON text and index into THAT — `json_bridged_navigation` now does exactly this,
/// so the assertion renders a real check instead of a skip.
#[test]
fn should_decode_and_index_an_indexed_step_into_a_json_bridged_leaf() {
    let out = render_assertion_on(assertion(
        "equals",
        "metadata.headings[0].level",
        Some(serde_json::json!(1)),
    ));
    assert!(
        !out.contains(JSON_BRIDGE_SKIP),
        "an indexed step past a JSON-bridged leaf is now navigable, got:\n{out}"
    );
    assert!(
        out.contains("JSONSerialization.jsonObject"),
        "must decode the bridged leaf's JSON text rather than subscript the RustString itself, got:\n{out}"
    );
    assert!(
        !out.contains("headings()[0]") && !out.contains("headings()?[0]"),
        "must not emit a subscript directly against a RustString leaf, got:\n{out}"
    );
}

/// The array-wildcard spelling of the same impossibility. The wildcard pre-dispatch refuses
/// these paths for its own reason, so this asserts the JSON-bridge reason specifically —
/// otherwise the test would pass on the unrelated wildcard skip and prove nothing.
#[test]
fn should_refuse_a_wildcard_step_into_a_json_bridged_leaf() {
    let out = render_assertion_on(assertion(
        "contains",
        "metadata.headings[].text",
        Some(serde_json::json!("Intro")),
    ));
    assert!(
        out.contains(JSON_BRIDGE_SKIP),
        "a wildcard over a JSON-bridged leaf must be refused for the JSON-bridge reason, got:\n{out}"
    );
    assert!(
        !out.contains("headings()[0]") && !out.contains("headings()?[0]"),
        "a wildcard over a JSON-bridged leaf must not emit an index-0 accessor, got:\n{out}"
    );
}

/// `Option<Vec<String>>` really emits `fn og_locale_alternates(&self) -> String`, so a count
/// suffix on it has no countable leaf. The oracle used to call every optional `Vec` whose element
/// was not a `Named` type countable, which emitted `?.count` against a `RustString`.
#[test]
fn should_refuse_a_count_suffix_on_an_optional_vec_of_string() {
    let out = render_assertion_on(assertion(
        "equals",
        "metadata.og_locale_alternates.length",
        Some(serde_json::json!(2)),
    ));
    assert!(
        out.contains(JSON_BRIDGE_SKIP),
        "a count on Option<Vec<String>> must render the JSON-bridge skip, got:\n{out}"
    );
    assert!(
        !out.contains("ogLocaleAlternates()?.count") && !out.contains("ogLocaleAlternates().count"),
        "must not emit .count against a RustString leaf, got:\n{out}"
    );
}

/// The guard must be keyed on the leaf, not on the path's depth: a genuinely countable
/// `Vec<Named>` getter (`fn favicons(&self) -> Vec<String>`) still gets its assertion.
#[test]
fn should_still_count_a_non_optional_vec_whose_getter_is_a_real_rust_vec() {
    let out = render_assertion_on(assertion(
        "equals",
        "metadata.favicons.length",
        Some(serde_json::json!(3)),
    ));
    assert!(
        !out.contains(JSON_BRIDGE_SKIP),
        "a real RustVec getter must not be refused as JSON-bridged, got:\n{out}"
    );
    assert!(
        out.contains("favicons()") && out.contains("count"),
        "a countable leaf must still render its count assertion, got:\n{out}"
    );
}

/// Reading the bridged leaf itself is legal — it is a readable `RustString`. Only stepping
/// *past* it is impossible, so an assertion that stops at the leaf must survive.
#[test]
fn should_keep_an_assertion_that_stops_at_the_json_bridged_leaf() {
    let out = render_assertion_on(assertion(
        "contains",
        "metadata.headings",
        Some(serde_json::json!("Intro")),
    ));
    assert!(
        !out.contains(JSON_BRIDGE_SKIP),
        "an assertion that does not step past the bridged leaf must not be refused, got:\n{out}"
    );
    assert!(
        out.contains("headings()"),
        "the bridged leaf itself must still be read, got:\n{out}"
    );
}

/// An optional leaf reached through an optional ancestor needs a `?` at BOTH steps. The chain's
/// own `?` unwraps the ancestor only; the leaf getter still returns `Optional<RustString>`, so
/// `.toString()` applied directly to it does not compile.
#[test]
fn should_chain_optionally_at_an_optional_leaf_behind_an_optional_ancestor() {
    let out = render_assertion_on(assertion(
        "equals",
        "article.published_time",
        Some(serde_json::json!("2024-01-01")),
    ));
    assert!(
        out.contains("article()?.publishedTime()?.toString()"),
        "an optional leaf must be unwrapped before toString(), got:\n{out}"
    );
    assert!(
        !out.contains("publishedTime().toString()"),
        "must not apply toString() directly to an Optional<RustString> leaf, got:\n{out}"
    );
}

/// The sibling that must NOT gain a `?`: a non-optional leaf behind the same optional ancestor.
/// Swift rejects `?.` on a non-optional value, so over-applying the fix is its own compile error.
#[test]
fn should_not_chain_optionally_at_a_non_optional_leaf_behind_an_optional_ancestor() {
    let out = render_assertion_on(assertion("equals", "article.byline", Some(serde_json::json!("Ada"))));
    assert!(
        out.contains("article()?.byline().toString()"),
        "a non-optional leaf must keep a plain toString(), got:\n{out}"
    );
    assert!(
        !out.contains("byline()?.toString()"),
        "must not apply optional chaining to a non-optional RustString leaf, got:\n{out}"
    );
}

/// A `result_fields` list naming a nested leaf but not its parent used to make the parent look
/// like a virtual namespace prefix, so the parent segment was dropped and the accessor was built
/// on the wrong receiver — `result.favicons()` against a result type that has no such field. The
/// IR declares `metadata` as a real struct field, which settles it whatever the config omits.
#[test]
fn should_not_strip_a_real_struct_segment_that_result_fields_omits() {
    let out = render_assertion_with_result_fields(
        assertion("equals", "metadata.favicons.length", Some(serde_json::json!(3))),
        &["favicons", "title"],
    );
    assert!(
        out.contains("metadata()") || out.contains("metadata."),
        "the real `metadata` segment must survive, got:\n{out}"
    );
    assert!(
        !out.contains("result.favicons()"),
        "must not build the accessor on the result when `metadata` is a real field, got:\n{out}"
    );
}

/// The companion: a segment the IR does NOT declare on the result type is a genuine virtual
/// namespace prefix and must still be stripped, or every namespaced fixture path breaks.
#[test]
fn should_still_strip_a_virtual_namespace_segment_the_ir_does_not_declare() {
    let out = render_assertion_with_result_fields(
        assertion("equals", "browser.title", Some(serde_json::json!("Hello"))),
        &["title"],
    );
    assert!(
        out.contains("result.title()"),
        "a virtual namespace prefix must still be stripped down to the real field, got:\n{out}"
    );
    assert!(
        !out.contains("browser()"),
        "a virtual namespace prefix must still be stripped, got:\n{out}"
    );
}

/// A plain scalar sibling proves the guard is not blanket-skipping every path on the type.
#[test]
fn should_keep_an_assertion_on_a_plain_string_field() {
    let out = render_assertion_on(assertion("equals", "metadata.title", Some(serde_json::json!("Hello"))));
    assert!(
        !out.contains(JSON_BRIDGE_SKIP),
        "a plain String field must not be refused as JSON-bridged, got:\n{out}"
    );
    assert!(
        out.contains("title()"),
        "a plain String field must still render its assertion, got:\n{out}"
    );
}
