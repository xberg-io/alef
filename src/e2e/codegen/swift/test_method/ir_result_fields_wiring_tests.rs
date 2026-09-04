//! Regression coverage for `with_ir_result_fields` wiring in `render_test_method`'s per-call
//! `FieldResolver` construction, AND for the follow-on bug that wiring alone did not fix: two
//! independent "is this field a Vec" oracles inside swift codegen that can disagree.
//!
//! ~keep Split into its own file rather than added inline: `test_method.rs` sits within a handful
//! of lines of the file-size ratchet's 1,000-line cap (`tests/file_size_baseline.txt` carries no
//! entry for it, meaning it must stay under the cap, not just avoid growing past a grandfathered
//! ceiling), so new test coverage belongs in a sibling file rather than inline.
//!
//! Before the FIRST fix, the resolver `render_test_method` builds called `with_ir_collection_map`
//! and `with_anchored_optional_paths` but never `with_ir_result_fields` — the same gap `kotlin`'s
//! `call_field_resolver.rs` documents and wires around, and `dart`/`php` also had independently.
//!
//! Wiring that in alone was NOT sufficient, because `count_min`/`count_equals`/`min_length`/
//! `max_length` never consult `is_optional`/`is_array` at all for their "is this a Vec" decision
//! — they route through `swift_array_count_expr` → `swift_count_target` (`swift/accessors.rs`),
//! which used to ask ONLY `FieldResolver::leaf_is_vec_via_swift_map` (backed by
//! `SwiftFirstClassMap`, populated by scanning the REAL swift-bridge output). A field the IR
//! proves is a genuine `Vec<T>`, but that the `SwiftFirstClassMap` has no data for at all (e.g.
//! reached only through an opaque owner type the scan never recorded field-level Vec-ness for),
//! read as "not a vec" — `leaf_is_vec_via_swift_map`'s own doc calls this a "best-effort"
//! bare-leaf check, not an authoritative one — and `swift_count_target` wrapped it with
//! `.toString()`, which COMPILES (RustString has `.toString()` and the result has `.count`) but
//! counts the CHARACTERS of a Rust `Debug`-style dump of the vec, not the element count. Silently
//! wrong, not a build failure. Fixed by adding `is_array`/`is_collection_root` as a fallback
//! inside `swift_count_target` itself, consulted only when the Swift map has no vec/bridge
//! opinion at all — see that function's doc comment.
//!
//! The inverse direction is the ORIGINAL reported CI failure: a field that is BOTH optional (per
//! the IR) AND positively recorded by the swift-bridge scan as JSON-bridged
//! (`SwiftFirstClassMap::json_bridged_field_names` — a real scalar `RustString` at the Swift
//! surface, e.g. because the element type doesn't cleanly bridge) while the IR also proves it a
//! `Vec<T>` at the Rust level. `not_empty`'s `field_is_array && field_is_optional` arm
//! (`swift/assertions.rs`) used to trust the IR's `is_array`/`is_collection_root` alone and emit
//! `{field_expr}?.isEmpty == false` directly against the bridged leaf — `.isEmpty` does not exist
//! on `RustString` (`value of type 'RustString' has no member 'isEmpty'`, the exact reported
//! error). Fixed by excluding a positively JSON-bridged field from `field_is_array` outright.
//!
//! ~keep That exclusion was originally allowed to fall through to the plain `field_is_optional`
//! arm, which emits `XCTAssertTrue(<expr> != nil, "expected non-empty value")`. It compiles, and
//! that was mistaken for correctness: a JSON-bridged getter is declared NON-optional, so the
//! comparison is a tautology Swift only warns about — a `not_empty` assertion that passes for an
//! empty collection, wearing a message claiming it checked. `leaf_shape::
//! unspellable_collection_emptiness_skip` now refuses that arm outright for a leaf every
//! collection oracle calls a collection, and the test below pins the refusal rather than the
//! tautology.
//!
//! Both bugs are the same root: two independent "is this a Vec" oracles that can each be
//! consulted alone, instead of a positive JSON-bridge fact winning over the IR fact, and the IR
//! fact being asked at all when the Swift-specific map has no opinion either way.

