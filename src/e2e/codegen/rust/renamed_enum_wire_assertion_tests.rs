//! Regression coverage for `equals` assertions on a serde-RENAMED enum variant in the Rust
//! e2e generator.
//!
//! Split into its own file rather than added to `rust/assertion_helpers.rs`, which is already
//! past the 800-line "split before adding behaviour" mark in `file-modularization`. ~keep
//!
//! The defect: the enum branch of `render_equals_assertion` stringifies with `format!("{:?}",
//! ..)` because `Debug` is the only trait an arbitrary IR enum is guaranteed to implement.
//! `Debug` renders the RUST identifier. A fixture's `expected` records the SERDE WIRE value.
//! Those two spellings are the same string only while no `#[serde(rename)]` /
//! `#[serde(rename_all)]` applies to the variant. Under a rename they diverge, and the emitted
//! `assert_eq!` compared a Rust identifier against a wire value: it failed for a perfectly
//! correct result, or — if a fixture author changed the fixture to the Rust spelling to make it
//! green — it passed while never exercising the wire surface the rename exists to define.

use std::collections::{HashMap, HashSet};

use super::assertion_helpers::render_equals_assertion;
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;

/// `Result { kind: NodeKind }`, where `NodeKind` renames one variant onto a different wire
/// spelling (`KeyValue` -> `"key-value"`) and leaves the other on its identifier (`Plain`).
/// Both variants live on the same enum so the renamed and unrenamed lookups run against
/// identical resolver state and only the fixture value differs between cases.
fn renamed_enum_resolver() -> FieldResolver {
    let type_defs = vec![TypeDef {
        name: "Result".to_string(),
        fields: vec![FieldDef {
            name: "kind".to_string(),
            ty: TypeRef::Named("NodeKind".to_string()),
            ..FieldDef::default()
        }],
        ..TypeDef::default()
    }];
    let enums = vec![EnumDef {
        name: "NodeKind".to_string(),
        variants: vec![
            EnumVariant {
                name: "KeyValue".to_string(),
                serde_rename: Some("key-value".to_string()),
                ..EnumVariant::default()
            },
            EnumVariant {
                name: "Plain".to_string(),
                ..EnumVariant::default()
            },
        ],
        ..EnumDef::default()
    }];
    let ir_enum_map = FieldResolver::ir_enum_fields(&type_defs, &enums);
    FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    )
    .with_ir_enum_map(ir_enum_map, Some("Result".to_string()))
}

fn render_kind_equals(resolver: &FieldResolver, fixture_value: &str) -> String {
    let assertion = Assertion {
        assertion_type: "equals".to_string(),
        field: Some("kind".to_string()),
        value: Some(serde_json::Value::String(fixture_value.to_string())),
        ..Assertion::default()
    };
    let mut out = String::new();
    render_equals_assertion(&mut out, &assertion, "result.kind", false, resolver);
    out
}

fn render_kind_equals_pre_unwrapped(resolver: &FieldResolver, fixture_value: &str) -> String {
    let assertion = Assertion {
        assertion_type: "equals".to_string(),
        field: Some("kind".to_string()),
        value: Some(serde_json::Value::String(fixture_value.to_string())),
        ..Assertion::default()
    };
    let mut out = String::new();
    // is_unwrapped=true: simulates the call-site unwrap pass already having produced a
    // `String` local (`FieldResolver::rust_unwrap_binding`'s optional-scalar branch,
    // `.as_ref().map(|v| v.to_string()).unwrap_or_default()`) named `_kind`, holding the
    // enum's own `Display` rendering -- which for a serde-derived `Display` (the documented
    // `FinishReason` convention) is the WIRE spelling, not the Rust identifier.
    render_equals_assertion(&mut out, &assertion, "_kind", true, resolver);
    out
}

