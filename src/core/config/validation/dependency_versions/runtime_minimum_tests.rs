use crate::core::config::{NewAlefConfig, PythonConfig, ResolvedCrateConfig};
use serde_json::{Value, json};

const CONFIG: &str = r#"
[workspace]
languages = ["python"]
[[crates]]
name = "sample-lib"
sources = []
[crates.python]
module_name = "_sample"
"#;

const INVALID_MINIMUMS: [&str; 8] = ["2.7", "3.9", "3.10", "3.15", "3.11.0", "", "not-a-version", ">=3.11"];

fn from_toml(value: toml::Value) -> Result<ResolvedCrateConfig, String> {
    let source = format!("{CONFIG}\n[crates.python.dependency_versions]\n\"runtime:python\" = {value}\n");
    let parsed = toml::from_str::<NewAlefConfig>(&source).map_err(|error| error.to_string())?;
    validate(parsed)
}

fn from_json(value: Value) -> Result<ResolvedCrateConfig, String> {
    let mut source = serde_json::to_value(toml::from_str::<NewAlefConfig>(CONFIG).unwrap()).unwrap();
    source["crates"][0]["python"]["dependency_versions"]["runtime:python"] = value;
    let parsed = serde_json::from_value::<NewAlefConfig>(source).map_err(|error| error.to_string())?;
    validate(parsed)
}

fn validate(config: NewAlefConfig) -> Result<ResolvedCrateConfig, String> {
    let mut resolved = config.resolve().map_err(|error| error.to_string())?;
    assert_eq!(resolved.len(), 1);
    let config = resolved.remove(0);
    crate::core::config::validation::validate_resolved(&config).map_err(|error| error.to_string())?;
    Ok(config)
}

fn assert_minimum(config: ResolvedCrateConfig, expected: &str) {
    let python = config.python.as_ref().expect("actual Python configuration");
    assert_eq!(python.module_name.as_deref(), Some("_sample"));
    assert_eq!(
        python.dependency_versions.get("runtime:python").map(String::as_str),
        Some(expected)
    );
}

#[test]
fn managed_python_311_minimum_is_accepted_from_toml() {
    assert_minimum(from_toml(toml::Value::String("3.11".into())).unwrap(), "3.11");
}

#[test]
fn managed_python_312_minimum_is_accepted_from_toml() {
    assert_minimum(from_toml(toml::Value::String("3.12".into())).unwrap(), "3.12");
}

#[test]
fn managed_python_311_minimum_is_accepted_from_json() {
    assert_minimum(from_json(json!("3.11")).unwrap(), "3.11");
}

#[test]
fn managed_python_312_minimum_is_accepted_from_json() {
    assert_minimum(from_json(json!("3.12")).unwrap(), "3.12");
}

#[test]
fn omitted_minimum_preserves_existing_serialized_bytes() {
    let config: PythonConfig = toml::from_str("module_name = '_sample'").unwrap();
    let expected = concat!(
        r#"{"dependency_versions":{},"send_sync_types":[],"module_name":"_sample","#,
        r#""async_runtime":null,"stubs":null,"pip_name":null,"features":null,"serde_rename_all":null,"#,
        r#""capsule_types":{},"release_gil":false,"exclude_functions":[],"exclude_types":[],"#,
        r#""extra_dependencies":{},"pip_dependencies":[],"scaffold_output":null,"rename_fields":{},"#,
        r#""run_wrapper":null,"extra_lint_paths":[],"extra_init_imports":{},"reexported_types":[],"#,
        r#""target_dep_overrides":[]}"#,
    );
    assert_eq!(serde_json::to_string(&config).unwrap(), expected);
}

#[test]
fn unsupported_minimum_boundaries_are_rejected_from_toml() {
    assert_minimum(from_toml(toml::Value::String("3.11".into())).unwrap(), "3.11");
    for minimum in INVALID_MINIMUMS {
        let error = from_toml(toml::Value::String(minimum.into())).unwrap_err();
        assert!(
            error.contains("unsupported Python runtime minimum"),
            "{minimum:?}: {error}"
        );
        assert!(error.contains("runtime:python"), "{error}");
    }
}

#[test]
fn unsupported_minimum_boundaries_are_rejected_from_json() {
    assert_minimum(from_json(json!("3.11")).unwrap(), "3.11");
    for minimum in INVALID_MINIMUMS {
        let error = from_json(json!(minimum)).unwrap_err();
        assert!(
            error.contains("unsupported Python runtime minimum"),
            "{minimum:?}: {error}"
        );
        assert!(error.contains("runtime:python"), "{error}");
    }
}

#[test]
fn numeric_minimum_is_rejected_in_both_formats() {
    assert!(from_toml(toml::Value::Float(3.11)).is_err());
    assert!(from_json(json!(3.11)).is_err());
}

#[test]
fn known_version_object_is_rejected_after_valid_string_resolves() {
    assert_minimum(from_json(json!("3.11")).unwrap(), "3.11");
    assert!(from_json(json!({"3.11": null})).is_err());
}
