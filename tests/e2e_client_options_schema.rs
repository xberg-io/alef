use alef::e2e::validate::validate_fixtures;
use serde_json::{Value, json};

fn validation_errors(value: Value) -> usize {
    let directory = tempfile::tempdir().expect("fixture directory");
    let fixture = json!({
        "id": "client_option_values",
        "description": "client option value schema",
        "client_options": {"value": value}
    });
    std::fs::write(directory.path().join("fixture.json"), fixture.to_string()).expect("write fixture");
    validate_fixtures(directory.path()).expect("schema validation").len()
}

#[test]
fn client_options_schema_accepts_scalars_and_homogeneous_string_and_number_arrays() {
    for value in [
        json!("text"),
        json!(-1.5),
        json!(true),
        json!([]),
        json!(["127.0.0.0/8", "quote\"slash\\"]),
        json!([-1, 0, 1.5]),
    ] {
        assert_eq!(validation_errors(value.clone()), 0, "valid option: {value}");
    }
}

#[test]
fn client_options_schema_rejects_null_objects_nested_and_mixed_arrays() {
    for value in [
        json!(null),
        json!({"nested": 1}),
        json!([["nested"]]),
        json!([true, false]),
        json!([1, "mixed"]),
        json!([null]),
    ] {
        assert_eq!(validation_errors(value.clone()), 1, "invalid option: {value}");
    }
}
