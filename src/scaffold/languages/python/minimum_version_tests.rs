//! Real scaffold output follows the resolved consumer Python minimum.

use super::{scaffold_python, scaffold_python_cargo};
use crate::core::config::{NewAlefConfig, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, FunctionDef, TypeRef};
use serde_json::json;

fn resolved(minimum: Option<&str>) -> ResolvedCrateConfig {
    let setting = minimum
        .map(|value| format!("[crates.python.dependency_versions]\n\"runtime:python\" = {value:?}"))
        .unwrap_or_default();
    let source = format!(
        r#"[workspace]
languages = ["python"]
[[crates]]
name = "sample-lib"
sources = []
[crates.python]
module_name = "_sample"
pip_name = "sample-pkg"
{setting}
"#
    );
    let parsed: NewAlefConfig = toml::from_str(&source).expect("real consumer configuration parses");
    let mut crates = parsed.resolve().expect("real consumer configuration resolves");
    assert_eq!(crates.len(), 1);
    let config = crates.remove(0);
    let python = config.python.as_ref().expect("resolved Python configuration");
    assert_eq!(python.module_name.as_deref(), Some("_sample"));
    let value = serde_json::to_value(python).expect("resolved Python configuration serializes");
    assert_eq!(
        value["dependency_versions"].get("runtime:python"),
        minimum.map(|version| json!(version)).as_ref()
    );
    config
}

fn outputs(minimum: Option<&str>) -> (String, String) {
    let config = resolved(minimum);
    let api = ApiSurface {
        crate_name: "sample-lib".to_string(),
        version: "1.2.3".to_string(),
        functions: vec![FunctionDef {
            name: "ping".to_string(),
            return_type: TypeRef::String,
            ..Default::default()
        }],
        ..Default::default()
    };
    assert_eq!(api.functions.len(), 1);
    let cargo = scaffold_python_cargo(&api, &config).expect("real Cargo scaffold");
    assert_eq!(cargo.len(), 1);
    assert_eq!(cargo[0].path.to_string_lossy(), "crates/sample-lib-py/Cargo.toml");
    let python = scaffold_python(&api, &config).expect("real package scaffold");
    assert_eq!(python.len(), 2);
    assert_eq!(python[0].path.to_string_lossy(), "packages/python/pyproject.toml");
    assert_eq!(python[1].path.to_string_lossy(), "packages/python/sample_pkg/py.typed");
    assert!(python[1].content.is_empty());
    let _: toml::Value = toml::from_str(&cargo[0].content).expect("actual Cargo output is valid TOML");
    let _: toml::Value = toml::from_str(&python[0].content).expect("actual package output is valid TOML");
    (cargo[0].content.clone(), python[0].content.clone())
}

fn manifest(source: &str) -> toml::Value {
    toml::from_str(source).expect("actual producer TOML")
}

fn strings(value: &toml::Value) -> Vec<&str> {
    value
        .as_array()
        .expect("actual array")
        .iter()
        .map(|item| item.as_str().expect("string item"))
        .collect()
}

#[test]
fn configured_311_sets_package_requirement() {
    let (_, project) = outputs(Some("3.11"));
    assert_eq!(
        manifest(&project)["project"]["requires-python"].as_str(),
        Some(">=3.11")
    );
}

#[test]
fn configured_311_excludes_older_python_classifiers() {
    let (_, project) = outputs(Some("3.11"));
    let value = manifest(&project);
    assert_eq!(
        strings(&value["project"]["classifiers"]),
        vec![
            "Programming Language :: Python :: 3 :: Only",
            "Programming Language :: Python :: 3.11",
            "Programming Language :: Python :: 3.12",
            "Programming Language :: Python :: 3.13",
            "Programming Language :: Python :: 3.14",
        ]
    );
}

#[test]
fn configured_311_sets_cargo_extension_abi() {
    let (cargo, _) = outputs(Some("3.11"));
    let value = manifest(&cargo);
    assert_eq!(
        strings(&value["features"]["extension-module"]),
        vec!["pyo3/extension-module", "pyo3/abi3-py311"]
    );
}

#[test]
fn configured_311_sets_maturin_abi() {
    let (_, project) = outputs(Some("3.11"));
    let value = manifest(&project);
    assert_eq!(
        strings(&value["tool"]["maturin"]["features"]),
        vec!["pyo3/extension-module", "pyo3/abi3-py311"]
    );
}

#[test]
fn configured_311_explains_the_actual_wheel_abi_floor() {
    let (_, project) = outputs(Some("3.11"));
    let comments: Vec<_> = project.lines().filter(|line| line.starts_with("# abi3-")).collect();
    assert_eq!(comments.len(), 1);
    assert_eq!(
        comments[0],
        "# abi3-py311 produces a single wheel per platform that loads on Python 3.11+,"
    );
}

#[test]
fn configured_311_sets_type_checker_version() {
    let (_, project) = outputs(Some("3.11"));
    assert_eq!(
        manifest(&project)["tool"]["pyrefly"]["python-version"].as_str(),
        Some("3.11")
    );
}

#[test]
fn configured_312_sets_metadata_without_hardcoding_311() {
    let (_, project) = outputs(Some("3.12"));
    let value = manifest(&project);
    assert_eq!(value["project"]["requires-python"].as_str(), Some(">=3.12"));
    assert_eq!(value["tool"]["pyrefly"]["python-version"].as_str(), Some("3.12"));
    assert_eq!(
        strings(&value["project"]["classifiers"]),
        vec![
            "Programming Language :: Python :: 3 :: Only",
            "Programming Language :: Python :: 3.12",
            "Programming Language :: Python :: 3.13",
            "Programming Language :: Python :: 3.14",
        ]
    );
}

#[test]
fn configured_312_sets_both_abi_surfaces_without_hardcoding_311() {
    let (cargo, project) = outputs(Some("3.12"));
    let cargo = manifest(&cargo);
    let project = manifest(&project);
    let expected = vec!["pyo3/extension-module", "pyo3/abi3-py312"];
    assert_eq!(strings(&cargo["features"]["extension-module"]), expected);
    assert_eq!(strings(&project["tool"]["maturin"]["features"]), expected);
}

#[expect(
    clippy::print_stdout,
    reason = "Qualification captures complete generated output bodies."
)]
#[test]
fn omitted_and_explicit_311_preserve_complete_default_output_bytes() {
    let omitted = outputs(None);
    assert_eq!(omitted, outputs(Some("3.11")));
    let cargo = manifest(&omitted.0);
    let project = manifest(&omitted.1);
    assert_eq!(project["project"]["requires-python"].as_str(), Some(">=3.11"));
    assert_eq!(project["tool"]["pyrefly"]["python-version"].as_str(), Some("3.11"));
    let expected = vec!["pyo3/extension-module", "pyo3/abi3-py311"];
    assert_eq!(strings(&cargo["features"]["extension-module"]), expected);
    assert_eq!(strings(&project["tool"]["maturin"]["features"]), expected);
    assert_eq!(
        crate::core::config::python_build::extension_module_feature_line(),
        "extension-module = [\"pyo3/extension-module\", \"pyo3/abi3-py311\"]"
    );
    println!(
        "ALEF_PYTHON_MINIMUM_DEFAULT_CAPTURE {}",
        json!({"kind": "scaffold", "cargo": omitted.0, "pyproject": omitted.1})
    );
}
