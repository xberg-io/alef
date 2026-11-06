//! Regression tests for a JSON key serde omits entirely (`#[serde(skip_serializing_if = "...")]`)
//! on a field that is NOT itself `Option<T>` -- e.g. a required `Vec<T>` skipped via
//! `Vec::is_empty`. `alef`'s IR previously had no concept of "required but conditionally
//! absent from the wire", so the Zig e2e generator's JSON-tree accessor chain always emitted
//! `.object.get("key").?`, which panics at runtime for exactly the values that legitimately
//! triggered the skip.
//!
//! Split into its own file rather than added to `zig/assertions.rs`: that file is already
//! over the repo's 1,000-line cap (see `file-modularization` in CLAUDE.md), so new test
//! coverage goes into a fresh module instead of growing it. ~keep

use super::assertions::render_json_assertion;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;
use std::collections::{HashMap, HashSet};

fn resolver_with_wire_optional(field: &str) -> FieldResolver {
    let wire_optional: HashSet<String> = [field.to_string()].into_iter().collect();
    FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    )
    .with_wire_optional_fields(wire_optional)
}

fn empty_resolver() -> FieldResolver {
    FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    )
}

fn is_empty_assertion(field: &str) -> Assertion {
    Assertion {
        assertion_type: "is_empty".to_string(),
        field: Some(field.to_string()),
        ..Assertion::default()
    }
}

/// The exact shape that crashed for tslp's `data_extraction_json_empty_object` fixture:
/// `DataNode.children: Vec<DataNode>` carries
/// `#[serde(default, skip_serializing_if = "Vec::is_empty")]`, so `process("{}", ...)` omits
/// the `"children"` key from the emitted JSON entirely -- not `null`, absent. Before the fix,
/// the generated accessor chain was `result.object.get("data").?.object.get("children").?`,
/// which panics with "attempt to use null value" on `.get("children").?` because there is no
/// entry for that key at all. The fix must render a guarded lookup for the wire-optional
/// segment instead. ~keep
#[test]
fn wire_optional_leaf_key_is_not_force_unwrapped() {
    let resolver = resolver_with_wire_optional("children");
    let assertion = is_empty_assertion("data.children");
    let mut out = String::new();

    render_json_assertion(&mut out, &assertion, "result", &resolver, false);

    assert!(
        !out.contains(".object.get(\"children\").?"),
        "a wire-optional key must not be force-unwrapped with `.?`, which panics when serde \
         omitted the key entirely. Rendered:\n{out}"
    );
    let qualified_null = "std.json.Value{ .null = {} }";
    assert!(
        out.contains(&format!(
            "(result.object.get(\"data\").object.get(\"children\") orelse {qualified_null})"
        )) || out.contains(&format!(".object.get(\"children\") orelse {qualified_null}")),
        "a wire-optional key must be guarded with `orelse {qualified_null}` so a missing key \
         renders the same as a present `null` value. A bare `orelse .null` compiles only where \
         Zig can infer a result type, which a chained `.object.get(...)` access does not \
         provide. Rendered:\n{out}"
    );
}

/// Companion negative case: a field the resolver does NOT know is wire-optional must keep the
/// existing `.?` force-unwrap -- the fix must not blanket-guard every JSON key, only ones the
/// IR actually marked `serde_skip_serializing_if`.
#[test]
fn non_wire_optional_leaf_key_keeps_force_unwrap() {
    let resolver = empty_resolver();
    let assertion = is_empty_assertion("data.children");
    let mut out = String::new();

    render_json_assertion(&mut out, &assertion, "result", &resolver, false);

    assert!(
        out.contains(".object.get(\"children\").?"),
        "a field not marked wire-optional must keep the plain `.?` accessor. Rendered:\n{out}"
    );
    assert!(
        !out.contains("orelse std.json.Value{ .null = {} }"),
        "a field not marked wire-optional must not be guarded. Rendered:\n{out}"
    );
}

/// alef#899: adding `data: Option<String>` (skipped by serde) to one type must not guard a
/// REQUIRED `data` field on a different type. The anchored IR resolves the owning type along the
/// path and answers for that type; the bare-name set is only the fallback when it cannot.
#[test]
fn wire_optionality_is_resolved_on_the_owning_type() {
    use crate::core::ir::{FieldDef, TypeDef, TypeRef};

    let type_defs = vec![
        TypeDef {
            name: "Response".to_string(),
            fields: vec![
                FieldDef {
                    name: "thinking".to_string(),
                    ty: TypeRef::Named("ThinkingBlock".to_string()),
                    ..FieldDef::default()
                },
                FieldDef {
                    name: "payload".to_string(),
                    ty: TypeRef::Named("Payload".to_string()),
                    ..FieldDef::default()
                },
            ],
            ..TypeDef::default()
        },
        TypeDef {
            name: "ThinkingBlock".to_string(),
            fields: vec![FieldDef {
                name: "data".to_string(),
                ty: TypeRef::Optional(Box::new(TypeRef::String)),
                optional: true,
                serde_skip_serializing_if: true,
                ..FieldDef::default()
            }],
            ..TypeDef::default()
        },
        TypeDef {
            name: "Payload".to_string(),
            fields: vec![FieldDef {
                name: "data".to_string(),
                ty: TypeRef::Vec(Box::new(TypeRef::String)),
                serde_skip_serializing_if: false,
                ..FieldDef::default()
            }],
            ..TypeDef::default()
        },
    ];
    let resolver = empty_resolver()
        .with_ir_result_fields(
            FieldResolver::ir_result_field_facts(&type_defs, "zig"),
            Some("Response".to_string()),
        )
        // The bare-name set still contains `data` because ThinkingBlock skips it; the anchored
        // answer must win for the Payload path.
        .with_wire_optional_fields(FieldResolver::ir_wire_optional_fields(&type_defs));

    let required = super::json_access::json_path_expr_with(
        "result",
        "",
        "payload.data",
        super::json_access::Unwrap::Panic,
        &resolver,
    );
    assert!(
        required.contains(".object.get(\"data\").?"),
        "a required `data` must keep the force-unwrap even when another type skips a `data` \
         field. Rendered:\n{required}"
    );
    assert!(
        !required.contains("orelse"),
        "a required `data` must not be guarded by another type's skip. Rendered:\n{required}"
    );

    let skipped = super::json_access::json_path_expr_with(
        "result",
        "",
        "thinking.data",
        super::json_access::Unwrap::Panic,
        &resolver,
    );
    assert!(
        skipped.contains("orelse std.json.Value{ .null = {} }"),
        "the owner that actually skips `data` must still be guarded. Rendered:\n{skipped}"
    );
}
