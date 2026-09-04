//! Renders a real Swift check for an assertion whose fixture path steps PAST a swift-bridge
//! JSON-bridged leaf, by decoding the leaf's `RustString` with `JSONSerialization` and then
//! subscripting/keying into the decoded value.
//!
//! ~keep Before this module existed, `assertions::render_assertion` refused every such path
//! outright — `leaf_shape::json_bridged_traversal_skip` fired for an indexed element
//! (`results[0].detected_languages[0]`), an un-indexed dotted projection
//! (`results[0].metadata.output_format`), and every deeper mix of the two, even though
//! `leaf_shape::swift_json_bridged_count_expr` already proved the same decode approach works for
//! the narrower "count the bridged leaf itself" case. `FieldResolver::swift_json_bridged_navigation`
//! supplies the WHAT (an ordered list of index/key steps); this module supplies the HOW (turning
//! those steps into a Swift expression and that expression into one of the assertion types the
//! e2e fixture schema declares).
//!
//! Scoped assertion types are those actually exercised by resolvable fixture paths at the time of
//! writing: `equals`, `not_empty`, `is_empty`, `count_min`, `count_equals`, `greater_than`,
//! `greater_than_or_equal`, `contains`, `contains_all`, `min_length`. An assertion type outside
//! that list still renders a skip —
//! [`crate::e2e::codegen::field_skip::FieldSkip::NavigatedJsonBridgedAssertionTypeNotSupportedInSwift`],
//! a `GeneratorGap` rather than the `LanguageLimitation` the caller used to render, because
//! navigation itself succeeded here and only the assertion-type renderer is missing.

use std::fmt::Write as FmtWrite;

use crate::e2e::codegen::field_skip::FieldSkip;
use crate::e2e::field_access::{FieldResolver, JsonNavStep};
use crate::e2e::fixture::Assertion;

use super::values::escape_swift;

/// Attempts to render `assertion` as a real Swift check over decoded JSON, when its field steps
/// past a swift-bridge JSON-bridged leaf.
///
/// Returns `false` (writes nothing) when `assertion.field` does not traverse past a JSON-bridged
/// leaf at all, or when [`FieldResolver::swift_json_bridged_navigation`] cannot express the
/// traversal (a wildcard or map-key bracket segment) — callers must keep their existing skip in
/// both cases, since neither is this module's to answer. Returns `true` in every other case: a
/// successfully navigated field always gets EITHER a real assertion or the
/// `NavigatedJsonBridgedAssertionTypeNotSupportedInSwift` skip, never silence.
pub(super) fn render_json_bridged_navigated_assertion(
    out: &mut String,
    assertion: &Assertion,
    field_resolver: &FieldResolver,
    result_var: &str,
) -> bool {
    let Some(field) = assertion.field.as_deref().filter(|f| !f.is_empty()) else {
        return false;
    };
    let Some((leaf_field, steps)) = field_resolver.swift_json_bridged_navigation(field) else {
        return false;
    };
    let leaf_expr = field_resolver.accessor(&leaf_field, "swift", result_var);
    let value_expr = navigated_value_expr(&leaf_expr, &steps);
    let local = json_nav_local_name(field, &assertion.assertion_type);

    let Some(body) = build_assertion_lines(assertion, &local) else {
        let _ = writeln!(
            out,
            "        // skipped: {}; assertion type '{}' has no Swift renderer over decoded JSON yet",
            FieldSkip::NavigatedJsonBridgedAssertionTypeNotSupportedInSwift.message(field),
            assertion.assertion_type
        );
        return true;
    };
    let _ = writeln!(out, "        let {local}: Any? = {value_expr}");
    out.push_str(&body);
    true
}

