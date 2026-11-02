use std::collections::BTreeMap;

use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

use super::schema::{ContainerDefaultedChild, DefaultedChild, PartlyDefaultedChild, RequiredChild};
use super::{TypeDef, TypeRef, WireParent, field, flattened, run_contract, structure, with_regular};

pub(super) fn oracle_cases() -> BTreeMap<String, Value> {
    let mut cases = BTreeMap::new();
    add_cases::<RequiredChild>(&mut cases, "required", true, true);
    add_cases::<PartlyDefaultedChild>(&mut cases, "partly_defaulted", true, false);
    add_cases::<DefaultedChild>(&mut cases, "defaulted", false, false);
    add_cases::<ContainerDefaultedChild>(&mut cases, "container_defaulted", false, false);
    assert_eq!(
        cases.len(),
        32,
        "four schemas, four required-field states and both emitter paths"
    );
    cases
}

fn add_cases<T: DeserializeOwned + Serialize>(
    cases: &mut BTreeMap<String, Value>,
    model: &str,
    requires_name: bool,
    requires_kind: bool,
) {
    for mixed in [false, true] {
        for (name, kind, state) in [
            (false, false, "absent"),
            (true, false, "name_only"),
            (false, true, "kind_only"),
            (true, true, "complete"),
        ] {
            let input = input_case(mixed, name, kind);
            let core: WireParent<Option<T>> =
                serde_json::from_value(input.clone()).expect("valid flattened Option input");
            let present = core.payload.is_some();
            assert_eq!(present, (!requires_name || name) && (!requires_kind || kind));
            let expected = serde_json::to_value(core).expect("serialize core optional struct oracle");
            let mode = if mixed { "mixed" } else { "only" };
            cases.insert(
                format!("{model}_{mode}_{state}"),
                json!({"input": input, "expected": expected, "present": present}),
            );
        }
    }
}

fn input_case(mixed: bool, name: bool, kind: bool) -> Value {
    let mut input = json!({"marker": "m"});
    if mixed {
        input["regular"] = json!({"kind": "live", "name": "regular"});
    }
    if name {
        input["required_name"] = json!("name");
    }
    if kind {
        input["required_kind"] = json!("kind");
    }
    input
}

pub(super) fn parent_types() -> Vec<TypeDef> {
    let mut types = Vec::new();
    for child in [
        "RequiredChild",
        "PartlyDefaultedChild",
        "DefaultedChild",
        "ContainerDefaultedChild",
    ] {
        for mixed in [false, true] {
            let mut payload = flattened("payload", TypeRef::Optional(Box::new(TypeRef::Named(child.into()))));
            payload.optional = true;
            let fields = vec![field("marker", TypeRef::String), payload];
            let mode = if mixed { "Mixed" } else { "Only" };
            types.push(structure(
                &format!("Optional{child}{mode}"),
                with_regular(fields, mixed),
            ));
        }
    }
    assert_eq!(types.len(), 8);
    types
}

#[test]
fn optional_named_structs_follow_serde_required_fields_and_defaults() {
    run_contract("TestOptionalStructFlatten", 32);
}
