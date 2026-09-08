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

use super::accessors::materialise_vec_temporaries;
use super::leaf_shape::mixed_map_then_vec_traversal_skip;
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
    let leaf_expr_raw = field_resolver.accessor(&leaf_field, "swift", result_var);
    let local = json_nav_local_name(field, &assertion.assertion_type);
    // ~keep The receiver may read through a `RustVec` element, e.g.
    // `result.results()[0].chunks()` — swift-bridge's `Vec_<T>$get` hands back a pointer into
    // the Vec's own storage, and `result.results()` is a temporary. Inlined directly into the
    // `let _json_nav_… : Any? = …` expression below, Swift ARC can release that temporary
    // before `.chunks().toString()` finishes reading it, so the bytes dangle and
    // `RustStr.toString()`'s force-unwrap traps at runtime instead of failing the one assertion.
    // Routing through the same hoisting `accessors::materialise_vec_temporaries` uses for
    // ordinary (non-JSON-bridged) assertions ties the Vec's lifetime to a `let` bound in the
    // enclosing test method, exactly like the sibling `count`/`contains` renderers already do.
    let local_suffix = {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        field.hash(&mut hasher);
        assertion.assertion_type.hash(&mut hasher);
        format!(
            "{}_{:x}",
            assertion.assertion_type.replace(['-', '.'], "_"),
            hasher.finish() & 0xffff_ffff,
        )
    };
    // `None` is the same mixed map-then-vec hazard `assertions::render_assertion` refuses for
    // ordinary paths — a string-key subscript decodes to a plain Swift `String` with nothing a
    // further `RustVec` hoist can act on. Unreachable for every fixture resolvable today (the
    // leaf's own accessor never carries a map subscript ahead of `steps`), but refusing rather
    // than emitting broken Swift keeps that guarantee explicit instead of assumed. ~keep
    let Some((vec_setup, leaf_expr, _is_map_subscript)) = materialise_vec_temporaries(&leaf_expr_raw, &local_suffix)
    else {
        out.push_str(&mixed_map_then_vec_traversal_skip(field));
        return true;
    };
    // ~keep `accessor()` never puts a `?` on the leaf segment itself (see
    // `navigated_value_expr`'s own doc), so a leaf whose OWN getter returns
    // `Optional<RustString>` -- e.g. `Metadata::format` -- looks identical here to a
    // non-optional one; only an optional ANCESTOR shows up as `?.` in `leaf_expr`. Without this,
    // `Metadata::format().toString()` was emitted unwrapped against a getter that returns
    // `Optional<RustString>`, which does not compile: "value of optional type 'Optional
    // <RustString>' must be unwrapped to refer to member 'toString'".
    let leaf_is_optional = field_resolver
        .swift_leaf_getter_is_optional(&leaf_field)
        .unwrap_or(false);
    let value_expr = navigated_value_expr(&leaf_expr, leaf_is_optional, &steps);

    // No renderer for this assertion type: neither `vec_setup` nor `value_expr` is referenced
    // in the emitted Swift, so skip before writing either — an unused `let` triggers a Swift
    // "variable was never used" warning. ~keep
    let Some(body) = build_assertion_lines(assertion, &local) else {
        let _ = writeln!(
            out,
            "        // skipped: {}; assertion type '{}' has no Swift renderer over decoded JSON yet",
            FieldSkip::NavigatedJsonBridgedAssertionTypeNotSupportedInSwift.message(field),
            assertion.assertion_type
        );
        return true;
    };
    for line in &vec_setup {
        let _ = writeln!(out, "        {line}");
    }
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
/// for a missing value. `leaf_is_optional` covers the other source of optionality: the leaf's OWN
/// getter returning `Optional<RustString>`, which `leaf_expr` itself never signals (see the call
/// site's `~keep`) but needs exactly the same `?.`/coalesce treatment.
fn navigated_value_expr(leaf_expr: &str, leaf_is_optional: bool, steps: &[JsonNavStep]) -> String {
    let json_text_expr = if leaf_expr.contains("?.") || leaf_is_optional {
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

    /// Regression for the `ContractTests.swift` compile break: `Metadata::format` is a
    /// JSON-bridged leaf whose OWN swift-bridge getter returns `Optional<RustString>` (not an
    /// optional ANCESTOR in the chain), so `leaf_expr` never contains `"?."` and the old
    /// `contains("?.")`-only check emitted an unwrapped `.format().toString()` against an
    /// `Optional<RustString>` -- "value of optional type 'Optional<RustString>' must be
    /// unwrapped to refer to member 'toString'". Builds the map the same way
    /// `values::tests::tagged_union_field_is_navigable_end_to_end_from_real_ir` does, from real
    /// IR rather than a hand-set flag, so this pins the getter_optionality plumbing rather than a
    /// stub.
    #[test]
    fn optional_leaf_getter_with_no_optional_ancestor_still_unwraps() {
        use super::super::values::build_swift_first_class_map;
        use crate::core::config::e2e::{CallConfig, E2eConfig};
        use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};

        let format_metadata_enum = EnumDef {
            name: "FormatMetadata".to_string(),
            has_serde: true,
            variants: vec![EnumVariant {
                name: "Html".to_string(),
                fields: vec![FieldDef {
                    name: "title".to_string(),
                    ty: TypeRef::String,
                    optional: false,
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        let extracted_document = TypeDef {
            name: "ExtractedDocument".to_string(),
            fields: vec![FieldDef {
                name: "metadata".to_string(),
                ty: TypeRef::Named("Metadata".to_string()),
                optional: false,
                ..Default::default()
            }],
            has_serde: true,
            ..Default::default()
        };
        let metadata = TypeDef {
            name: "Metadata".to_string(),
            fields: vec![FieldDef {
                name: "format".to_string(),
                ty: TypeRef::Named("FormatMetadata".to_string()),
                optional: true,
                ..Default::default()
            }],
            has_serde: true,
            ..Default::default()
        };

        let mut map = build_swift_first_class_map(
            &[extracted_document, metadata],
            &[format_metadata_enum],
            &E2eConfig::default(),
            &CallConfig::default(),
        );
        map.root_type = Some("ExtractedDocument".to_string());
        assert_eq!(
            map.getter_is_optional("Metadata", "format"),
            Some(true),
            "precondition: Metadata::format's getter must be Optional<RustString>"
        );

        let resolver = FieldResolver::new_with_swift_first_class(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashMap::new(),
            map,
        );
        let mut out = String::new();

        let rendered = render_json_bridged_navigated_assertion(
            &mut out,
            &assertion(
                "equals",
                "metadata.format.html.title",
                Some(serde_json::json!("Simple Table")),
            ),
            &resolver,
            "result",
        );

        assert!(rendered, "got:\n{out}");
        assert!(
            !out.contains("metadata().format().toString()") && !out.contains("metadata.format().toString()"),
            "must not emit an unwrapped .toString() call on an Optional<RustString> getter, got:\n{out}"
        );
        assert!(
            out.contains("?.toString() ?? \"null\""),
            "an optional leaf getter must be unwrapped with the same ?./coalesce idiom used for \
             an optional ancestor, got:\n{out}"
        );
    }

    /// Regression for the `ContractTests.testChunkingConfigAndOutput` runtime crash: the
    /// receiver of a JSON-bridged navigated leaf can itself read through a `RustVec` element
    /// (`result.results()[0].chunks()`, `results: Vec<ChunkingResult>`). Inlined directly into
    /// the `let _json_nav_… : Any? = …` expression, swift-bridge's `Vec_<T>$get` pointer into
    /// `results()`'s temporary storage can dangle before `.chunks().toString()` reads it --
    /// `RustStr.toString()`'s force-unwrap then traps at runtime with "Unexpectedly found nil
    /// while unwrapping an Optional value" instead of the assertion simply failing. Built from
    /// real IR (`build_swift_first_class_map`), not a hand-set flag, so this pins the actual
    /// accessor `swift_build_accessor` produces for a real-Vec ancestor feeding a JSON-bridged
    /// leaf, mirroring `optional_leaf_getter_with_no_optional_ancestor_still_unwraps` above.
    #[test]
    fn receiver_through_a_real_vec_element_is_hoisted_not_inlined() {
        use super::super::values::build_swift_first_class_map;
        use crate::core::config::e2e::{CallConfig, E2eConfig};
        use crate::core::ir::{FieldDef, TypeDef, TypeRef};

        let extracted_document = TypeDef {
            name: "ExtractedDocument".to_string(),
            fields: vec![FieldDef {
                name: "results".to_string(),
                ty: TypeRef::Vec(Box::new(TypeRef::Named("ChunkingResult".to_string()))),
                optional: false,
                ..Default::default()
            }],
            has_serde: true,
            ..Default::default()
        };
        let chunking_result = TypeDef {
            name: "ChunkingResult".to_string(),
            fields: vec![
                FieldDef {
                    name: "chunks".to_string(),
                    ty: TypeRef::Vec(Box::new(TypeRef::String)),
                    // `Option<Vec<String>>` -- optional plus `Vec` -- is exactly the shape
                    // `field_needs_json_bridge` collapses to a whole-value JSON `RustString` getter.
                    optional: true,
                    ..Default::default()
                },
                FieldDef {
                    // `swift_first_class_field_supported` has no arm for `Map`, so this keeps
                    // `ChunkingResult` (and transitively `ExtractedDocument`) classified opaque
                    // -- a real `RustBridge` class reached through method calls, not a Codable
                    // struct reached by property syntax -- matching the actual `ChunkingResult`
                    // shape this regression is pinned against. ~keep
                    name: "extra".to_string(),
                    ty: TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
                    optional: false,
                    ..Default::default()
                },
            ],
            has_serde: true,
            ..Default::default()
        };

        let mut map = build_swift_first_class_map(
            &[extracted_document, chunking_result],
            &[],
            &E2eConfig::default(),
            &CallConfig::default(),
        );
        map.root_type = Some("ExtractedDocument".to_string());
        assert!(
            map.is_json_bridged_field_name("chunks"),
            "precondition: ChunkingResult::chunks must be JSON-bridged on the real IR map"
        );

        let resolver = FieldResolver::new_with_swift_first_class(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashMap::new(),
            map,
        );
        let mut out = String::new();

        let rendered = render_json_bridged_navigated_assertion(
            &mut out,
            &assertion("not_empty", "results[0].chunks[0]", None),
            &resolver,
            "result",
        );

        assert!(rendered, "got:\n{out}");
        assert!(
            !out.contains("()[0].chunks().toString()") && !out.contains("()[0]?.chunks().toString()"),
            "the RustVec receiver must be hoisted, never subscripted inline into the JSON-decode \
             expression, got:\n{out}"
        );
        assert!(
            out.lines().any(|line| line.trim_start().starts_with("let _vec_results_")),
            "must hoist `result.results()` into a local before subscripting it, got:\n{out}"
        );
    }

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