use super::render_test_method;
use crate::core::ir::{FieldDef, FunctionDef, TypeDef, TypeRef};
use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::field_access::SwiftFirstClassMap;
use crate::e2e::fixture::{Assertion, Fixture};
use std::collections::HashSet;

/// `Envelope { results: Vec<Document> }`, `Document { chunks: Option<Vec<Chunk>> }` — the same
/// envelope-projection shape used by the sibling rust and `is_array` IR-fallback regressions, so
/// all three pin the identical bug from each backend's own vantage point.
fn envelope_document_type_defs() -> Vec<TypeDef> {
    vec![
        TypeDef {
            name: "Envelope".to_string(),
            fields: vec![FieldDef {
                name: "results".to_string(),
                ty: TypeRef::Vec(Box::new(TypeRef::Named("Document".to_string()))),
                ..FieldDef::default()
            }],
            ..TypeDef::default()
        },
        TypeDef {
            name: "Document".to_string(),
            fields: vec![FieldDef {
                name: "chunks".to_string(),
                ty: TypeRef::Vec(Box::new(TypeRef::Named("Chunk".to_string()))),
                optional: true,
                ..FieldDef::default()
            }],
            ..TypeDef::default()
        },
    ]
}

fn envelope_document_functions() -> Vec<FunctionDef> {
    vec![FunctionDef {
        name: "get_report".to_string(),
        return_type: TypeRef::Named("Envelope".to_string()),
        ..FunctionDef::default()
    }]
}

fn render(assertions: Vec<Assertion>, swift_first_class_map: &SwiftFirstClassMap) -> String {
    let fixture = Fixture {
        id: "get_report_chunks".to_string(),
        description: "swift report with an IR-only-optional chunks collection".to_string(),
        assertions,
        ..Fixture::default()
    };
    let e2e_config = E2eConfig {
        call: CallConfig {
            function: "get_report".to_string(),
            result_var: "result".to_string(),
            returns_result: true,
            result_fields: HashSet::from(["results".to_string()]),
            ..CallConfig::default()
        },
        ..E2eConfig::default()
    };
    let mut out = String::new();
    render_test_method(
        &mut out,
        &fixture,
        &e2e_config,
        "get_report",
        "result",
        &[],
        false,
        None,
        swift_first_class_map,
        "SampleModule",
        &crate::core::config::ResolvedCrateConfig::default(),
        &envelope_document_type_defs(),
        &[],
        &envelope_document_functions(),
        &[],
    );
    out
}

/// The confirmed defect (silent-wrong direction): a `count_min` assertion against
/// `results[0].chunks` — the IR proves it optional and `Vec`-typed, but `SwiftFirstClassMap` has
/// NO data for it at all (a default/empty map, matching a field reached only through an opaque
/// owner type the swift-bridge scan never recorded). The generated Swift must count the VEC's
/// elements via the optional-safe `?.count ?? 0` form, never stringify the vec and count
/// characters.
///
/// Pinning the exact tail rather than a loose substring: the earlier version of this test
/// asserted only `out.contains("?.count ?? 0")`, which is satisfiable by unrelated output and
/// gave no evidence about what actually followed `.chunks()`. The real pre-fix output was
/// `_vec_results_count_min_<hash>[0].chunks().toString().count` (the `<hash>` suffix
/// `materialise_vec_temporaries` derives is not reproducible by hand in this test), so this pins
/// the load-bearing, hash-independent SUFFIX — `[0].chunks()?.count ?? 0)` — and separately
/// forbids `.toString()` from appearing anywhere in the method at all.
///
/// Revert symptom: reverting `swift_count_target`'s IR fallback (`is_array`/`is_collection_root`)
/// in `swift/accessors.rs` makes this fail two ways — the `[0].chunks()?.count ?? 0)` suffix is
/// absent, and `.toString()` is present (as `.chunks().toString().count`) — because
/// `leaf_is_vec_via_swift_map` alone answers `false` for a field the empty `SwiftFirstClassMap`
/// has no data for.
#[test]
fn count_min_on_ir_only_optional_collection_leaf_counts_the_vec_not_a_debug_string() {
    let out = render(
        vec![Assertion {
            assertion_type: "count_min".to_string(),
            field: Some("results[0].chunks".to_string()),
            value: Some(serde_json::json!(2)),
            ..Default::default()
        }],
        &SwiftFirstClassMap::default(),
    );

    assert!(
        out.contains("[0].chunks()?.count ?? 0)"),
        "must emit the optional-safe count form against the Vec, not a stringified leaf; got:\n{out}"
    );
    assert!(
        !out.contains(".toString()"),
        "must not stringify a field the IR proves is a genuine Vec<Chunk>; got:\n{out}"
    );
}

