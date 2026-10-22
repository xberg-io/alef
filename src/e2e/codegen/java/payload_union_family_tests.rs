//! Per-family coverage for the Java payload-union assertion gate (`payload_union_gate`).
//!
//! ~keep Every test drives the real entry point, `render_test_method`, against a single IR that
//! carries both shapes side by side: `StageOutput` is a `#[serde(untagged)]` union with a data
//! variant, which `backends::java::gen_bindings::emits_get_value` refuses `getValue()` and the
//! binding renders as a wrapper class; `StageStatus` is fieldless, which it renders as a plain
//! Java `enum`. Nothing here reaches into the gate's own helpers — the point is to prove the
//! wiring from IR, through `test_method.rs`'s `with_java_wrapper_enum_names`, to the emitted
//! line, not that a predicate returns what it returns.
//!
//! Every family is asserted from BOTH sides: a case that must register a skip, and a control on
//! a field of a different shape that must still emit a real assertion. A suite that only checked
//! the skip side would pass just as well if the gate refused everything, which is the failure
//! this file exists to make impossible.

use super::test_method::render_test_method;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, FunctionDef, PrimitiveType, TypeDef, TypeRef};
use crate::e2e::codegen::field_skip::FieldSkip;
use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::fixture::{Assertion, Fixture};

/// A `#[serde(untagged)]` union with a data-carrying variant: the shape the Java binding renders
/// as a wrapper class with no `getValue()`.
fn stage_output_enum() -> EnumDef {
    EnumDef {
        name: "StageOutput".to_string(),
        variants: vec![EnumVariant {
            name: "Text".to_string(),
            fields: vec![FieldDef {
                name: "0".to_string(),
                ty: TypeRef::String,
                ..FieldDef::default()
            }],
            is_tuple: true,
            ..EnumVariant::default()
        }],
        serde_untagged: true,
        ..EnumDef::default()
    }
}

/// A fieldless enum: the shape the Java binding renders as a plain `enum` with `getValue()`.
fn stage_status_enum() -> EnumDef {
    EnumDef {
        name: "StageStatus".to_string(),
        variants: vec![EnumVariant {
            name: "Queued".to_string(),
            ..EnumVariant::default()
        }],
        ..EnumDef::default()
    }
}

fn field(name: &str, ty: TypeRef, optional: bool) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        optional,
        ..FieldDef::default()
    }
}

/// One result type carrying every shape the gate distinguishes:
///
/// - `summary` — an OPTIONAL payload union, whose accessor is wrapped in `Optional.ofNullable`.
/// - `payload` — a NON-OPTIONAL payload union, whose accessor stays bare.
/// - `status` — a fieldless enum, the control that keeps `.getValue()`.
/// - `stages` — a collection of structs each carrying a payload union, for the wildcard paths.
/// - `title` / `attempts` / `tags` / `flag` — plain scalars and a collection, the per-family
///   controls that must keep emitting their normal assertion.
///
/// ~keep The numeric control field is `attempts`, deliberately NOT `count`. `parse_path` lowers
/// a segment literally named `length`, `count`, or `size` to a `PathSegment::Length`
/// pseudo-segment, so a field called `count` never addresses the IR field at all — `render_java`
/// emits `result.size()` for it. A control built on that name exercises the length pseudo-path
/// instead of the numeric one, which is a test of the fixture rather than of the generator.
fn union_ir() -> (Vec<TypeDef>, Vec<EnumDef>, Vec<FunctionDef>) {
    let type_defs = vec![
        TypeDef {
            name: "UnionResult".to_string(),
            fields: vec![
                field(
                    "summary",
                    TypeRef::Optional(Box::new(TypeRef::Named("StageOutput".to_string()))),
                    true,
                ),
                field("payload", TypeRef::Named("StageOutput".to_string()), false),
                field("status", TypeRef::Named("StageStatus".to_string()), false),
                field(
                    "stages",
                    TypeRef::Vec(Box::new(TypeRef::Named("Stage".to_string()))),
                    false,
                ),
                field("title", TypeRef::String, false),
                field("attempts", TypeRef::Primitive(PrimitiveType::U32), false),
                field("tags", TypeRef::Vec(Box::new(TypeRef::String)), false),
                field("flag", TypeRef::Primitive(PrimitiveType::Bool), false),
            ],
            ..TypeDef::default()
        },
        TypeDef {
            name: "Stage".to_string(),
            fields: vec![
                field("payload", TypeRef::Named("StageOutput".to_string()), false),
                field("label", TypeRef::String, false),
            ],
            ..TypeDef::default()
        },
    ];
    let enums = vec![stage_output_enum(), stage_status_enum()];
    let functions = vec![FunctionDef {
        name: "read_union".to_string(),
        return_type: TypeRef::Named("UnionResult".to_string()),
        ..FunctionDef::default()
    }];
    (type_defs, enums, functions)
}

