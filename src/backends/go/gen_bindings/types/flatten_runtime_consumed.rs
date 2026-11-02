use std::collections::BTreeMap;

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

use super::schema::{OwnedDefaultedChild, OwnedRequiredChild};
use super::{TypeDef, TypeRef, WireChoice, field, flattened, run_contract, structure, with_regular};

#[derive(Deserialize, Serialize)]
struct ParentOwned<T> {
    name: String,
    #[serde(flatten)]
    payload: Option<T>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    regular: Option<WireChoice>,
}

#[derive(Deserialize, Serialize)]
struct SiblingOwned {
    #[serde(flatten)]
    earlier: OwnedRequiredChild,
    #[serde(flatten)]
    payload: Option<OwnedRequiredChild>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    regular: Option<WireChoice>,
}

pub(super) fn oracle_cases() -> BTreeMap<String, Value> {
    let mut cases = BTreeMap::new();
    for mixed in [false, true] {
        let mode = if mixed { "mixed" } else { "only" };
        let mut input = json!({"name": "before"});
        if mixed {
            input["regular"] = json!({"kind": "live", "name": "regular"});
        }
        cases.insert(
            format!("required_{mode}"),
            parent_case::<OwnedRequiredChild>(&input, None, |child| &child.name),
        );
        cases.insert(
            format!("defaulted_{mode}"),
            parent_case::<OwnedDefaultedChild>(&input, Some(""), |child| &child.name),
        );
        let core: SiblingOwned = serde_json::from_value(input.clone()).expect("valid earlier-owned key input");
        assert_eq!(core.earlier.name, "before");
        assert!(core.payload.is_none(), "the earlier flattened sibling consumes name");
        let expected = serde_json::to_value(core).expect("serialize sibling ownership oracle");
        assert_eq!(expected["name"], "before");
        cases.insert(
            format!("sibling_{mode}"),
            json!({"input": input, "expected": expected, "present": false, "child_name": null}),
        );
    }
    assert_eq!(cases.len(), 6, "four parent and two sibling ownership oracles");
    cases
}

fn parent_case<T: DeserializeOwned + Serialize>(
    input: &Value,
    child_name: Option<&str>,
    name_of: fn(&T) -> &str,
) -> Value {
    let mut core: ParentOwned<T> = serde_json::from_value(input.clone()).expect("valid parent-owned key input");
    assert_eq!(core.name, "before");
    let present = core.payload.is_some();
    assert_eq!(present, child_name.is_some());
    assert_eq!(core.payload.as_ref().map(name_of), child_name);
    let expected = serde_json::to_value(&core).expect("serialize decoded parent ownership oracle");
    assert_eq!(expected["name"], child_name.unwrap_or("before"));
    core.name = "after".into();
    let mutated = serde_json::to_value(&core).expect("serialize mutated parent ownership oracle");
    assert_eq!(mutated["name"], child_name.unwrap_or("after"));
    json!({"input": input, "expected": expected, "mutated": mutated, "present": present, "child_name": child_name})
}

pub(super) fn parent_types() -> Vec<TypeDef> {
    let mut types = Vec::new();
    for mixed in [false, true] {
        let mode = if mixed { "Mixed" } else { "Only" };
        for (model, child) in [("Required", "OwnedRequiredChild"), ("Defaulted", "OwnedDefaultedChild")] {
            types.push(structure(
                &format!("Consumed{model}{mode}"),
                with_regular(vec![field("name", TypeRef::String), payload(child)], mixed),
            ));
        }
        types.push(structure(
            &format!("ConsumedSibling{mode}"),
            with_regular(
                vec![
                    flattened("earlier", TypeRef::Named("OwnedRequiredChild".into())),
                    payload("OwnedRequiredChild"),
                ],
                mixed,
            ),
        ));
    }
    assert_eq!(types.len(), 6);
    types
}

fn payload(child: &str) -> super::FieldDef {
    let mut field = flattened("payload", TypeRef::Optional(Box::new(TypeRef::Named(child.into()))));
    field.optional = true;
    field
}

#[test]
fn flattened_children_only_receive_unconsumed_parent_and_sibling_keys() {
    run_contract("TestConsumedKeysFlatten", 10);
}