/// The confirmed defect (compile-failure direction, the ORIGINAL reported CI shape): a
/// `not_empty` assertion against a field that is BOTH optional (per the IR) AND positively
/// recorded by the swift-bridge scan as JSON-bridged (`json_bridged_field_names`) — a real
/// `RustString` at the Swift surface regardless of what the IR says about its Rust-level shape.
///
/// `field_is_array` is correctly `false` for such a leaf (the Swift surface really is a string,
/// so `.isEmpty` on it does not compile) — but the presence-only `!= nil` fallthrough this used
/// to pin is a check that CANNOT FAIL: a JSON-bridged getter is non-optional, and the bridged
/// text is `"[]"`/`"null"` for exactly the empty collections `not_empty` exists to rule out.
///
/// `leaf_shape::swift_json_bridged_count_expr` now decodes the bridged JSON text instead of
/// refusing, so the honest output is a real (fallible) count check, not the registered
/// `CountOnJsonBridgedLeafInSwift` skip — this is that recovery's `not_empty` counterpart to the
/// `count_min`/`count_equals` case `count_min_on_ir_only_optional_collection_leaf_...` covers.
///
/// Revert symptom, two directions. Reverting `field_is_array`'s JSON-bridge guard in
/// `swift/assertions.rs` puts `.chunks()?.isEmpty == false` back — the literal `value of type
/// 'RustString' has no member 'isEmpty'` compile failure from the original CI report. Reverting
/// the `swift_json_bridged_count_expr` recovery in the `not_empty` arm puts `.chunks() != nil`
/// back — the unfailable green assertion this test exists to keep out.
#[test]
fn not_empty_on_optional_json_bridged_collection_leaf_decodes_and_counts() {
    let swift_first_class_map = SwiftFirstClassMap {
        json_bridged_field_names: HashSet::from(["chunks".to_string()]),
        ..SwiftFirstClassMap::default()
    };
    let out = render(
        vec![Assertion {
            assertion_type: "not_empty".to_string(),
            field: Some("results[0].chunks".to_string()),
            value: Some(serde_json::json!(true)),
            ..Default::default()
        }],
        &swift_first_class_map,
    );

    assert!(
        out.contains("JSONSerialization.jsonObject") && out.contains("XCTAssertGreaterThan("),
        "a JSON-bridged collection leaf must decode and count instead of being refused; got:\n{out}"
    );
    assert!(
        !out.contains("// skipped: field 'results[0].chunks' has no countable Swift leaf"),
        "the count recovery makes the old blanket skip obsolete for this shape; got:\n{out}"
    );
    assert!(
        !out.contains("!= nil"),
        "must never emit a non-nil check against a non-optional bridged getter: it cannot fail, \
         and it wears the \"expected non-empty value\" message; got:\n{out}"
    );
    assert!(
        !out.contains(".isEmpty"),
        "must never call .isEmpty on a JSON-bridged RustString leaf; got:\n{out}"
    );
}
