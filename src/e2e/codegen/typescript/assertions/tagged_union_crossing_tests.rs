//! End-to-end coverage for `render_assertion` on a fixture path that crosses a tagged-union
//! variant boundary alef used to refuse outright for TypeScript -- see
//! `field_refusal::refusal_line` and `FieldResolver::typescript_tagged_union_accessor` for the
//! reachability check this exercises through the real render path, not just the resolver.

use super::render_assertion;
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;
use std::collections::{HashMap, HashSet};

fn field(name: &str, ty: TypeRef) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        ..FieldDef::default()
    }
}

/// A `Metadata.format: Option<FormatMetadata>` field where `FormatMetadata` is internally
/// tagged (`#[serde(tag = "format_type")]`) and its `Html` variant wraps exactly one Named
/// type -- the real shape `results[0].metadata.format.html.title` reaches.
fn resolver_over_format_metadata() -> FieldResolver {
    let types = vec![TypeDef {
        name: "Metadata".to_string(),
        fields: vec![field(
            "format",
            TypeRef::Optional(Box::new(TypeRef::Named("FormatMetadata".to_string()))),
        )],
        ..TypeDef::default()
    }];
    let enums = vec![EnumDef {
        name: "FormatMetadata".to_string(),
        serde_tag: Some("format_type".to_string()),
        variants: vec![EnumVariant {
            name: "Html".to_string(),
            is_tuple: true,
            fields: vec![field("_0", TypeRef::Named("HtmlMetadata".to_string()))],
            ..EnumVariant::default()
        }],
        ..EnumDef::default()
    }];
    let method_calls: HashSet<String> = ["format.html".to_string()].into_iter().collect();
    FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &method_calls,
    )
    .with_ir_enum_map(FieldResolver::ir_enum_fields(&types, &enums), Some("Metadata".to_string()))
}

fn make_assertion(assertion_type: &str, field: &str, value: serde_json::Value) -> Assertion {
    Assertion {
        assertion_type: assertion_type.to_string(),
        field: Some(field.to_string()),
        value: Some(value),
        ..Assertion::default()
    }
}

/// The defect itself, for node: the crossing must render napi's real flattened field with
/// optional chaining, never a "skipped" comment.
#[test]
fn node_renders_the_flattened_napi_field_instead_of_skipping() {
    let resolver = resolver_over_format_metadata();
    let assertion = make_assertion(
        "equals",
        "format.html.title",
        serde_json::Value::String("Simple Table Test".to_string()),
    );
    let mut out = String::new();
    render_assertion(
        &mut out,
        &assertion,
        "result",
        &resolver,
        false,
        &HashMap::new(),
        "node",
        false,
        false,
        false,
    );
    assert!(!out.contains("skipped"), "got: {out}");
    assert!(
        out.contains("result.format.html?.title"),
        "expected the real flattened field with optional chaining, got: {out}"
    );
}

/// wasm's flattened `JsValue` payload has no variant segment at all -- the suffix reads
/// straight off the container.
#[test]
fn wasm_renders_the_flattened_serde_payload_instead_of_skipping() {
    let resolver = resolver_over_format_metadata();
    let assertion = make_assertion(
        "equals",
        "format.html.title",
        serde_json::Value::String("Simple Table Test".to_string()),
    );
    let mut out = String::new();
    render_assertion(
        &mut out,
        &assertion,
        "result",
        &resolver,
        false,
        &HashMap::new(),
        "wasm",
        false,
        false,
        false,
    );
    assert!(!out.contains("skipped"), "got: {out}");
    assert!(
        out.contains("result.format.title"),
        "expected the flattened payload with no variant segment, got: {out}"
    );
    assert!(
        !out.contains("result.format.html"),
        "wasm has no `html` member to spell, got: {out}"
    );
}

/// The control that stops "every declared crossing renders now" from passing: a variant shape
/// the IR does not resolve to a single Named payload (two inline fields) must still be skipped,
/// which is exactly the pre-existing, defensible refusal for a shape neither binding names.
#[test]
fn a_multi_field_variant_crossing_still_renders_a_skip() {
    let types = vec![TypeDef {
        name: "Metadata".to_string(),
        fields: vec![field("auth", TypeRef::Named("AuthConfig".to_string()))],
        ..TypeDef::default()
    }];
    let enums = vec![EnumDef {
        name: "AuthConfig".to_string(),
        serde_tag: Some("type".to_string()),
        variants: vec![EnumVariant {
            name: "Basic".to_string(),
            fields: vec![
                field("username", TypeRef::String),
                field("password", TypeRef::String),
            ],
            ..EnumVariant::default()
        }],
        ..EnumDef::default()
    }];
    let method_calls: HashSet<String> = ["auth.basic".to_string()].into_iter().collect();
    let resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &method_calls,
    )
    .with_ir_enum_map(FieldResolver::ir_enum_fields(&types, &enums), Some("Metadata".to_string()));
    let assertion = make_assertion(
        "equals",
        "auth.basic.username",
        serde_json::Value::String("alice".to_string()),
    );
    let mut out = String::new();
    render_assertion(
        &mut out,
        &assertion,
        "result",
        &resolver,
        false,
        &HashMap::new(),
        "node",
        false,
        false,
        false,
    );
    assert!(
        out.contains("skipped: field 'auth.basic.username' crosses a tagged-union variant boundary"),
        "got: {out}"
    );
}