fn fixture(id: &str, assertion: Assertion) -> Fixture {
    Fixture {
        docs: None,
        requirements: Vec::new(),
        id: id.to_string(),
        category: None,
        description: "test".to_string(),
        tags: vec![],
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
        client_options: None,
        preserve_input_urls: false,
        assertions: vec![assertion],
        visitor: None,
        args: vec![],
        assertion_recipes: vec![],
    }
}

fn assertion(assertion_type: &str, field_path: &str, value: Option<serde_json::Value>) -> Assertion {
    Assertion {
        assertion_type: assertion_type.to_string(),
        field: Some(field_path.to_string()),
        value,
        ..Assertion::default()
    }
}

fn text(value: &str) -> Option<serde_json::Value> {
    Some(serde_json::Value::String(value.to_string()))
}

fn number(value: u64) -> Option<serde_json::Value> {
    Some(serde_json::Value::Number(value.into()))
}

/// Render one assertion through the real `render_test_method` entry point.
fn render(assertion: Assertion, fields_display_as_text: &[&str]) -> String {
    let (type_defs, enums, functions) = union_ir();
    let e2e_config = E2eConfig {
        call: CallConfig {
            function: "read_union".to_string(),
            result_var: "result".to_string(),
            ..CallConfig::default()
        },
        fields_display_as_text: fields_display_as_text.iter().map(|s| s.to_string()).collect(),
        ..Default::default()
    };
    let mut out = String::new();
    render_test_method(
        &mut out,
        &fixture("union_family", assertion),
        "SampleClass",
        "",
        "",
        &[],
        None,
        false,
        &e2e_config,
        &std::collections::HashMap::new(),
        false,
        &[],
        &ResolvedCrateConfig::default(),
        &type_defs,
        &enums,
        &functions,
        &[],
    );
    out
}

/// Assert `rendered` carries a registered payload-union skip for `field_path`, and that the
/// unsupported lowering `fragment` names is nowhere in it.
///
/// ~keep The `extract_classified` round-trip is the load-bearing half: a plain `//` comment that
/// merely reads like a skip is invisible to the strict-gate marker scan, so a helper emitting an
/// unregistered wording would still look right in a generated file while counting as nothing.
fn assert_skipped(rendered: &str, field_path: &str, fragment: &str) {
    let line = rendered
        .lines()
        .find(|line| line.contains("skipped:"))
        .unwrap_or_else(|| panic!("expected a skip line for '{field_path}', got:\n{rendered}"));
    assert_eq!(
        FieldSkip::extract_classified(line),
        Some((field_path, FieldSkip::PayloadUnionHasNoScalarWireAccessor)),
        "the skip must be registered, not just commented; got: {line}"
    );
    assert!(
        !rendered.contains(fragment),
        "'{fragment}' must not be emitted for a payload-union leaf, got:\n{rendered}"
    );
}

/// Assert `rendered` emits `fragment` and registers no skip at all.
fn assert_emitted(rendered: &str, fragment: &str) {
    assert!(
        rendered.contains(fragment),
        "expected '{fragment}' to still be emitted, got:\n{rendered}"
    );
    assert!(
        !rendered.contains("payload-carrying union"),
        "this shape must not be refused as a payload union, got:\n{rendered}"
    );
}

#[test]
fn regex_on_a_payload_union_is_skipped() {
    let out = render(assertion("matches_regex", "payload", text("^ok.*$")), &[]);
    assert_skipped(&out, "payload", "result.payload().matches(");
}

#[test]
fn regex_on_an_optional_payload_union_is_skipped() {
    let out = render(assertion("matches_regex", "summary", text("^ok.*$")), &[]);
    assert_skipped(&out, "summary", ".matches(");
}

/// Opposite control: the same family on a plain `String` leaf must still emit.
#[test]
fn regex_on_a_string_field_still_emits() {
    let out = render(assertion("matches_regex", "title", text("^ok.*$")), &[]);
    assert_emitted(&out, ".matches(");
}

#[test]
fn length_on_a_payload_union_is_skipped() {
    let out = render(assertion("min_length", "payload", number(3)), &[]);
    assert_skipped(&out, "payload", "result.payload().length()");
}

/// Opposite control for the length half of the family.
#[test]
fn length_on_a_string_field_still_emits() {
    let out = render(assertion("min_length", "title", number(3)), &[]);
    assert_emitted(&out, ".length() >= 3");
}

