//! Unit tests for `assertions.rs`.
//!
//! Split out of `assertions.rs`, which is at the repo's 1,000-line file-modularization cap and
//! may not grow (mirrors why `wildcard_tests.rs`, nested below, was already split out).

use std::collections::{HashMap, HashSet};

use super::*;
use crate::core::ir::{FieldDef, TypeDef, TypeRef};
use crate::e2e::field_access::{FieldResolver, PythonTypedDictMap};
use crate::e2e::fixture::Assertion;

fn empty_resolver() -> FieldResolver {
    FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    )
}

fn resolver_with_array_field(field: &str) -> FieldResolver {
    FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::from([field.to_string()]),
        &HashSet::new(),
    )
}

fn make_assertion(assertion_type: &str, field: Option<&str>, value: Option<serde_json::Value>) -> Assertion {
    Assertion {
        assertion_type: assertion_type.to_string(),
        field: field.map(|s| s.to_string()),
        value,
        ..Default::default()
    }
}

fn render_field_contains(resolver: &FieldResolver, field: &str, value: &str) -> String {
    let assertion = make_assertion("contains", Some(field), Some(serde_json::json!(value)));
    let mut out = String::new();
    render_assertion(
        &mut out,
        &assertion,
        "result",
        resolver,
        &HashSet::new(),
        &HashMap::new(),
        false,
    );
    out
}

fn resolver_with_optional_field(field: &str) -> FieldResolver {
    FieldResolver::new(
        &HashMap::new(),
        &HashSet::from([field.to_string()]),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    )
}

fn render_field_assertion(resolver: &FieldResolver, assertion: &Assertion) -> String {
    let mut out = String::new();
    render_assertion(
        &mut out,
        assertion,
        "result",
        resolver,
        &HashSet::new(),
        &HashMap::new(),
        false,
    );
    out
}

/// A resolver anchored at `root_type`, classifying `typeddict_types` as Python `TypedDict`s and
/// recording `(owner, field, target)` traversal edges — the same shape production code builds
/// via `FieldResolver::python_typeddict_fields`, but hand-assembled here so a test can name an
/// exact, minimal type graph instead of routing through IR extraction.
fn typeddict_resolver(typeddict_types: &[&str], field_types: &[(&str, &str, &str)], root_type: &str) -> FieldResolver {
    let mut map = PythonTypedDictMap {
        typeddict_types: typeddict_types.iter().map(|s| s.to_string()).collect(),
        ..Default::default()
    };
    for (owner, field, target) in field_types {
        map.field_types
            .entry(owner.to_string())
            .or_default()
            .insert(field.to_string(), target.to_string());
    }
    empty_resolver().with_python_typeddict_map(map, Some(root_type.to_string()))
}

fn map_owner_type_defs() -> Vec<TypeDef> {
    vec![
        TypeDef {
            name: "Report".to_string(),
            fields: vec![FieldDef {
                name: "entries".to_string(),
                ty: TypeRef::Map(
                    Box::new(TypeRef::String),
                    Box::new(TypeRef::Named("Metadata".to_string())),
                ),
                ..FieldDef::default()
            }],
            is_return_type: true,
            has_default: true,
            ..TypeDef::default()
        },
        TypeDef {
            name: "Metadata".to_string(),
            fields: vec![FieldDef {
                name: "title".to_string(),
                ty: TypeRef::String,
                ..FieldDef::default()
            }],
            is_return_type: true,
            has_default: true,
            ..TypeDef::default()
        },
    ]
}

fn production_map_resolver() -> FieldResolver {
    empty_resolver().with_python_typeddict_facts(
        FieldResolver::python_typeddict_facts(&map_owner_type_defs()),
        Some("Report".to_string()),
    )
}

