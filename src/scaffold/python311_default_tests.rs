//! Qualify current upstream Python 3.11 defaults through actual producers.

use super::languages::{scaffold_python, scaffold_python_cargo};
use crate::core::backend::GeneratedFile;
use crate::core::config::{NewAlefConfig, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, FunctionDef, TypeRef};
use crate::e2e::codegen::{E2eCodegen, python::PythonE2eCodegen};
use crate::e2e::config::DependencyMode;
use crate::e2e::fixture::{Assertion, Fixture, FixtureGroup};
use serde_json::json;

fn resolved() -> ResolvedCrateConfig {
    let source = r#"[workspace]
languages = ["python"]
[[crates]]
name = "sample-lib"
sources = []
[crates.python]
module_name = "_sample"
pip_name = "sample-pkg"
[crates.e2e.call]
module = "sample_pkg"
function = "ping"
result_var = "result"
[crates.e2e.packages.python]
name = "sample-pkg"
path = "../../packages/python"
version = "1.2.3"
"#;
    let config: NewAlefConfig = toml::from_str(source).expect("current consumer configuration");
    let mut crates = config.resolve().expect("resolve current consumer");
    assert_eq!(crates.len(), 1);
    let config = crates.remove(0);
    assert_eq!(
        config
            .python
            .as_ref()
            .expect("Python configuration")
            .module_name
            .as_deref(),
        Some("_sample")
    );
    assert_eq!(config.e2e.as_ref().expect("E2E configuration").call.function, "ping");
    config
}

fn function() -> FunctionDef {
    FunctionDef {
        name: "ping".to_string(),
        return_type: TypeRef::String,
        ..Default::default()
    }
}

fn scaffold() -> (String, String) {
    let config = resolved();
    let api = ApiSurface {
        crate_name: "sample-lib".to_string(),
        version: "1.2.3".to_string(),
        functions: vec![function()],
        ..Default::default()
    };
    assert_eq!(api.functions.len(), 1);
    let cargo = scaffold_python_cargo(&api, &config).expect("actual Cargo producer");
    let python = scaffold_python(&api, &config).expect("actual Python producer");
    assert_eq!(cargo.len(), 1);
    assert_eq!(python.len(), 2);
    assert_eq!(cargo[0].path.to_string_lossy(), "crates/sample-lib-py/Cargo.toml");
    assert_eq!(python[0].path.to_string_lossy(), "packages/python/pyproject.toml");
    assert_eq!(python[1].path.to_string_lossy(), "packages/python/sample_pkg/py.typed");
    assert!(python[1].content.is_empty());
    let _: toml::Value = toml::from_str(&cargo[0].content).expect("valid actual Cargo TOML");
    let _: toml::Value = toml::from_str(&python[0].content).expect("valid actual project TOML");
    (cargo[0].content.clone(), python[0].content.clone())
}

fn strings(value: &toml::Value) -> Vec<&str> {
    value
        .as_array()
        .expect("actual array")
        .iter()
        .map(|item| item.as_str().expect("string entry"))
        .collect()
}

fn assert_python311(value: &toml::Value) {
    assert_eq!(value["project"]["requires-python"].as_str(), Some(">=3.11"));
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

fn e2e(registry: bool) -> Vec<GeneratedFile> {
    let config = resolved();
    let mut e2e = config.e2e.as_ref().expect("resolved E2E").clone();
    if registry {
        e2e.dep_mode = DependencyMode::Registry;
    }
    let groups = vec![FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![Fixture {
            id: "python311_probe".to_string(),
            description: "A real string-returning call".to_string(),
            assertions: vec![Assertion {
                assertion_type: "equals".to_string(),
                value: Some(json!("pong")),
                ..Default::default()
            }],
            ..Default::default()
        }],
    }];
    let functions = vec![function()];
    assert_eq!(groups[0].fixtures.len(), 1);
    assert_eq!(groups[0].fixtures[0].assertions.len(), 1);
    assert_eq!(functions.len(), 1);
    let files = PythonE2eCodegen
        .generate(&groups, &e2e, &config, &[], &[], &functions, &[])
        .expect("actual default E2E generation");
    assert_e2e_files(&files, registry);
    files
}

