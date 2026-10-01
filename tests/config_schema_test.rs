use alef::core::config::{
    NewAlefConfig, alef_config_schema, check_alef_config_schema, render_alef_config_schema, write_alef_config_schema,
};

#[test]
fn generated_schema_contains_versioned_release_metadata() {
    let schema = alef_config_schema("1.2.3").expect("schema generation succeeds");

    assert_eq!(
        schema.get("$id").and_then(serde_json::Value::as_str),
        Some("https://github.com/xberg-io/alef/releases/download/v1.2.3/alef.schema.json")
    );
    assert_eq!(schema.get("version").and_then(serde_json::Value::as_str), Some("1.2.3"));
    assert_eq!(
        schema.get("x-alef-version").and_then(serde_json::Value::as_str),
        Some("1.2.3")
    );
    assert_eq!(
        schema.get("$schema").and_then(serde_json::Value::as_str),
        Some("https://json-schema.org/draft/2020-12/schema")
    );
    let rendered = serde_json::to_string(&schema).expect("schema serializes");
    assert!(rendered.contains("shares_native_runtime"));
    assert!(rendered.contains("exact native runtime instance"));
}

#[test]
fn schema_check_fails_when_file_is_stale() {
    let dir = tempfile::tempdir().expect("tempdir");
    let schema_path = dir.path().join("alef.schema.json");
    write_alef_config_schema(&schema_path, "1.2.3").expect("schema writes");

    let error = check_alef_config_schema(&schema_path, "1.2.4").expect_err("stale schema should fail");

    assert!(
        error.to_string().contains("is stale"),
        "expected stale schema error, got: {error}"
    );
}

#[test]
fn repository_alef_toml_validates_against_generated_schema() {
    let schema = alef_config_schema(env!("CARGO_PKG_VERSION")).expect("schema generation succeeds");
    let validator = jsonschema::validator_for(&schema).expect("schema compiles");
    let toml_value: toml::Value = toml::from_str(include_str!("../alef.toml")).expect("alef.toml parses");
    let json_value = serde_json::to_value(toml_value).expect("TOML value converts to JSON");

    assert!(
        validator.is_valid(&json_value),
        "repository alef.toml must validate against the generated schema"
    );
}

#[test]
fn committed_schema_matches_current_package_version() {
    let expected = render_alef_config_schema(env!("CARGO_PKG_VERSION")).expect("schema renders");
    let actual = include_str!("../schemas/alef.schema.json");

    assert_eq!(actual, expected);
}

#[test]
fn python_send_sync_types_match_schema_and_rust_config() {
    let source = r#"
[workspace]
languages = ["python"]
[[crates]]
name = "sample"
sources = ["src/lib.rs"]
[crates.python]
send_sync_types = ["SharedHandle"]
"#;
    let schema = alef_config_schema(env!("CARGO_PKG_VERSION")).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    let toml_value: toml::Value = toml::from_str(source).unwrap();
    let json_value = serde_json::to_value(toml_value).unwrap();
    assert!(validator.is_valid(&json_value));
    let config: NewAlefConfig = toml::from_str(source).unwrap();
    let resolved = config.resolve().unwrap().remove(0);
    assert_eq!(resolved.python.unwrap().send_sync_types, ["SharedHandle"]);
}

#[test]
fn trait_bridge_callback_unsupported_mode_matches_schema_and_rust_config() {
    let source = r#"
[workspace]
languages = ["php"]

[[crates]]
name = "sample"
sources = ["src/lib.rs"]

[[crates.trait_bridges]]
trait_name = "OcrBackend"
exclude_languages = ["php:callbacks"]
"#;
    let schema = alef_config_schema(env!("CARGO_PKG_VERSION")).expect("schema generation succeeds");
    let rendered = serde_json::to_string(&schema).expect("schema serializes");
    assert!(rendered.contains("php:callbacks"));

    let validator = jsonschema::validator_for(&schema).expect("schema compiles");
    let toml_value: toml::Value = toml::from_str(source).expect("TOML parses");
    let json_value = serde_json::to_value(toml_value).expect("TOML value converts to JSON");
    assert!(validator.is_valid(&json_value));

    let config: NewAlefConfig = toml::from_str(source).expect("TOML deserializes");
    let resolved = config.resolve().expect("config resolves");
    let bridge = &resolved[0].trait_bridges[0];
    assert!(bridge.is_active_for("php"));
    assert!(bridge.php_callbacks_unsupported());
}

/// A `[crates.cargo_lints]` table with both string- and table-valued entries must
/// validate against the generated schema and deserialize into `NewAlefConfig`,
/// pinning the `CargoLintsConfig` schema entry to the type it describes rather
/// than to hand-edited JSON. ~keep
#[test]
fn cargo_lints_config_matches_schema_and_rust_type() {
    let toml_src = r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "sample"
sources = ["src/lib.rs"]

[crates.cargo_lints.rust]
unused_must_use = "deny"

[crates.cargo_lints.rust.unexpected_cfgs]
level = "warn"
check-cfg = ["cfg(docsrs)"]

[crates.cargo_lints.clippy]
print_stdout = "deny"
print_stderr = "deny"
"#;

    let schema = alef_config_schema(env!("CARGO_PKG_VERSION")).expect("schema generation succeeds");
    let validator = jsonschema::validator_for(&schema).expect("schema compiles");
    let toml_value: toml::Value = toml::from_str(toml_src).expect("toml parses");
    let json_value = serde_json::to_value(&toml_value).expect("TOML value converts to JSON");
    assert!(
        validator.is_valid(&json_value),
        "cargo_lints TOML must validate against the generated schema: {:?}",
        validator.iter_errors(&json_value).collect::<Vec<_>>()
    );

    let config: NewAlefConfig = toml::from_str(toml_src).expect("toml deserializes into NewAlefConfig");
    let lints = &config.crates[0].cargo_lints;
    assert_eq!(lints.rust["unused_must_use"].as_str(), Some("deny"));
    assert_eq!(lints.clippy["print_stdout"].as_str(), Some("deny"));
    assert_eq!(lints.clippy["print_stderr"].as_str(), Some("deny"));
    assert!(lints.rust["unexpected_cfgs"].is_table());
}