fn assert_generated_python_runs(setup: &str, assertion: &str) {
    let script = format!("{setup}\n\ndef test_case():\n{assertion}\ntest_case()\n");
    let output = std::process::Command::new("python3")
        .arg("-c")
        .arg(&script)
        .output()
        .expect("python3 must execute generated Python assertion");
    assert!(
        output.status.success(),
        "generated Python assertion failed:\n{}\nscript:\n{script}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// `Option<DataNode>` presence: before the fix this rendered `assert result.data is
/// True`, which is never true for a present non-bool object (Python's `is` compares
/// identity, and no struct instance is ever the singleton `True`).
#[test]
fn is_true_on_optional_struct_field_checks_presence() {
    let out = render_field_assertion(
        &resolver_with_optional_field("data"),
        &make_assertion("is_true", Some("data"), None),
    );
    assert_eq!(out, "    assert result.data is not None\n");
}

#[test]
fn is_false_on_optional_struct_field_checks_absence() {
    let out = render_field_assertion(
        &resolver_with_optional_field("data"),
        &make_assertion("is_false", Some("data"), None),
    );
    assert_eq!(out, "    assert result.data is None\n");
}

/// A follow-on member access through the optional field: Python's dynamic typing means
/// `result.data.kind` needs no unwrap ceremony at the codegen level (unlike Rust/Java/
/// Kotlin) -- it only needs `is_true`'s presence check (above) to be correct so the
/// assertion the fixture actually declares runs before this one, rather than always
/// failing first regardless of whether `data` is present.
#[test]
fn equals_on_nested_field_through_optional_parent_is_unchanged() {
    let out = render_field_assertion(
        &resolver_with_optional_field("data"),
        &make_assertion("equals", Some("data.kind"), Some(serde_json::json!("KeyValue"))),
    );
    assert!(out.contains("result.data.kind"), "got: {out}");
}

#[test]
fn is_true_on_non_optional_field_is_unchanged() {
    let out = render_field_assertion(&empty_resolver(), &make_assertion("is_true", Some("active"), None));
    assert_eq!(out, "    assert result.active is True\n");
}

#[cfg(test)]
#[path = "tests/wildcard_tests.rs"]
mod wildcard_tests;

#[test]
fn not_empty_for_python_rejects_empty_sized_values_but_accepts_zero() {
    let resolver = empty_resolver();
    let assertion = make_assertion("not_empty", None, None);
    let mut out = String::new();
    render_assertion(
        &mut out,
        &assertion,
        "result",
        &resolver,
        &HashSet::new(),
        &HashMap::new(),
        false,
    );
    // Bare `assert result` fails on a legitimate 0, 0.0 or False.
    assert_eq!(
        out.trim(),
        "assert result is not None and (not hasattr(result, \"__len__\") or len(result) > 0)"
    );
}

/// Regression test for a one-sided-strip bug: `.strip()` was applied to the actual value
/// while the fixture `expected` literal was emitted verbatim. Fixture expectations may
/// legitimately end in `\n`, so stripping only one side made those assertions impossible
/// to satisfy — and stripping both would silently mask real trailing-whitespace
/// regressions. Equals is exact: neither side is normalized.
/// Control for the trim fix: the tightened contract must still DISCRIMINATE values that
/// differ only in trailing whitespace. If either side were normalized, the emitted
/// assertion for "hello\n" and for "hello" would be identical and a real trailing-newline
/// regression would pass unnoticed.
#[test]
fn render_assertion_equals_still_discriminates_trailing_whitespace() {
    let render_for = |value: &str| {
        let resolver = empty_resolver();
        let assertion = make_assertion("equals", None, Some(serde_json::Value::String(value.into())));
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            &resolver,
            &HashSet::new(),
            &HashMap::new(),
            false,
        );
        out
    };
    let emitted = render_for("hello\n");
    // The actual side must be the bare expression: any normalizing call (trim/strip/
    // case-folding) wrapped around it would silently accept a mismatched value.
    assert_eq!(
        emitted, "    assert result == \"hello\\n\"\n",
        "emitted assertion drifted: {emitted}"
    );
    // And a value differing only by the trailing newline must still produce a
    // different expectation, proving trailing whitespace is discriminated.
    assert_ne!(
        emitted,
        render_for("hello"),
        "trailing newline must still change the emitted assertion"
    );
}

#[test]
fn render_assertion_equals_string_compares_exactly_without_strip() {
    let resolver = empty_resolver();
    let assertion = make_assertion("equals", None, Some(serde_json::Value::String("hello\n".into())));
    let mut out = String::new();
    render_assertion(
        &mut out,
        &assertion,
        "result",
        &resolver,
        &HashSet::new(),
        &HashMap::new(),
        false,
    );
    assert!(
        !out.contains(".strip()"),
        "equals must not strip either side; got: {out}"
    );
    assert!(out.contains("assert result =="), "got: {out}");
}

#[test]
fn render_assertion_contains_string_array_uses_item_texts() {
    let resolver = resolver_with_array_field("structure");
    let assertion = make_assertion(
        "contains",
        Some("structure"),
        Some(serde_json::Value::String("Function".into())),
    );
    let mut out = String::new();
    render_assertion(
        &mut out,
        &assertion,
        "result",
        &resolver,
        &HashSet::new(),
        &HashMap::new(),
        false,
    );

    assert!(out.contains("_alef_e2e_item_texts(item)"), "got: {out}");
    assert!(out.contains("for item in result.structure"), "got: {out}");
}

#[test]
fn build_python_method_call_root_child_count() {
    let expr = build_python_method_call("tree", "root_child_count", None);
    assert_eq!(expr, "tree.root_node().child_count()");
}

#[test]
fn negate_contains_expr_simple_membership_not_in() {
    let expr = "\"test\" in result.content";
    let negated = negate_contains_expr(expr, false, false);
    assert_eq!(negated, "\"test\" not in result.content");
}

#[test]
fn negate_contains_expr_array_uses_not_wrapper() {
    let expr = "any(\"test\" in text for item in result.structure for text in _alef_e2e_item_texts(item))";
    let negated = negate_contains_expr(expr, true, false);
    assert!(
        negated.contains("not ("),
        "expected `not (...)` wrapper for array expression"
    );
}

#[test]
fn negate_contains_expr_enum_uses_not_wrapper() {
    let expr = "\"test\".lower() in str(result.status).lower()";
    let negated = negate_contains_expr(expr, false, true);
    assert!(
        negated.contains("not ("),
        "expected `not (...)` wrapper for enum expression"
    );
}

#[test]
fn negate_contains_expr_preserves_already_negated() {
    let expr = "\"test\" not in result.content";
    let negated = negate_contains_expr(expr, false, false);
    // Should not double-negate: ` not in ` already present, so wrap with `not (...)`
    assert!(
        negated.contains("not ("),
        "expected `not (...)` wrapper for already-negated expression"
    );
}

#[test]
#[should_panic(expected = "unsupported assertion type 'bogus_type' on synthetic field 'chunks_have_content'")]
fn python_synthetic_chunks_unsupported_type_fails_loudly() {
    let assertion = make_assertion("bogus_type", Some("chunks_have_content"), None);
    let mut out = String::new();
    render_synthetic_field(&mut out, &assertion, "result", "chunks_have_content", &empty_resolver());
}

#[test]
fn python_synthetic_chunks_supported_type_renders_assertion() {
    let assertion = make_assertion("is_true", Some("chunks_have_content"), None);
    let mut out = String::new();
    let handled = render_synthetic_field(&mut out, &assertion, "result", "chunks_have_content", &empty_resolver());
    assert!(handled);
    assert_eq!(out.trim(), "assert all(c.content for c in (result.chunks or []))");
}

#[test]
#[should_panic(expected = "unsupported assertion type 'bogus_type' on synthetic field 'embeddings'")]
fn python_synthetic_embeddings_unsupported_type_fails_loudly() {
    let assertion = make_assertion("bogus_type", Some("embeddings"), None);
    let mut out = String::new();
    render_synthetic_field(&mut out, &assertion, "result", "embeddings", &empty_resolver());
}

#[test]
fn python_synthetic_embeddings_supported_type_renders_assertion() {
    let assertion = make_assertion("not_empty", Some("embeddings"), None);
    let mut out = String::new();
    let handled = render_synthetic_field(&mut out, &assertion, "result", "embeddings", &empty_resolver());
    assert!(handled);
    assert_eq!(out.trim(), "assert len(result) > 0");
}

#[test]
fn python_embedding_dimensions_unsupported_type_no_longer_emits_invalid_syntax() {
    let assertion = make_assertion("bogus_type", Some("embedding_dimensions"), None);
    let mut out = String::new();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        render_synthetic_field(
            &mut out,
            &assertion,
            "result",
            "embedding_dimensions",
            &empty_resolver(),
        );
    }));
    assert!(result.is_err(), "expected a panic for unsupported assertion type");
    assert!(
        !out.contains("//"),
        "generated output must never contain a `//` (invalid Python comment token): {out}"
    );
}