/// Builds the `JSONSerialization`-decode-then-navigate Swift expression for `leaf_expr` (the
/// bridged leaf's own accessor call, e.g. `result.results()[0].detectedLanguages()`) and `steps`.
///
/// ~keep Mirrors `leaf_shape::swift_json_bridged_count_expr`'s `?.`-detection: an optional
/// ancestor in `leaf_expr` makes the whole chain `Optional<RustString>`, so the trailing call
/// needs its own `?.`, coalesced to the JSON text `"null"` (never `""`, which is not valid JSON)
/// for a missing value.
fn navigated_value_expr(leaf_expr: &str, steps: &[JsonNavStep]) -> String {
    let json_text_expr = if leaf_expr.contains("?.") {
        format!("({leaf_expr}?.toString() ?? \"null\")")
    } else {
        format!("{leaf_expr}.toString()")
    };
    let mut expr = format!("(try? JSONSerialization.jsonObject(with: Data({json_text_expr}.utf8)))");
    for step in steps {
        expr = match step {
            // ~keep `?[n]` would be an UNCHECKED Swift subscript: optional chaining guards a nil
            // array but not a short one, so a decoded `[]` where the fixture expects an element
            // traps with "Index out of range" and aborts the whole XCTest process -- destroying
            // every later test's result instead of failing this one assertion. `dropFirst(n)
            // .first` yields nil for that case, so a genuine regression reads as a normal
            // assertion failure.
            JsonNavStep::Index(index) => {
                format!("(({expr}) as? [Any]).flatMap {{ $0.dropFirst({index}).first }}")
            }
            JsonNavStep::Key(key) => format!("(({expr}) as? [String: Any])?[\"{}\"]", escape_swift(key)),
        };
    }
    expr
}

/// A stable per-(field, assertion type) local variable name, so two navigated assertions in the
/// same generated test method never redeclare the same Swift identifier.
fn json_nav_local_name(field: &str, assertion_type: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    field.hash(&mut hasher);
    assertion_type.hash(&mut hasher);
    format!("_json_nav_{:x}", hasher.finish() & 0xffff_ffff)
}

/// The rendered assertion body referencing `local` (an `Any?` already bound to the navigated,
/// decoded value), or `None` when `assertion`'s type or value shape has no renderer here yet.
fn build_assertion_lines(assertion: &Assertion, local: &str) -> Option<String> {
    match assertion.assertion_type.as_str() {
        "equals" => build_equals(assertion, local),
        "not_empty" => Some(build_presence(local, true)),
        "is_empty" => Some(build_presence(local, false)),
        "count_min" | "count_equals" => build_count(assertion, local),
        "greater_than" | "greater_than_or_equal" => build_numeric_compare(assertion, local),
        "contains" => build_contains(assertion, local),
        "contains_all" => build_contains_all(assertion, local),
        "min_length" => build_min_length(assertion, local),
        _ => None,
    }
}

fn build_equals(assertion: &Assertion, local: &str) -> Option<String> {
    let expected = assertion.value.as_ref()?;
    if let Some(s) = expected.as_str() {
        let escaped = escape_swift(s);
        Some(format!(
            "        XCTAssertEqual(({local} as? String) ?? \"\", \"{escaped}\")\n"
        ))
    } else if let Some(b) = expected.as_bool() {
        Some(format!("        XCTAssertEqual(({local} as? Bool) ?? false, {b})\n"))
    } else {
        let n = expected.as_i64()?;
        Some(format!(
            "        XCTAssertEqual(({local} as? NSNumber)?.intValue ?? 0, {n})\n"
        ))
    }
}

/// `not_empty`/`is_empty` cover both an array-shaped leaf (`pages`, `extracted_keywords`) and a
/// string-shaped one reached through a further key (`chunks[0].content`) with one predicate: the
/// decoded value counts as empty when it is neither a non-empty `[Any]` nor a non-empty `String`
/// (and a decode failure, `nil`, reads the same way — correctly empty).
fn build_presence(local: &str, expect_non_empty: bool) -> String {
    let is_empty_predicate =
        format!("(({local} as? [Any])?.isEmpty ?? true) && (({local} as? String)?.isEmpty ?? true)");
    if expect_non_empty {
        format!("        XCTAssertFalse({is_empty_predicate}, \"expected non-empty value\")\n")
    } else {
        format!("        XCTAssertTrue({is_empty_predicate}, \"expected empty value\")\n")
    }
}

fn build_count(assertion: &Assertion, local: &str) -> Option<String> {
    let n = assertion.value.as_ref()?.as_u64()?;
    let count_expr = format!("(({local} as? [Any])?.count ?? 0)");
    Some(if assertion.assertion_type == "count_min" {
        format!("        XCTAssertGreaterThanOrEqual({count_expr}, {n})\n")
    } else {
        format!("        XCTAssertEqual({count_expr}, {n})\n")
    })
}

fn build_numeric_compare(assertion: &Assertion, local: &str) -> Option<String> {
    let n = assertion.value.as_ref()?.as_i64()?;
    let num_expr = format!("(({local} as? NSNumber)?.intValue ?? 0)");
    Some(if assertion.assertion_type == "greater_than" {
        format!("        XCTAssertGreaterThan({num_expr}, {n})\n")
    } else {
        format!("        XCTAssertGreaterThanOrEqual({num_expr}, {n})\n")
    })
}