/// The renamed variant's fixture value must be reconciled onto the surface the emitted
/// expression actually renders, while every value that is NOT a renamed variant of that enum
/// passes through byte-for-byte.
#[test]
fn render_equals_assertion_renamed_enum_variant_compares_wire_value() {
    let resolver = renamed_enum_resolver();

    let renamed = render_kind_equals(&resolver, "key-value");
    assert_eq!(
        renamed, "    assert_eq!(format!(\"{:?}\", result.kind), r#\"KeyValue\"#, \"equals assertion failed\");\n",
        "the renamed variant's wire value must be reconciled to its Rust identifier, got: {renamed}"
    );

    // Control 1: the unrenamed sibling variant on the same enum passes through untouched,
    // proving the translation is keyed on an actual rename rather than applied to every enum
    // field indiscriminately.
    let unrenamed = render_kind_equals(&resolver, "Plain");
    assert_eq!(
        unrenamed, "    assert_eq!(format!(\"{:?}\", result.kind), r#\"Plain\"#, \"equals assertion failed\");\n",
        "an unrenamed variant must keep its fixture literal verbatim, got: {unrenamed}"
    );

    // Control 2: a fixture value naming no variant of this enum is left alone, so a genuinely
    // wrong expectation still generates a failing assertion instead of being rewritten into a
    // passing one.
    let unknown = render_kind_equals(&resolver, "not-a-variant");
    assert_eq!(
        unknown, "    assert_eq!(format!(\"{:?}\", result.kind), r#\"not-a-variant\"#, \"equals assertion failed\");\n",
        "an unrecognized fixture value must not be rewritten, got: {unknown}"
    );

    // Control 3: the three emissions stay mutually distinguishable. Were the translation ever
    // to collapse to a constant, all three would render identically and a real wire-value
    // regression would pass unnoticed.
    assert_ne!(renamed, unrenamed);
    assert_ne!(renamed, unknown);
}

/// Without the IR (no `with_ir_enum_map`, i.e. a resolver that only knows the field is an enum
/// from the hand-maintained `fields_enum` config) there is no rename to consult, so the fixture
/// literal must survive untranslated. This pins the "purely additive" claim: the new lookup can
/// only change output where the IR positively resolves a renamed variant.
#[test]
fn config_only_enum_classification_leaves_the_fixture_literal_untranslated() {
    let resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    )
    .with_enum_fields(HashSet::from(["kind".to_string()]));

    let out = render_kind_equals(&resolver, "key-value");
    assert_eq!(
        out, "    assert_eq!(format!(\"{:?}\", result.kind), r#\"key-value\"#, \"equals assertion failed\");\n",
        "no IR means no rename knowledge; the literal must be emitted verbatim, got: {out}"
    );
}

/// THE REGRESSION for the CI-confirmed defect on the PRE-UNWRAPPED path (a real downstream crate,
/// `FinishReason`/`ContentFilter`, CI run 33482291337): when the call-site unwrap pass has
/// already produced a `String` local holding the enum's `Display` rendering (which, per
/// `FieldResolver::rust_unwrap_binding`'s own doc, is the serde WIRE spelling for a
/// serde-derived `Display` impl like `FinishReason`'s), the emitted assertion must compare BOTH
/// operands on that SAME wire surface -- not translate `expected` to the Rust identifier
/// (`renamed_variant_expected`'s job for the `format!("{:?}", ..)` paths) while the expression
/// operand is already the wire string. Checking the FULL line (both operands) is deliberate:
/// a text-only check for the absence of `format!("{:?}"` passed while the two operands still
/// disagreed by case (`_kind` == `"key-value"` vs a translated `KeyValue`), which is the exact
/// gap that let this ship. ~keep
#[test]
fn render_equals_assertion_pre_unwrapped_renamed_enum_variant_compares_wire_value() {
    let resolver = renamed_enum_resolver();

    let renamed = render_kind_equals_pre_unwrapped(&resolver, "key-value");
    assert_eq!(
        renamed, "    assert_eq!(_kind, r#\"key-value\"#, \"equals assertion failed\");\n",
        "the pre-unwrapped local already holds the WIRE spelling; `expected` must NOT be \
         translated to the Rust identifier here, got: {renamed}"
    );
    assert!(
        !renamed.contains("format!(\"{:?}\""),
        "must not Debug-format an already-stringified Display local; got: {renamed}"
    );

    // Control: the unrenamed sibling variant is unaffected either way.
    let unrenamed = render_kind_equals_pre_unwrapped(&resolver, "Plain");
    assert_eq!(
        unrenamed, "    assert_eq!(_kind, r#\"Plain\"#, \"equals assertion failed\");\n",
        "got: {unrenamed}"
    );
}