#[test]
fn python_embedding_dimensions_supported_type_renders_assertion() {
    let assertion = make_assertion(
        "greater_than",
        Some("embedding_dimensions"),
        Some(serde_json::Value::from(10)),
    );
    let mut out = String::new();
    let handled = render_synthetic_field(
        &mut out,
        &assertion,
        "result",
        "embedding_dimensions",
        &empty_resolver(),
    );
    assert!(handled);
    assert_eq!(out.trim(), "assert (len(result[0]) if result else 0) > 10");
}

// ---------------------------------------------------------------------------
// TypedDict-vs-attribute-access accessor dispatch, through the real assertion pipeline
// ---------------------------------------------------------------------------

/// A scalar field on a `TypedDict`-classified result renders a subscript assertion — the exact
/// shape the reported defect needed: a consumer with `[workspace.dto] python_output =
/// "typed-dict"` generated `result.status_code`, which is `AttributeError: 'dict' object has no
/// attribute 'status_code'` against the plain `dict` the backend actually returns.
#[test]
fn a_scalar_field_on_a_typeddict_result_type_renders_a_subscript_assertion() {
    let resolver = typeddict_resolver(&["ApiResult"], &[], "ApiResult");
    let out = render_field_assertion(
        &resolver,
        &make_assertion("equals", Some("status_code"), Some(serde_json::json!(200))),
    );
    assert_eq!(out, "    assert result[\"status_code\"] == 200\n");
}

