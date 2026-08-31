//! Regression tests for `FieldResolver::ir_wire_optional_fields` -- the IR-derived fact that a
//! field's JSON key may be entirely absent from the wire format
//! (`#[serde(skip_serializing_if = "...")]`) even though the underlying Rust field is always
//! present. This is a genuinely different fact from `FieldResolver::ir_field_sets`'s `optional`
//! set, which tracks `Option<T>`-ness -- see `FieldDef::serde_skip_serializing_if`.
//!
//! Split into its own file rather than added to `field_access/tests.rs`: that file is already
//! over the repo's 1,000-line cap (see `file-modularization` in CLAUDE.md), so new test
//! coverage goes into a fresh module instead of growing it. ~keep

use super::FieldResolver;
use crate::core::ir::{FieldDef, TypeDef};

#[test]
fn a_required_vec_with_skip_serializing_if_is_wire_optional() {
    let type_def = TypeDef {
        name: "DataNode".to_string(),
        fields: vec![FieldDef {
            name: "children".to_string(),
            optional: false,
            serde_skip_serializing_if: true,
            serde_skip: false,
            ..FieldDef::default()
        }],
        ..TypeDef::default()
    };

    let wire_optional = FieldResolver::ir_wire_optional_fields(&[type_def]);
    assert!(wire_optional.contains("children"));
}

/// `ir_field_sets`'s `optional` set must NOT pick up a `serde_skip_serializing_if` field that
/// is not itself `Option<T>` -- conflating the two would make e.g. the Rust e2e backend emit
/// `.as_ref().unwrap()` against a plain `Vec<T>`, which does not compile.
#[test]
fn a_required_vec_with_skip_serializing_if_is_not_ir_optional() {
    let type_def = TypeDef {
        name: "DataNode".to_string(),
        fields: vec![FieldDef {
            name: "children".to_string(),
            optional: false,
            serde_skip_serializing_if: true,
            serde_skip: false,
            ..FieldDef::default()
        }],
        ..TypeDef::default()
    };

    let (_reachable, _excluded, optional) = FieldResolver::ir_field_sets(&[type_def]);
    assert!(
        !optional.contains("children"),
        "a required Vec<T> field must not be treated as Option<T>-optional just because it \
         carries skip_serializing_if"
    );
}

/// A field with neither `Option<T>` nor `skip_serializing_if` must not appear in either set.
#[test]
fn a_plain_required_field_is_neither_optional_nor_wire_optional() {
    let type_def = TypeDef {
        name: "DataNode".to_string(),
        fields: vec![FieldDef {
            name: "kind".to_string(),
            optional: false,
            serde_skip_serializing_if: false,
            serde_skip: false,
            ..FieldDef::default()
        }],
        ..TypeDef::default()
    };

    let wire_optional = FieldResolver::ir_wire_optional_fields(std::slice::from_ref(&type_def));
    assert!(!wire_optional.contains("kind"));
    let (_reachable, _excluded, optional) = FieldResolver::ir_field_sets(&[type_def]);
    assert!(!optional.contains("kind"));
}

/// `is_wire_optional_key` is a bare-key lookup (unlike `is_optional`, which resolves a full
/// dotted path): `wire_optional_fields` is IR-derived from bare field names with no notion of
/// nesting depth, so a resolver built from `ir_wire_optional_fields` must answer per key, not
/// per full path.
#[test]
fn is_wire_optional_key_matches_the_bare_key_regardless_of_nesting() {
    let type_def = TypeDef {
        name: "DataNode".to_string(),
        fields: vec![FieldDef {
            name: "children".to_string(),
            serde_skip_serializing_if: true,
            serde_skip: false,
            ..FieldDef::default()
        }],
        ..TypeDef::default()
    };
    let wire_optional = FieldResolver::ir_wire_optional_fields(&[type_def]);
    let resolver = FieldResolver::new(
        &std::collections::HashMap::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
    )
    .with_wire_optional_fields(wire_optional);

    assert!(resolver.is_wire_optional_key("children"));
    assert!(!resolver.is_wire_optional_key("data"));
}