#[test]
fn count_on_an_optional_payload_union_is_skipped() {
    let out = render(assertion("count_min", "summary", number(1)), &[]);
    assert_skipped(&out, "summary", ".size()");
}

/// Opposite control for the count half of the family.
#[test]
fn count_on_a_collection_field_still_emits() {
    let out = render(assertion("count_min", "tags", number(1)), &[]);
    assert_emitted(&out, ".size() >= 1");
}

#[test]
fn numeric_comparison_on_a_payload_union_is_skipped() {
    let out = render(assertion("greater_than", "payload", number(1)), &[]);
    assert_skipped(&out, "payload", "result.payload() > 1");
}

/// Opposite control: the same family on a numeric leaf must still emit.
#[test]
fn numeric_comparison_on_a_numeric_field_still_emits() {
    let out = render(assertion("greater_than", "attempts", number(1)), &[]);
    assert_emitted(&out, "result.attempts() > 1");
}

/// `equals` compiles on a wrapper instance through `assertEquals(Object, Object)` and is false
/// for every fixture that runs — the reason this family is refused rather than left alone.
#[test]
fn equality_on_a_payload_union_is_skipped() {
    let out = render(assertion("equals", "payload", text("ok")), &[]);
    assert_skipped(&out, "payload", "assertEquals");
}

#[test]
fn equality_on_an_optional_payload_union_is_skipped() {
    let out = render(assertion("equals", "summary", text("ok")), &[]);
    assert_skipped(&out, "summary", "assertEquals");
}

/// Opposite control: a fieldless enum keeps `getValue()`, so its equality assertion is real.
#[test]
fn equality_on_a_fieldless_enum_field_still_emits() {
    let out = render(assertion("equals", "status", text("Queued")), &[]);
    assert_emitted(&out, "result.status().getValue()");
}

#[test]
fn string_containment_on_an_optional_payload_union_is_skipped() {
    let out = render(assertion("contains", "summary", text("ok")), &[]);
    assert_skipped(&out, "summary", ".contains(");
}

/// Opposite control for the string half of the family.
#[test]
fn string_containment_on_a_string_field_still_emits() {
    let out = render(assertion("contains", "title", text("ok")), &[]);
    assert_emitted(&out, ".contains(");
}

/// A non-optional payload union has no `Optional` to switch on, so `is_true` renders
/// `assertTrue(wrapperInstance, ...)` — the invalid-boolean family.
#[test]
fn boolean_on_a_non_optional_payload_union_is_skipped() {
    let out = render(assertion("is_true", "payload", None), &[]);
    assert_skipped(&out, "payload", "assertTrue(result.payload()");
}

/// Opposite control, and the substantiated half of the same family: on an OPTIONAL leaf,
/// `is_true` means "present" and lowers to `Optional.isPresent()`, which is real Java for any
/// `T`. This is the case that proves the gate discriminates on shape rather than on family.
#[test]
fn boolean_on_an_optional_payload_union_still_emits_a_presence_check() {
    let out = render(assertion("is_true", "summary", None), &[]);
    assert_emitted(&out, "java.util.Optional.ofNullable(result.summary()).isPresent()");
}

/// Opposite control on a genuinely boolean leaf.
#[test]
fn boolean_on_a_bool_field_still_emits() {
    let out = render(assertion("is_true", "flag", None), &[]);
    assert_emitted(&out, "assertTrue(result.flag()");
}

/// Retained presence: the optional lowering substantiates it.
#[test]
fn presence_on_an_optional_payload_union_still_emits() {
    let out = render(assertion("not_empty", "summary", None), &[]);
    assert_emitted(&out, "java.util.Optional.ofNullable(result.summary())");
}

/// Unsubstantiated presence: a non-optional union leaf is never `field_is_object` (enum-typed
/// fields never enter the IR `field_types` map), so the template would render
/// `result.payload().isEmpty()`, which the wrapper class has no method for.
#[test]
fn presence_on_a_non_optional_payload_union_is_skipped() {
    let out = render(assertion("not_empty", "payload", None), &[]);
    assert_skipped(&out, "payload", "result.payload().isEmpty()");
}

// ---- fields_display_as_text: the exemption is narrowed to the string and length families ----
//
// ~keep `.text()` is real, but it yields a `String`, so only the families a `String` answers are
// substantiated by it. Each pair below is one family from both sides against the SAME field and
// the SAME config, so the only thing that can explain a difference is the family itself.

/// String/equality family: `equals` with a string value is what the `.text()` surface is for,
/// and an existing pinned test (`assertion_union_enum_field_classification_tests`) depends on it.
#[test]
fn a_display_as_text_union_field_still_emits_through_the_text_accessor() {
    let out = render(assertion("equals", "summary", text("ok")), &["summary"]);
    assert_emitted(&out, ".map(v -> v.text()).orElse(\"\")");
}