/// CONTROL: the identical field/assertion against a resolver with no `TypedDict` classification
/// (the default every resolver had before `PythonTypedDictMap` existed) still renders attribute
/// access — proving the subscript behaviour above is conditional on the map, not blanket.
#[test]
fn a_scalar_field_on_a_non_typeddict_result_type_renders_an_attribute_assertion() {
    let out = render_field_assertion(
        &empty_resolver(),
        &make_assertion("equals", Some("status_code"), Some(serde_json::json!(200))),
    );
    assert_eq!(out, "    assert result.status_code == 200\n");
}

/// REGRESSION (tree-sitter-language-pack#183): both `Report` and `Metadata` are `is_return_type`,
/// so `FieldResolver::python_typeddict_facts` (the real pyo3-backend-agreeing classifier) never
/// puts either in `typeddict_types` -- the whole chain renders attribute access, matching the
/// native `#[pyclass]` the pyo3 backend actually returns for both. Before the fix, a `["Metadata"
/// not reexported]`-style config could make the classifier disagree between the two, subscripting
/// one side of the chain; that branch no longer exists, so there is nothing left to distinguish.
#[test]
fn a_return_type_map_value_under_a_return_type_owner_runs_the_generated_assertion() {
    let resolver = production_map_resolver();
    let out = render_field_assertion(
        &resolver,
        &make_assertion("equals", Some("entries[alpha].title"), Some(serde_json::json!("Doc"))),
    );
    assert_eq!(out, "    assert result.entries.get(\"alpha\").title == \"Doc\"\n");
    assert_generated_python_runs(
        concat!(
            "class Metadata:\n",
            "    def __init__(self):\n",
            "        self.title = \"Doc\"\n\n",
            "class Report:\n",
            "    def __init__(self):\n",
            "        self.entries = {\"alpha\": Metadata()}\n\n",
            "result = Report()",
        ),
        &out,
    );
}

/// An `Optional` field owned by a `TypedDict` type narrows via subscript access in both the
/// ternary's condition and its consequent, then keeps subscripting into the nested `TypedDict`
/// field beyond it.
#[test]
fn an_optional_typeddict_field_narrows_via_subscript_before_descending() {
    let resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::from(["markdown".to_string()]),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    )
    .with_python_typeddict_map(
        {
            let mut map = PythonTypedDictMap {
                typeddict_types: ["ApiResult", "Markdown"].iter().map(|s| s.to_string()).collect(),
                ..Default::default()
            };
            map.field_types
                .entry("ApiResult".to_string())
                .or_default()
                .insert("markdown".to_string(), "Markdown".to_string());
            map
        },
        Some("ApiResult".to_string()),
    );
    let out = render_field_assertion(
        &resolver,
        &make_assertion("equals", Some("markdown.content"), Some(serde_json::json!("hi"))),
    );
    assert_eq!(
        out,
        "    assert (result[\"markdown\"][\"content\"] if result[\"markdown\"] else None) == \"hi\"\n"
    );
}

