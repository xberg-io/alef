//! Regression coverage for the three ways the Swift e2e generator used to emit an assertion that
//! could not fail, or drop one that could.
//!
//! Split into its own file rather than added to `swift/assertions.rs` (at its
//! `tests/file_size_baseline.txt` ceiling) or `swift/test_method.rs` (at the 1,000-line cap).
//!
//! All four fixtures below drive the real entry point, `render_test_method`, against a
//! `SwiftFirstClassMap` built by the production scanner (`values::build_swift_first_class_map`)
//! from the same IR, so the two oracles that disagree in production disagree here too:
//!
//! * `Report { section: Option<Section>, archive: Archive, title: Option<String> }`
//! * `Section { entries: Vec<Entry> }` — a genuine `RustVec` at the Swift surface
//! * `Archive { entries: Option<Vec<Entry>> }` — `field_needs_json_bridge` collapses this to one
//!   JSON `RustString`, so the NAME `entries` lands in the flat `json_bridged_field_names` set
//!
//! ~keep The two `entries` fields are the whole point. `leaf_is_json_bridged_via_swift_map` used
//! to answer from that flat, crate-wide, bare-leaf-name set, so `Archive`'s bridged `entries`
//! marked `Section`'s genuine `Vec<Entry>` bridged too. That dropped `section.entries` out of
//! `field_is_array` and into the presence-only `not_empty` arm — `XCTAssertTrue(<expr> != nil,
//! "expected non-empty value")`, which passes for an empty collection — and made `count_min`
//! count the characters of `.toString()`. Same failure the enum classifier has already been
//! taught to avoid (`ir_enum`'s module doc): a crate can declare one field name with two shapes
//! on two types, so the answer has to be type-driven.

use crate::core::config::{ResolvedCrateConfig, SwiftConfig};
use crate::core::ir::{FieldDef, FunctionDef, TypeDef, TypeRef};
use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::fixture::{Assertion, Fixture};
use std::collections::HashSet;

fn field(name: &str, ty: TypeRef, optional: bool) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        optional,
        ..FieldDef::default()
    }
}

/// Mirrors the extractor's own convention: `Option<T>` is unwrapped into `FieldDef::optional`,
/// leaving `ty` as the inner type (see `extract::extractor::helpers::fields::unwrap_optional`).
fn type_defs() -> Vec<TypeDef> {
    let entry = || TypeRef::Vec(Box::new(TypeRef::Named("Entry".to_string())));
    vec![
        TypeDef {
            name: "Report".to_string(),
            fields: vec![
                field("section", TypeRef::Named("Section".to_string()), true),
                field("archive", TypeRef::Named("Archive".to_string()), false),
                field("title", TypeRef::String, true),
            ],
            ..TypeDef::default()
        },
        TypeDef {
            name: "Section".to_string(),
            fields: vec![field("entries", entry(), false)],
            ..TypeDef::default()
        },
        TypeDef {
            name: "Archive".to_string(),
            fields: vec![field("entries", entry(), true)],
            ..TypeDef::default()
        },
        TypeDef {
            name: "Entry".to_string(),
            fields: vec![field("label", TypeRef::String, false)],
            ..TypeDef::default()
        },
    ]
}

fn functions() -> Vec<FunctionDef> {
    vec![FunctionDef {
        name: "analyze".to_string(),
        return_type: TypeRef::Named("Report".to_string()),
        ..FunctionDef::default()
    }]
}

fn result_fields() -> HashSet<String> {
    ["section", "archive", "title"]
        .into_iter()
        .map(str::to_string)
        .collect()
}

fn e2e_config() -> E2eConfig {
    E2eConfig {
        call: CallConfig {
            function: "analyze".to_string(),
            result_var: "result".to_string(),
            returns_result: true,
            result_fields: result_fields(),
            ..CallConfig::default()
        },
        result_fields: result_fields(),
        ..E2eConfig::default()
    }
}

fn render_with_config(assertion: Assertion, config: &ResolvedCrateConfig) -> String {
    let fixture = Fixture {
        id: "analyze_entries".to_string(),
        description: "entries reached through an optional parent".to_string(),
        assertions: vec![assertion],
        ..Fixture::default()
    };
    let e2e_config = e2e_config();
    let type_defs = type_defs();
    let functions = functions();
    let swift_first_class_map =
        super::values::build_swift_first_class_map(&type_defs, &[], &e2e_config, &e2e_config.call);
    let mut out = String::new();
    super::test_method::render_test_method(
        &mut out,
        &fixture,
        &e2e_config,
        "analyze",
        "result",
        &[],
        false,
        None,
        &swift_first_class_map,
        "SampleModule",
        config,
        &type_defs,
        &[],
        &functions,
        &[],
    );
    out
}

fn render(assertion: Assertion) -> String {
    render_with_config(assertion, &ResolvedCrateConfig::default())
}

