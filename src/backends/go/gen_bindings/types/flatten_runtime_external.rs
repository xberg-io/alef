use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::schema::ExternalChoice;
use super::{
    EnumDef, GoEnumRepresentation, TypeDef, TypeRef, WireChoice, field, flattened, go_enum_representation,
    run_contract, structure, with_regular,
};

#[derive(Deserialize, Serialize)]
struct Parent {
    marker: String,
    #[serde(flatten)]
    payload: ExternalChoice,
    #[serde(flatten)]
    extras: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    regular: Option<WireChoice>,
}

pub(super) fn enumeration() -> EnumDef {
    let directory = tempfile::tempdir().expect("public external data-enum extraction fixture");
    let source = directory.path().join("lib.rs");
    std::fs::write(&source, include_str!("flatten_runtime_schema.rs")).unwrap();
    let api = crate::extract::extractor::extract(&[source.as_path()], "codec_fixture", "0.1.0", None)
        .expect("actual public extraction of external data enum");
    assert_eq!(api.types.len(), 7);
    assert_eq!(api.enums.len(), 1);
    let enumeration = api.enums.into_iter().next().unwrap();
    assert_eq!(enumeration.name, "ExternalChoice");
    assert!(enumeration.serde_tag.is_none() && enumeration.serde_content.is_none());
    assert!(!enumeration.serde_untagged);
    assert_eq!(enumeration.variants.len(), 2);
    assert!(enumeration.variants.iter().all(|variant| {
        !variant.is_tuple && variant.fields.len() == 1 && matches!(&variant.fields[0].ty, TypeRef::String)
    }));
    assert_eq!(
        go_enum_representation(&enumeration),
        GoEnumRepresentation::DataInterface
    );
    enumeration
}

pub(super) fn oracle_cases() -> BTreeMap<String, Value> {
    let mut cases = BTreeMap::new();
    for mixed in [false, true] {
        let mode = if mixed { "mixed" } else { "only" };
        for (active, remainder) in [
            ("alpha", "only"),
            ("zeta", "only"),
            ("alpha", "extra"),
            ("zeta", "extra"),
            ("alpha", "inactive"),
        ] {
            let (input, extras) = input_case(active, remainder, mixed);
            let core: Parent = serde_json::from_value(input.clone())
                .expect("Serde must certify the selected external variant and remaining map before native admission");
            match &core.payload {
                ExternalChoice::Alpha { text } => {
                    assert_eq!(active, "alpha");
                    assert_eq!(text, "selected");
                }
                ExternalChoice::Zeta { note } => {
                    assert_eq!(active, "zeta");
                    assert_eq!(note, "selected");
                }
            }
            assert_eq!(core.extras, extras);
            assert!(
                !core.extras.contains_key(active),
                "the selected payload key is consumed"
            );
            if remainder == "inactive" {
                assert_eq!(core.extras.get("zeta").map(String::as_str), Some("inactive-kept"));
            }
            let expected = serde_json::to_value(core).expect("serialize actual external data-enum oracle");
            cases.insert(
                format!("{mode}_{active}_{remainder}"),
                json!({"input": input, "expected": expected, "extras": extras}),
            );
        }
    }
    assert_eq!(cases.len(), 10);
    cases
}

fn input_case(active: &str, remainder: &str, mixed: bool) -> (Value, BTreeMap<String, String>) {
    let mut input = json!({"marker": "m"});
    input[active] = if active == "alpha" {
        json!({"text": "selected"})
    } else {
        json!({"note": "selected"})
    };
    let mut extras = BTreeMap::new();
    if remainder != "only" {
        input["extra"] = json!("kept");
        extras.insert("extra".into(), "kept".into());
    }
    if remainder == "inactive" {
        input["zeta"] = json!("inactive-kept");
        extras.insert("zeta".into(), "inactive-kept".into());
    }
    if mixed {
        input["regular"] = json!({"kind": "live", "name": "regular"});
    }
    (input, extras)
}

pub(super) fn parent_types() -> Vec<TypeDef> {
    [false, true]
        .into_iter()
        .map(|mixed| {
            let mode = if mixed { "Mixed" } else { "Only" };
            structure(
                &format!("ExternalRemainder{mode}"),
                with_regular(
                    vec![
                        field("marker", TypeRef::String),
                        flattened("payload", TypeRef::Named("ExternalChoice".into())),
                        flattened(
                            "extras",
                            TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
                        ),
                    ],
                    mixed,
                ),
            )
        })
        .collect()
}

#[test]
fn core_serde_certifies_external_data_enum_selected_and_remainder_keys() {
    enumeration();
    assert_eq!(oracle_cases().len(), 10);
}

#[test]
fn external_data_enum_flatten_preserves_serde_remainder_keys() {
    run_contract("TestExternalDataFlatten", 10);
}