/// A path that starts on a `TypedDict` result but descends into a field whose OWN type is not
/// itself classified as `TypedDict` (e.g. it stays a native `#[pyclass]`) switches back to
/// attribute access at that link, rather than inheriting the root's classification.
#[test]
fn descending_from_a_typeddict_result_into_a_non_typeddict_nested_type_uses_attribute_access() {
    let resolver = typeddict_resolver(&["ApiResult"], &[("ApiResult", "metadata", "Metadata")], "ApiResult");
    let out = render_field_assertion(
        &resolver,
        &make_assertion("equals", Some("metadata.title"), Some(serde_json::json!("Doc"))),
    );
    assert_eq!(out, "    assert result[\"metadata\"].title == \"Doc\"\n");
}

/// End-to-end regression for the redundant-paren defect (ruff `UP034`): `not_empty` wraps
/// `field_access` in its own `is not None` check (parens load-bearing there) AND places it as the
/// sole argument to `hasattr`/`len` (parens redundant there). When `field_access` is itself an
/// `Optional`-narrowing ternary — exactly the shape a `TypedDict` field produces — the emitted
/// `hasattr`/`len` calls must not double-wrap it in a second, redundant pair of parens.
#[test]
fn not_empty_on_an_optional_typeddict_field_does_not_double_wrap_the_narrowing_ternary() {
    let resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::from(["markdown".to_string()]),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    )
    .with_python_typeddict_map(
        {
            let mut map = PythonTypedDictMap {
                typeddict_types: ["ApiResult", "Markdown"].iter().map(|s| s.to_string()).collect(),
                ..Default::default()
            };
            map.field_types
                .entry("ApiResult".to_string())
                .or_default()
                .insert("markdown".to_string(), "Markdown".to_string());
            map
        },
        Some("ApiResult".to_string()),
    );
    let out = render_field_assertion(&resolver, &make_assertion("not_empty", Some("markdown.content"), None));
    assert_eq!(
        out.trim(),
        "assert (result[\"markdown\"][\"content\"] if result[\"markdown\"] else None) is not None and \
         (not hasattr(result[\"markdown\"][\"content\"] if result[\"markdown\"] else None, \"__len__\") \
         or len(result[\"markdown\"][\"content\"] if result[\"markdown\"] else None) > 0)"
    );
    assert!(
        !out.contains("hasattr(("),
        "hasattr's argument must not be double-wrapped: {out}"
    );
    assert!(
        !out.contains("len(("),
        "len's argument must not be double-wrapped: {out}"
    );
}

/// A resolver identical to `typeddict_resolver`'s "optional TypedDict field" shape, reused by
/// both `str()` call-argument regression tests below so the same narrowing ternary
/// (`(result["markdown"]["content"] if result["markdown"] else None)`) exercises both
/// `_alef_e2e_text(...)` (equals+enum) and `str(...)` (contains_any+enum).
///
/// Both `"markdown"` AND `"markdown.content"` are registered as optional: `"markdown"` alone
/// drives the ternary crossing in the rendered accessor (a crossing at a NON-last segment, per
/// `render_python_with_optionals`'s doc), while `"markdown.content"` -- the full leaf path -- is
/// what `FieldResolver::is_optional` checks by exact match to set the `field_is_optional` flag
/// the `contains_any` template's presence guard reads. Registering only `"markdown"` would leave
/// that flag false and the guard line unrendered; both are independently optional in the IR for
/// the analogous nested-`Option` shape this stands in for.
fn typeddict_resolver_with_optional_markdown_content() -> FieldResolver {
    FieldResolver::new(
        &HashMap::new(),
        &HashSet::from(["markdown".to_string(), "markdown.content".to_string()]),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    )
    .with_python_typeddict_map(
        {
            let mut map = PythonTypedDictMap {
                typeddict_types: ["ApiResult", "Markdown"].iter().map(|s| s.to_string()).collect(),
                ..Default::default()
            };
            map.field_types
                .entry("ApiResult".to_string())
                .or_default()
                .insert("markdown".to_string(), "Markdown".to_string());
            map
        },
        Some("ApiResult".to_string()),
    )
}

// ---------------------------------------------------------------------------
// `str()`/`_alef_e2e_text()` call-argument redundant-paren sweep (round 2)
// ---------------------------------------------------------------------------