/// String family, containment half.
#[test]
fn string_containment_on_a_display_as_text_union_still_emits() {
    let out = render(assertion("contains", "summary", text("ok")), &["summary"]);
    assert_emitted(&out, ".map(v -> v.text()).orElse(\"\").contains(");
}

/// Length family: `String.length()` is real.
#[test]
fn length_on_a_display_as_text_union_still_emits() {
    let out = render(assertion("min_length", "summary", number(3)), &["summary"]);
    assert_emitted(&out, ".map(v -> v.text()).orElse(\"\").length() >= 3");
}

/// Numeric family: renders `{String} > 1`, which does not compile. javac rejects it.
#[test]
fn numeric_comparison_on_a_display_as_text_union_is_skipped() {
    let out = render(assertion("greater_than", "summary", number(1)), &["summary"]);
    assert_skipped(&out, "summary", ".orElse(\"\") > 1");
}

/// Count family: renders `{String}.size()`, which does not compile.
#[test]
fn count_on_a_display_as_text_union_is_skipped() {
    let out = render(assertion("count_min", "summary", number(1)), &["summary"]);
    assert_skipped(&out, "summary", ".size()");
}

/// Regex family. Unlike the two above this one COMPILES (`String.matches` exists), so it is
/// refused on soundness: a regex fixture is written against the union's wire form, while
/// `.text()` is a lossy display projection. Nothing but this gate would ever catch it.
#[test]
fn regex_on_a_display_as_text_union_is_skipped() {
    let out = render(assertion("matches_regex", "summary", text("^ok$")), &["summary"]);
    assert_skipped(&out, "summary", ".matches(");
}

/// A numeric-valued `equals` is NOT the string family: the template routes it through
/// `.map(Number::longValue)`, which does not compile on the `String` `.text()` returns.
#[test]
fn numeric_valued_equality_on_a_display_as_text_union_is_skipped() {
    let out = render(assertion("equals", "summary", number(1)), &["summary"]);
    assert_skipped(&out, "summary", "assertEquals");
}

/// Boolean stays substantiated on a display-as-text field, but through the PRESENCE rule, not
/// the text surface — `render_assertion`'s display-as-text branch returns the raw `Optional` for
/// `is_true`/`is_false`, so `.text()` never enters the expression. Narrowing the text exemption
/// must not regress this.
#[test]
fn boolean_on_a_display_as_text_union_still_emits_a_presence_check() {
    let out = render(assertion("is_true", "summary", None), &["summary"]);
    assert_emitted(&out, "java.util.Optional.ofNullable(result.summary()).isPresent()");
}

// ---- bracket-wildcard leaves: gated BEFORE `render_wildcard_assertion` lowers them ----
//
// ~keep The wildcard renderer stringifies each element with `String.valueOf({elem})`. On a
// wrapper leaf that is Jackson's JSON rendering — quotes and object keys included — so its
// `contains` arms match the diagnostic form rather than the value, and its `not_empty` arm
// cannot fail at all, since `String.valueOf` of an absent payload is the four-character "null".
// Both COMPILE, so only this gate can catch them.

#[test]
fn wildcard_containment_on_a_payload_union_leaf_is_skipped() {
    let out = render(assertion("contains", "stages[].payload", text("ok")), &[]);
    assert_skipped(&out, "stages[].payload", "anyMatch");
}

/// The vacuous one: `!String.valueOf(wrapper).isEmpty()` is true even for an absent payload.
#[test]
fn wildcard_presence_on_a_payload_union_leaf_is_skipped() {
    let out = render(assertion("not_empty", "stages[].payload", None), &[]);
    assert_skipped(&out, "stages[].payload", "anyMatch");
}

/// Opposite control, and the one that proves the gate did not simply disable wildcard lowering:
/// a `String` leaf on the SAME container must still expand to the element-relative `anyMatch`.
#[test]
fn wildcard_containment_on_a_string_leaf_still_emits() {
    let out = render(assertion("contains", "stages[].label", text("ok")), &[]);
    assert_emitted(&out, "result.stages().stream().anyMatch(");
    assert!(
        out.contains(".label()).contains("),
        "the lambda body must address the element's own field, got:\n{out}"
    );
}

/// Same control for the presence family.
#[test]
fn wildcard_presence_on_a_string_leaf_still_emits() {
    let out = render(assertion("not_empty", "stages[].label", None), &[]);
    assert_emitted(&out, "result.stages().stream().anyMatch(");
}
