use alef::e2e::fixture::{Assertion, Fixture, load_fixtures};
use serde_json::{Value, json};

#[test]
fn explicit_null_survives_fixture_deserialization_and_round_trip() {
    let fixture: Fixture = serde_json::from_value(json!({
        "id": "empty_output", "call": "read_output", "input": {},
        "assertions": [{"type": "equals", "field": "output", "value": null}]
    }))
    .unwrap();
    assert_eq!(fixture.assertions.len(), 1);
    assert_eq!(fixture.assertions[0].value, Some(Value::Null));
    let encoded = serde_json::to_value(&fixture).unwrap();
    assert_eq!(encoded["assertions"][0].get("value"), Some(&Value::Null));
    let decoded: Fixture = serde_json::from_value(encoded).unwrap();
    assert_eq!(decoded.assertions[0].value, Some(Value::Null));
}

#[test]
fn directory_loader_preserves_null_in_single_and_array_fixtures() {
    let directory = tempfile::tempdir().unwrap();
    for (name, fixture) in [
        (
            "single.json",
            json!({"id": "single", "call": "read_output",
            "assertions": [{"type": "equals", "value": null}]}),
        ),
        (
            "array.json",
            json!([{"id": "array", "call": "read_output",
            "assertions": [{"type": "equals", "value": null}]}]),
        ),
    ] {
        std::fs::write(directory.path().join(name), serde_json::to_vec(&fixture).unwrap()).unwrap();
    }
    let fixtures = load_fixtures(directory.path()).unwrap();
    assert_eq!(fixtures.len(), 2);
    for fixture in fixtures {
        assert_eq!(fixture.assertions[0].value, Some(Value::Null));
    }
}

#[test]
fn omitted_expected_value_stays_absent_after_round_trip() {
    let assertion: Assertion = serde_json::from_value(json!({"type": "not_empty"})).unwrap();
    assert_eq!(assertion.value, None);
    let encoded = serde_json::to_value(&assertion).unwrap();
    assert!(encoded.get("value").is_none());
    let decoded: Assertion = serde_json::from_value(encoded).unwrap();
    assert_eq!(decoded.value, None);
}

#[test]
fn non_null_expected_values_are_unchanged() {
    for value in [
        json!(false),
        json!(0),
        json!(""),
        json!([]),
        json!({}),
        json!([null, false]),
    ] {
        let assertion: Assertion = serde_json::from_value(json!({"type": "equals", "value": value})).unwrap();
        assert_eq!(assertion.value, Some(value.clone()));
        let encoded = serde_json::to_value(&assertion).unwrap();
        assert_eq!(encoded.get("value"), Some(&value));
        let decoded: Assertion = serde_json::from_value(encoded).unwrap();
        assert_eq!(decoded.value, Some(value));
    }
}