fn assertion(assertion_type: &str, field: &str) -> Assertion {
    Assertion {
        assertion_type: assertion_type.to_string(),
        field: Some(field.to_string()),
        ..Assertion::default()
    }
}

fn count_min(field: &str, minimum: u64) -> Assertion {
    Assertion {
        value: Some(serde_json::json!(minimum)),
        ..assertion("count_min", field)
    }
}

fn config_excluding(entry: &str) -> ResolvedCrateConfig {
    ResolvedCrateConfig {
        name: "sample".into(),
        swift: Some(SwiftConfig {
            exclude_fields: vec![entry.to_string()],
            ..SwiftConfig::default()
        }),
        ..ResolvedCrateConfig::default()
    }
}

/// Defect 1. `section.entries` is a real `Vec<Entry>` reached through the optional parent
/// `section`, so `not_empty` must keep the discriminating emptiness check. Answering the
/// JSON-bridge question from the crate-wide bare-name set instead of `Section`'s own getter made
/// `field_is_array` false and degraded this to `!= nil` — true for an empty collection, and true
/// even for an absent one, since the bridged getter it was written for is non-optional.
#[test]
fn not_empty_on_a_collection_reached_through_an_optional_parent_stays_discriminating() {
    let out = render(assertion("not_empty", "section.entries"));
    assert!(
        out.contains("isEmpty == false"),
        "a genuine Vec must keep a real emptiness check, got:\n{out}"
    );
    assert!(
        !out.contains("!= nil"),
        "must not degrade to a non-nil check that passes for an empty collection, got:\n{out}"
    );
    assert!(
        !out.contains(".toString()"),
        "must not stringify a field `Section` declares as Vec<Entry>, got:\n{out}"
    );
}

/// Control for defect 1: a genuinely optional NON-collection keeps the `!= nil` check. If this
/// failed alongside the test above, the fix would be a blanket rewrite of the optional arm rather
/// than a correction to one classifier.
#[test]
fn not_empty_on_a_genuinely_optional_scalar_still_checks_non_nil() {
    let out = render(assertion("not_empty", "title"));
    assert!(
        out.contains("!= nil"),
        "an optional String's not_empty must keep the non-nil check, got:\n{out}"
    );
    assert!(
        !out.contains("isEmpty == false"),
        "a scalar must not take the collection branch, got:\n{out}"
    );
}

/// The half of defect 1 that survives the classifier fix: `archive.entries` really IS
/// JSON-bridged, so `field_is_array` is correctly false and a bare `!= nil`/`.isEmpty` check is
/// dishonest — `"[]"` and `"null"` are non-empty strings for exactly the empty collections the
/// fixture is ruling out.
///
/// Was `..._is_refused_not_faked`, asserting the registered skip. Refusal is no longer the honest
/// answer here either: `swift_json_bridged_count_expr` (`leaf_shape.rs`) decodes the bridged
/// `RustString` back into a JSON array and counts its real elements, so `not_empty` gets a check
/// that CAN fail — `count_on_a_genuinely_bridged_collection_decodes_elements_not_characters`
/// above is this test's `count_min`/`count_equals` sibling.
#[test]
fn not_empty_on_a_genuinely_bridged_collection_decodes_and_counts() {
    let out = render(assertion("not_empty", "archive.entries"));
    assert!(
        out.contains("JSONSerialization.jsonObject") && out.contains("XCTAssertGreaterThan("),
        "a bridged collection's not_empty must decode and count instead of being refused, got:\n{out}"
    );
    assert!(
        !out.contains("// skipped: field 'archive.entries' has no countable Swift leaf"),
        "the count recovery makes the old blanket skip obsolete for this shape, got:\n{out}"
    );
    assert!(
        !out.contains("!= nil") && !out.contains(".isEmpty") && !out.contains(".toString().count"),
        "must never emit a check that cannot fail, got:\n{out}"
    );
}

/// Defect 2. `count_min` must count the collection's ELEMENTS. The same bare-name bridge verdict
/// sent `swift_count_target` down its `.toString()` path, so the generated Swift compared the
/// character length of a JSON dump against the fixture's minimum — a comparison an empty
/// collection passes, because `"[]"` is two characters long.
#[test]
fn count_min_on_a_collection_counts_elements_not_characters() {
    let out = render(count_min("section.entries", 2));
    assert!(
        out.contains("XCTAssertGreaterThanOrEqual") && out.contains(".count"),
        "count_min must still emit a count comparison, got:\n{out}"
    );
    assert!(
        !out.contains(".toString()"),
        "must count the Vec's elements, not the characters of a stringified Vec, got:\n{out}"
    );
}