fn assert_e2e_files(files: &[GeneratedFile], registry: bool) {
    assert_eq!(files.len(), 4);
    let base = if registry { "test_apps" } else { "e2e" };
    for name in [
        "conftest.py",
        "tests/__init__.py",
        "pyproject.toml",
        "tests/test_smoke.py",
    ] {
        assert_eq!(
            files
                .iter()
                .filter(|file| file.path.to_string_lossy() == format!("{base}/python/{name}"))
                .count(),
            1
        );
    }
    let test = &files
        .iter()
        .find(|file| file.path.ends_with("test_smoke.py"))
        .expect("actual test")
        .content;
    assert!(test.contains("def test_python311_probe"));
    assert!(test.contains("ping(") && test.contains("assert ") && test.contains("pong"));
}

fn assert_e2e311(registry: bool) {
    let files = e2e(registry);
    let source = &files
        .iter()
        .find(|file| file.path.ends_with("pyproject.toml"))
        .expect("actual project")
        .content;
    let value: toml::Value = toml::from_str(source).expect("actual project is TOML");
    assert_python311(&value);
    assert_eq!(value["project"]["name"].as_str(), Some("sample-pkg-e2e"));
    let expected = if registry { "sample-pkg==1.2.3" } else { "sample-pkg" };
    assert_eq!(strings(&value["project"]["dependencies"])[0], expected);
    if registry {
        assert!(value.get("tool").and_then(|tool| tool.get("uv")).is_none());
    } else {
        assert_eq!(
            value["tool"]["uv"]["sources"]["sample-pkg"]["path"].as_str(),
            Some("../../packages/python")
        );
    }
    println!(
        "ALEF_PYTHON311_CURRENT_CAPTURE {}",
        json!({
            "kind": if registry { "e2e_registry" } else { "e2e_local" },
            "files": files.iter().map(|file| json!({
                "path": file.path, "header": file.generated_header, "body": file.content,
            })).collect::<Vec<_>>(),
        })
    );
}

#[test]
fn current_default_package_metadata_and_typechecking_require_311() {
    let (_, project) = scaffold();
    let value: toml::Value = toml::from_str(&project).expect("actual project");
    assert_python311(&value);
    assert_eq!(value["tool"]["pyrefly"]["python-version"].as_str(), Some("3.11"));
    assert!(project.contains("# abi3-py311 produces a single wheel per platform that loads on Python 3.11+,"));
}

#[test]
fn current_default_cargo_and_maturin_request_311_abi() {
    let (cargo, project) = scaffold();
    let cargo_value: toml::Value = toml::from_str(&cargo).expect("actual Cargo");
    let project_value: toml::Value = toml::from_str(&project).expect("actual project");
    let expected = vec!["pyo3/extension-module", "pyo3/abi3-py311"];
    assert_eq!(strings(&cargo_value["features"]["extension-module"]), expected);
    assert_eq!(strings(&project_value["tool"]["maturin"]["features"]), expected);
    assert_eq!(
        crate::core::config::python_build::extension_module_feature_line(),
        "extension-module = [\"pyo3/extension-module\", \"pyo3/abi3-py311\"]"
    );
    println!(
        "ALEF_PYTHON311_CURRENT_CAPTURE {}",
        json!({
            "kind": "scaffold", "cargo": cargo, "pyproject": project,
        })
    );
}

#[test]
fn current_default_local_e2e_requires_311() {
    assert_e2e311(false);
}

#[test]
fn current_default_registry_e2e_requires_311() {
    assert_e2e311(true);
}

#[test]
fn current_registered_template_without_new_context_requires_311() {
    let rendered = crate::e2e::template_env::render(
        "python/pyproject.toml.jinja",
        crate::alef_context! {
            pkg_name => "sample-pkg",
            deps_line => "dependencies = [ \"sample-pkg\" ]",
            uv_sources_block => "",
        },
    );
    let value: toml::Value = toml::from_str(&rendered).expect("actual registered template TOML");
    assert_python311(&value);
    assert_eq!(strings(&value["project"]["dependencies"]), vec!["sample-pkg"]);
    println!(
        "ALEF_PYTHON311_CURRENT_CAPTURE {}",
        json!({
            "kind": "registered_template", "body": rendered,
        })
    );
}