fn build_contains(assertion: &Assertion, local: &str) -> Option<String> {
    let s = assertion.value.as_ref()?.as_str()?;
    let escaped = escape_swift(s);
    Some(format!(
        "        XCTAssertTrue((({local} as? String) ?? \"\").contains(\"{escaped}\"), \"expected to contain: \
         {escaped}\")\n"
    ))
}

/// Mirrors the non-bridged `min_length` renderer in `assertions.rs`: character-count comparison
/// against a `String`, not the decoded value's own `count` (which would be array-length for a
/// `[Any]`). Coalescing a non-`String` decode to `""` before `.count` keeps a decode failure or a
/// `nil` reading as length 0 rather than crashing the cast.
fn build_min_length(assertion: &Assertion, local: &str) -> Option<String> {
    let n = assertion.value.as_ref()?.as_u64()?;
    Some(format!(
        "        XCTAssertGreaterThanOrEqual((({local} as? String) ?? \"\").count, {n})\n"
    ))
}

fn build_contains_all(assertion: &Assertion, local: &str) -> Option<String> {
    let values = assertion.values.as_ref()?;
    let mut lines = String::new();
    for value in values {
        let s = value.as_str()?;
        let escaped = escape_swift(s);
        let _ = writeln!(
            lines,
            "        XCTAssertTrue((({local} as? [Any])?.contains(where: {{ ($0 as? String) == \"{escaped}\" }}) ?? \
             false), \"expected to contain: {escaped}\")"
        );
    }
    Some(lines)
}

#[cfg(test)]
mod tests {
    use super::render_json_bridged_navigated_assertion;
    use crate::e2e::field_access::{FieldResolver, SwiftFirstClassMap};
    use crate::e2e::fixture::Assertion;
    use std::collections::{HashMap, HashSet};

    fn resolver_with_json_bridged_field(field_name: &str) -> FieldResolver {
        let swift_first_class_map = SwiftFirstClassMap {
            json_bridged_field_names: HashSet::from([field_name.to_string()]),
            ..SwiftFirstClassMap::default()
        };
        FieldResolver::new_with_swift_first_class(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::from([field_name.to_string()]),
            &HashSet::new(),
            &HashMap::new(),
            swift_first_class_map,
        )
    }

    fn assertion(assertion_type: &str, field: &str, value: Option<serde_json::Value>) -> Assertion {
        Assertion {
            assertion_type: assertion_type.to_string(),
            field: Some(field.to_string()),
            value,
            ..Assertion::default()
        }
    }

    /// The `language_detection_config` shape: `equals` on a numeric-indexed element of a
    /// JSON-bridged array leaf. This is the fixture whose ONLY assertion used to degrade the
    /// whole generated test to `XCTSkipIf(true, ...)`.
    #[test]
    fn equals_on_an_indexed_element_renders_a_real_decode_and_compare() {
        let resolver = resolver_with_json_bridged_field("detected_languages");
        let mut out = String::new();

        let rendered = render_json_bridged_navigated_assertion(
            &mut out,
            &assertion(
                "equals",
                "results[0].detected_languages[0]",
                Some(serde_json::json!("eng")),
            ),
            &resolver,
            "result",
        );

        assert!(rendered, "an indexed element after a bridged leaf must be handled");
        assert!(
            out.contains("JSONSerialization.jsonObject"),
            "must decode the bridged leaf's JSON, got:\n{out}"
        );
        assert!(
            out.contains("as? [Any]).flatMap { $0.dropFirst(0).first }"),
            "must index element 0 of the decoded array bounds-safely, got:\n{out}"
        );
        assert!(
            out.contains("XCTAssertEqual(") && out.contains("\"eng\""),
            "must compare against the fixture's expected value, got:\n{out}"
        );
    }

    /// The un-indexed projection shape: a dotted key with no bracket at all.
    #[test]
    fn equals_on_an_un_indexed_dotted_projection_renders_a_real_decode_and_compare() {
        let resolver = resolver_with_json_bridged_field("metadata");
        let mut out = String::new();

        let rendered = render_json_bridged_navigated_assertion(
            &mut out,
            &assertion(
                "equals",
                "results[0].metadata.output_format",
                Some(serde_json::json!("markdown")),
            ),
            &resolver,
            "result",
        );

        assert!(rendered, "a dotted projection after a bridged leaf must be handled");
        assert!(
            out.contains("as? [String: Any])?[\"output_format\"]"),
            "must key into the decoded object, got:\n{out}"
        );
        assert!(out.contains("\"markdown\""), "got:\n{out}");
    }