/// Control for defect 2: a real scalar `String` leaf still gets `.toString().count`, which is the
/// meaningful count for it. Proves the refusal is scoped to bridged collections rather than
/// disabling `.toString()` wholesale.
#[test]
fn count_min_on_a_string_field_still_counts_characters() {
    let out = render(count_min("title", 2));
    assert!(
        out.contains(".toString().count"),
        "a scalar String's count must stay a character count, got:\n{out}"
    );
}

/// Defect 3. `[languages.swift].exclude_fields = ["Archive.entries"]` excludes ONE field on ONE
/// type. The type-blind name fallback ran first and unconditionally, matched the bare segment
/// `entries` against the union of every excluded type's field set, and dropped every assertion
/// whose path merely contains that name — reporting `ExcludedFromSwiftBinding`, which reads as a
/// deliberate config decision rather than the misclassification it is. One entry was enough to
/// void an entire fixture category.
#[test]
fn an_exclusion_on_an_unrelated_type_does_not_drop_a_same_named_field() {
    let out = render_with_config(
        assertion("not_empty", "section.entries"),
        &config_excluding("Archive.entries"),
    );
    assert!(
        !out.contains("excluded from the Swift binding"),
        "excluding Archive.entries must not drop Section.entries, got:\n{out}"
    );
    assert!(
        out.contains("isEmpty == false"),
        "the assertion must still be emitted, got:\n{out}"
    );
}

/// Control for defect 3: excluding the field's REAL owner still drops it. Without this the fix
/// could have been "stop excluding anything" and this file would not notice.
#[test]
fn an_exclusion_on_the_real_owner_type_still_drops_the_field() {
    let out = render_with_config(
        assertion("not_empty", "section.entries"),
        &config_excluding("Section.entries"),
    );
    assert!(
        out.contains("// skipped: field 'section.entries' references a field or type excluded from the Swift binding"),
        "excluding Section.entries must still drop it, got:\n{out}"
    );
}

fn count_equals(field: &str, expected: u64) -> Assertion {
    Assertion {
        value: Some(serde_json::json!(expected)),
        ..assertion("count_equals", field)
    }
}

/// The half of defect 2 that nothing observed. `count_min`/`count_equals` on a leaf that really IS
/// JSON-bridged must never count the CHARACTERS of the JSON text — that silently compares the
/// length of `"[]"` (or any other serialized array) against the expected element count.
///
/// ~keep `count_min_on_a_collection_counts_elements_not_characters` above asserts on
/// `section.entries`, which the owner-type fix correctly classifies as a genuine `Vec` — so it
/// never reaches the bridged-leaf branch at all. Reverting that branch to
/// `Some("{expr}.toString()")` left the ENTIRE 10,436-test lib suite green, which is what a change
/// no test can distinguish from its absence looks like. A consumer found the live instance: a
/// `count_equals: 2` rendering as `...toolCalls().toString().count == 2`, comparing the length of
/// `"[]"` against 2 for an empty collection. This test anchors on `archive.entries` — the
/// fixture's genuinely bridged `Option<Vec<Entry>>` — so it exercises the branch the other one
/// cannot reach.
///
/// Was `..._is_refused_not_counted_as_characters`, asserting the registered skip comment. Refusal
/// is no longer the honest answer: `swift_json_bridged_count_expr` (`leaf_shape.rs`) now decodes
/// the bridged `RustString` back into a JSON array via `JSONSerialization` and asserts on the real
/// element count, so the field is no longer unspellable. What must still never happen — reading
/// `.count` on the bridged STRING itself, which answers a different question (character length,
/// not element count) — is exactly what the first assertion below still catches: it fails
/// unconditionally the moment anyone reintroduces `.toString().count` on this leaf, independent
/// of whether that reintroduction goes through the old bridged-leaf branch or a new one.
#[test]
fn count_on_a_genuinely_bridged_collection_decodes_elements_not_characters() {
    for (label, out) in [
        ("count_equals", render(count_equals("archive.entries", 2))),
        ("count_min", render(count_min("archive.entries", 2))),
    ] {
        assert!(
            !out.contains(".toString().count"),
            "{label} must not count the characters of a stringified collection, got:\n{out}"
        );
        assert!(
            out.contains("JSONSerialization.jsonObject(with: Data(result.archive().entries().toString().utf8))"),
            "{label} on a bridged leaf must decode it and count elements, got:\n{out}"
        );
        assert!(
            !out.contains("// skipped: field 'archive.entries' has no countable Swift leaf"),
            "{label} on a bridged collection leaf is no longer unspellable, got:\n{out}"
        );
    }
}

/// Control: the refusal is scoped to bridged COLLECTIONS. A scalar `String`'s character count is
/// the meaningful reading, so it must survive — otherwise the fix above could be "refuse every
/// count" and this file would not notice. ~keep
#[test]
fn count_equals_on_a_string_field_still_counts_characters() {
    let out = render(count_equals("title", 5));
    assert!(
        out.contains(".toString().count"),
        "a scalar String's count_equals must stay a character count, got:\n{out}"
    );
}
