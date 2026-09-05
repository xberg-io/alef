//! Reproduces a defect verified in a downstream release: a docs fixture whose field path
//! carries a virtual namespace label (`interaction.action_results`, grouping assertions under a
//! label the emitted result has no member for -- see `FieldResolver::namespace_stripped_path`)
//! over an `Option<Vec<T>>` field emitted an unguarded TypeScript accessor, `TS18048` under
//! `strict`. Kept separate from `anchored_result_facts_tests.rs`: that file covers whether the
//! anchored IR answers optionality correctly for a BARE field name; this covers whether that
//! answer survives being looked up under a NAMESPACE-PREFIXED fixture path.

use super::*;
use crate::core::config::e2e::CallConfig;
use crate::core::ir::{FieldDef, FunctionDef, TypeDef, TypeRef};

fn field(name: &str, ty: TypeRef, optional: bool) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        optional,
        ..FieldDef::default()
    }
}

fn config() -> E2eConfig {
    E2eConfig {
        call: CallConfig {
            function: "interact".into(),
            result_var: "result".into(),
            ..CallConfig::default()
        },
        // `namespace_stripped_path` only strips a virtual label once the consumer has declared
        // its real top-level result fields -- see that method's doc comment for why this must
        // stay opt-in. ~keep
        result_fields: ["action_results".to_string()].into_iter().collect(),
        ..E2eConfig::default()
    }
}

fn type_defs() -> Vec<TypeDef> {
    vec![
        TypeDef {
            name: "InteractionResult".to_string(),
            fields: vec![field(
                "action_results",
                TypeRef::Optional(Box::new(TypeRef::Vec(Box::new(TypeRef::Named(
                    "ActionResult".to_string(),
                ))))),
                true,
            )],
            ..TypeDef::default()
        },
        TypeDef {
            name: "ActionResult".to_string(),
            fields: vec![field("action_type", TypeRef::String, false)],
            ..TypeDef::default()
        },
        // A same-named, non-optional twin on an unrelated type. `FieldResolver::ir_field_sets`
        // only calls a bare name optional when EVERY declaration of it across the whole IR is
        // `Option<T>` (see `anchored_result_facts_tests.rs`), so without this twin the bare-name
        // vote alone already gets `action_results` right and the namespace-prefix defect below
        // never gets exercised -- the anchored, root-specific answer has to be the one doing the
        // work, exactly as it does for the real `InteractionResult` shape. ~keep
        TypeDef {
            name: "UnrelatedBatch".to_string(),
            fields: vec![field(
                "action_results",
                TypeRef::Vec(Box::new(TypeRef::Named("ActionResult".to_string()))),
                false,
            )],
            ..TypeDef::default()
        },
    ]
}

fn returning(type_name: &str) -> Vec<FunctionDef> {
    vec![FunctionDef {
        name: "interact".to_string(),
        return_type: TypeRef::Named(type_name.to_string()),
        ..FunctionDef::default()
    }]
}

/// The exact shape that shipped broken: a fixture author writes `docs.shows` under a virtual
/// `interaction.` namespace label (so several assertions on one call's many side effects can be
/// grouped in generated docs without inventing a struct field the result does not have), naming
/// an array field the call's result type genuinely declares `Option<Vec<T>>`. Only the STRIPPED
/// spelling (`action_results`) is a real member; the label itself resolves nowhere. The for-loop
/// must therefore fall back with `?? []`, exactly as it would for the same field shown without
/// the label. ~keep
#[test]
fn an_iterate_over_a_namespace_prefixed_optional_array_field_gets_the_nullish_fallback() {
    let fixture = docs_fixture_with_presentation(FixtureDocsOperation::Iterate {
        path: "interaction.action_results".into(),
        item: "action".into(),
        fields: vec!["action_type".into()],
        display: true,
        optional: false,
    });

    let operations = resolve(
        &fixture,
        &config(),
        "node",
        &type_defs(),
        &[],
        &returning("InteractionResult"),
    );

    assert_eq!(
        operations.len(),
        1,
        "expected exactly one iterate operation: {operations:?}"
    );
    assert!(
        operations[0].optional,
        "the anchored IR says `action_results` is `Option<Vec<T>>`; the namespace label in front \
         of it must not hide that from the `?? []` guard: {operations:?}"
    );
}

/// The `show` counterpart: an element access one segment further into the same namespace-
/// labelled optional array must render `?.[0]`, not a bare `[0]` that dereferences the possibly-
/// `undefined` array directly.
#[test]
fn a_show_into_an_element_of_a_namespace_prefixed_optional_array_field_gets_optional_chaining() {
    let fixture = docs_fixture_with_presentation(FixtureDocsOperation::Show {
        path: "interaction.action_results[0].action_type".into(),
        display: false,
    });

    let operations = resolve(
        &fixture,
        &config(),
        "node",
        &type_defs(),
        &[],
        &returning("InteractionResult"),
    );

    assert_eq!(
        operations.iter().map(|o| o.expression.as_str()).collect::<Vec<_>>(),
        vec!["result.actionResults?.[0]?.actionType"],
        "the array itself is optional, so both the index and the field past it must chain \
         through `?.`"
    );
}

/// `with_anchored_optional_paths` carries no language parameter at all -- it populates the one
/// `optional_fields` set every per-language renderer in `render_relative_to` reads from, node's
/// included. Proves the fix is not TypeScript-specific: the same namespace-labelled path crossing
/// the same optional array, rendered for python instead of node, must also gain python's own
/// optional guard (a conditional expression -- see `render_python_with_optionals_from_owner`'s
/// `crossings` list), not just node's `?.`. ~keep
#[test]
fn a_show_into_an_element_of_the_same_namespace_prefixed_optional_field_is_guarded_for_python_too() {
    let fixture = docs_fixture_with_presentation(FixtureDocsOperation::Show {
        path: "interaction.action_results[0].action_type".into(),
        display: false,
    });

    let operations = resolve(
        &fixture,
        &config(),
        "python",
        &type_defs(),
        &[],
        &returning("InteractionResult"),
    );

    assert_eq!(
        operations.iter().map(|o| o.expression.as_str()).collect::<Vec<_>>(),
        vec!["(result.action_results[0].action_type if result.action_results else None)"],
        "python's own conditional-expression guard must also fire once the anchored path is \
         looked up under its stripped spelling: {operations:?}"
    );
}

fn docs_fixture_with_presentation(operation: FixtureDocsOperation) -> Fixture {
    serde_json::from_value(serde_json::json!({
        "id": "interact_action_sequence",
        "description": "Run an interaction and inspect its action results",
        "input": {},
        "docs": {
            "topic": "interaction",
            "stem": "action-sequence",
            "presentation": {
                "operations": [operation]
            }
        }
    }))
    .expect("fixture must parse")
}