/// `equals` on an enum-classified field whose accessor is a narrowing ternary: the ternary's own
/// parens are dropped once it lands as the sole argument of `_alef_e2e_text(...)` -- the same
/// `strip_redundant_call_arg_parens` fix `not_empty`/`min_length` already had, extended to the
/// `str()`-shaped `_alef_e2e_text()` call this task closes.
#[test]
fn equals_on_an_optional_typeddict_enum_field_does_not_double_wrap_the_narrowing_ternary_in_alef_e2e_text() {
    let resolver = typeddict_resolver_with_optional_markdown_content();
    let assertion = make_assertion("equals", Some("markdown.content"), Some(serde_json::json!("hi")));
    let fields_enum: HashSet<String> = ["markdown.content".to_string()].into_iter().collect();
    let mut out = String::new();
    render_assertion(
        &mut out,
        &assertion,
        "result",
        &resolver,
        &fields_enum,
        &HashMap::new(),
        false,
    );
    assert_eq!(
        out.trim(),
        "assert _alef_e2e_text(result[\"markdown\"][\"content\"] if result[\"markdown\"] else None).lower() \
         == \"hi\".lower()"
    );
    assert!(
        !out.contains("_alef_e2e_text(("),
        "_alef_e2e_text's argument must not be double-wrapped: {out}"
    );
}

/// `contains_any` on the SAME optional/`TypedDict` enum field renders the identical narrowing
/// ternary TWICE: once bare, as the `field_is_optional` presence guard (`{{ field_access }} is
/// not None`), where the parens are load-bearing -- Python's conditional-expression precedence is
/// lower than `is not`, so an unparenthesized `A if B else C is not None` parses as `A if B else
/// (C is not None)`, not `(A if B else C) is not None` -- and once as the sole argument of
/// `str(...)` inside the `any(...)` comparison, where the same parens are redundant. Both must
/// render correctly from the one fix: the guard keeps its parens, `str(...)`'s argument does not
/// double them.
#[test]
fn contains_any_on_an_optional_typeddict_enum_field_keeps_the_guards_parens_but_not_strs() {
    let resolver = typeddict_resolver_with_optional_markdown_content();
    let assertion = Assertion {
        assertion_type: "contains_any".to_string(),
        field: Some("markdown.content".to_string()),
        values: Some(vec![serde_json::json!("hi")]),
        ..Default::default()
    };
    let fields_enum: HashSet<String> = ["markdown.content".to_string()].into_iter().collect();
    let mut out = String::new();
    render_assertion(
        &mut out,
        &assertion,
        "result",
        &resolver,
        &fields_enum,
        &HashMap::new(),
        false,
    );
    let guard_line = "    assert (result[\"markdown\"][\"content\"] if result[\"markdown\"] else None) is not None\n";
    let cmp_line = "    assert any(v.lower() in str(result[\"markdown\"][\"content\"] if result[\"markdown\"] else \
        None).lower() for v in [\"hi\"])\n";
    assert_eq!(out, format!("{guard_line}{cmp_line}"));
}

/// `python_contains_expr` backs `contains`/`contains_all`/`not_contains`. The enum branch wraps
/// its `field_access` argument in `str(...)`, the same call-argument shape `contains_any` has
/// above; unit-tested directly here (rather than through the full assertion pipeline) since the
/// function takes the narrowing-ternary shape as a plain `&str` argument.
#[test]
fn python_contains_expr_enum_branch_does_not_double_wrap_a_narrowing_ternary() {
    let field_access = "(result[\"markdown\"][\"content\"] if result[\"markdown\"] else None)";
    let expr = python_contains_expr(field_access, "\"hi\"", true, false, true);
    assert_eq!(
        expr,
        "\"hi\".lower() in str(result[\"markdown\"][\"content\"] if result[\"markdown\"] else None).lower()"
    );
}

/// CONTROL: a `field_access` with no enclosing parens to strip is passed through unchanged inside
/// `str(...)` — proving the fix only ever removes a REDUNDANT wrap, never alters an expression
/// that had none to begin with.
#[test]
fn python_contains_expr_enum_branch_leaves_an_unparenthesized_field_access_untouched() {
    let expr = python_contains_expr("result.status", "\"hi\"", true, false, true);
    assert_eq!(expr, "\"hi\".lower() in str(result.status).lower()");
}