    /// The `pdf_hierarchy_config` shape: `not_empty` reached through an index AND two further
    /// dotted keys — `pages[0].hierarchy.blocks`.
    #[test]
    fn not_empty_through_an_index_and_nested_keys_still_renders() {
        let resolver = resolver_with_json_bridged_field("pages");
        let mut out = String::new();

        let rendered = render_json_bridged_navigated_assertion(
            &mut out,
            &assertion("not_empty", "results[0].pages[0].hierarchy.blocks", None),
            &resolver,
            "result",
        );

        assert!(rendered, "a not_empty over a navigated field must be handled");
        assert!(out.contains("XCTAssertFalse("), "got:\n{out}");
    }

    /// count_min on a nested array reached through two dotted keys.
    #[test]
    fn count_min_on_a_nested_array_renders_a_count_comparison() {
        let resolver = resolver_with_json_bridged_field("metadata");
        let mut out = String::new();

        let rendered = render_json_bridged_navigated_assertion(
            &mut out,
            &assertion(
                "count_min",
                "results[0].metadata.format.html.headers",
                Some(serde_json::json!(2)),
            ),
            &resolver,
            "result",
        );

        assert!(rendered);
        assert!(
            out.contains("XCTAssertGreaterThanOrEqual(") && out.contains(".count ?? 0), 2)"),
            "got:\n{out}"
        );
    }

    /// The `chunking_config_and_output` shape: `min_length` on a string reached through an index
    /// and a further key — `chunks[0].content`. This was the last assertion type in the whole
    /// generated consumer suite that still fell through to the "no Swift renderer" skip.
    #[test]
    fn min_length_on_a_nested_string_renders_a_character_count_comparison() {
        let resolver = resolver_with_json_bridged_field("chunks");
        let mut out = String::new();

        let rendered = render_json_bridged_navigated_assertion(
            &mut out,
            &assertion("min_length", "results[0].chunks[0].content", Some(serde_json::json!(9))),
            &resolver,
            "result",
        );

        assert!(rendered, "min_length over a navigated field must be handled");
        assert!(
            out.contains("XCTAssertGreaterThanOrEqual(((") && out.contains("as? String) ?? \"\").count, 9)"),
            "must compare the decoded value's character count against the fixture's expected minimum, got:\n{out}"
        );
    }

    /// CONTROL: navigation succeeds (the path IS reachable) but this module deliberately has no
    /// renderer for `not_contains` over a decoded JSON value — the fallback must still emit a
    /// visible, correctly classified skip rather than silently dropping the assertion.
    #[test]
    fn an_assertion_type_with_no_renderer_still_emits_a_visible_skip() {
        let resolver = resolver_with_json_bridged_field("metadata");
        let mut out = String::new();

        let rendered = render_json_bridged_navigated_assertion(
            &mut out,
            &assertion(
                "not_contains",
                "results[0].metadata.output_format",
                Some(serde_json::json!("html")),
            ),
            &resolver,
            "result",
        );

        assert!(
            rendered,
            "a navigated-but-unrenderable assertion type must still write something"
        );
        assert!(
            out.contains("skipped:") && out.contains("navigated JSON-bridged field"),
            "must use the GeneratorGap wording, not silence, got:\n{out}"
        );
        assert!(
            out.contains("not_contains"),
            "the skip should name which assertion type has no renderer, got:\n{out}"
        );
    }

    /// CONTROL: a field that never steps past a JSON-bridged leaf at all must fall through
    /// untouched, so callers keep their own (unrelated) handling for it.
    #[test]
    fn a_non_bridged_field_is_left_untouched() {
        let resolver = resolver_with_json_bridged_field("metadata");
        let mut out = String::new();

        let rendered = render_json_bridged_navigated_assertion(
            &mut out,
            &assertion(
                "equals",
                "results[0].mime_type",
                Some(serde_json::json!("application/pdf")),
            ),
            &resolver,
            "result",
        );

        assert!(!rendered, "got:\n{out}");
        assert!(out.is_empty(), "got:\n{out}");
    }
}
