//! Actual E2E generators and registered templates inherit the consumer Python minimum.

use super::PythonE2eCodegen;
use crate::core::backend::GeneratedFile;
use crate::core::config::{NewAlefConfig, ResolvedCrateConfig};
use crate::core::ir::{FunctionDef, TypeRef};
use crate::e2e::codegen::E2eCodegen;
use crate::e2e::fixture::{Assertion, Fixture, FixtureGroup};
use serde_json::json;

fn resolved(minimum: Option<&str>, mode: &str) -> ResolvedCrateConfig {
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
[crates.e2e]
[crates.e2e.call]
module = "sample_pkg"
function = "ping"
result_var = "result"
[crates.e2e.packages.python]
name = "sample-pkg"
path = "../../packages/python"
version = "1.2.3"
"#
    );
    let parsed: NewAlefConfig = toml::from_str(&source).expect("real consumer configuration parses");
    let mut crates = parsed.resolve().expect("real consumer configuration resolves");
    assert_eq!(crates.len(), 1);
    let mut config = crates.remove(0);
    let python = serde_json::to_value(config.python.as_ref().expect("resolved Python configuration"))
        .expect("resolved Python serializes");
    assert_eq!(
        python["dependency_versions"].get("runtime:python"),
        minimum.map(|version| json!(version)).as_ref()
    );
    let e2e = config.e2e.as_mut().expect("consumer-resolved E2E configuration");
    if mode == "registry" {
        e2e.dep_mode = crate::core::config::e2e::DependencyMode::Registry;
    }
    assert_eq!(
        serde_json::to_value(e2e.dep_mode).expect("dependency mode"),
        json!(mode)
    );
    assert_eq!(e2e.call.function, "ping");
    config
}

fn groups() -> Vec<FixtureGroup> {
    vec![FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![Fixture {
            id: "minimum_probe".to_string(),
            description: "A real string-returning function".to_string(),
            assertions: vec![Assertion {
                assertion_type: "equals".to_string(),
                value: Some(json!("pong")),
                ..Default::default()
            }],
            ..Default::default()
        }],
    }]
}

fn output(minimum: Option<&str>, mode: &str) -> Vec<GeneratedFile> {
    let config = resolved(minimum, mode);
    let groups = groups();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].fixtures.len(), 1);
    assert_eq!(groups[0].fixtures[0].assertions.len(), 1);
    let functions = vec![FunctionDef {
        name: "ping".to_string(),
        return_type: TypeRef::String,
        ..Default::default()
    }];
    assert_eq!(functions.len(), 1);
    let files = PythonE2eCodegen
        .generate(
            &groups,
            config.e2e.as_ref().expect("resolved E2E"),
            &config,
            &[],
            &[],
            &functions,
            &[],
        )
        .expect("actual E2E generator");
    assert_files(&files, mode);
    assert_dependencies(&project(&files), mode);
    files
}

fn assert_files(files: &[GeneratedFile], mode: &str) {
    assert_eq!(files.len(), 4);
    let base = if mode == "registry" { "test_apps" } else { "e2e" };
    for expected in [
        "conftest.py",
        "tests/__init__.py",
        "pyproject.toml",
        "tests/test_smoke.py",
    ] {
        assert_eq!(
            files
                .iter()
                .filter(|file| file.path.to_string_lossy() == format!("{base}/python/{expected}"))
                .count(),
            1
        );
    }
    let test = &files
        .iter()
        .find(|file| file.path.ends_with("test_smoke.py"))
        .expect("actual runnable test")
        .content;
    assert!(test.contains("def test_minimum_probe"));
    assert!(test.contains("ping("));
    assert!(test.contains("assert ") && test.contains("pong"));
}

fn assert_dependencies(project: &toml::Value, mode: &str) {
    assert_eq!(project["project"]["name"].as_str(), Some("sample-pkg-e2e"));
    let dependencies = strings(&project["project"]["dependencies"]);
    let expected = if mode == "registry" {
        "sample-pkg==1.2.3"
    } else {
        "sample-pkg"
    };
    assert_eq!(dependencies[0], expected);
    if mode == "local" {
        assert_eq!(
            project["tool"]["uv"]["sources"]["sample-pkg"]["path"].as_str(),
            Some("../../packages/python")
        );
    } else {
        assert!(project.get("tool").and_then(|value| value.get("uv")).is_none());
    }
}

fn project(files: &[GeneratedFile]) -> toml::Value {
    let file = files
        .iter()
        .find(|file| file.path.ends_with("pyproject.toml"))
        .expect("one actual pyproject");
    toml::from_str(&file.content).expect("actual E2E output is valid TOML")
}

fn strings(value: &toml::Value) -> Vec<&str> {
    value
        .as_array()
        .expect("actual array")
        .iter()
        .map(|item| item.as_str().expect("string item"))
        .collect()
}

fn template(minimum: Option<&str>) -> String {
    let config = resolved(minimum, "local");
    let rendered = crate::e2e::template_env::render(
        "python/pyproject.toml.jinja",
        crate::alef_context! {
            pkg_name => "sample-pkg",
            deps_line => "dependencies = [ \"sample-pkg\" ]",
            uv_sources_block => "",
            python => &config.python,
        },
    );
    let value: toml::Value = toml::from_str(&rendered).expect("registered template emits valid TOML");
    assert_eq!(value["project"]["name"].as_str(), Some("sample-pkg-e2e"));
    assert_eq!(strings(&value["project"]["dependencies"]), vec!["sample-pkg"]);
    rendered
}

#[test]
fn local_311_e2e_requires_the_consumer_minimum() {
    let files = output(Some("3.11"), "local");
    assert_eq!(project(&files)["project"]["requires-python"].as_str(), Some(">=3.11"));
}

#[test]
fn registry_311_e2e_requires_the_consumer_minimum() {
    let files = output(Some("3.11"), "registry");
    assert_eq!(project(&files)["project"]["requires-python"].as_str(), Some(">=3.11"));
}

#[test]
fn local_311_e2e_excludes_unsupported_classifiers() {
    let files = output(Some("3.11"), "local");
    let value = project(&files);
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
fn registry_312_e2e_does_not_hardcode_311() {
    let files = output(Some("3.12"), "registry");
    let value = project(&files);
    assert_eq!(value["project"]["requires-python"].as_str(), Some(">=3.12"));
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

#[expect(
    clippy::print_stdout,
    reason = "Qualification captures complete generated output bodies."
)]
#[test]
fn omitted_and_explicit_310_e2e_files_remain_byte_identical_in_both_modes() {
    let mut captures = Vec::new();
    for mode in ["local", "registry"] {
        let omitted = output(None, mode);
        let explicit = output(Some("3.10"), mode);
        assert_eq!(omitted.len(), explicit.len());
        for (left, right) in omitted.iter().zip(&explicit) {
            assert_eq!(left.path, right.path);
            assert_eq!(left.generated_header, right.generated_header);
            assert_eq!(left.content, right.content);
        }
        assert_eq!(project(&omitted)["project"]["requires-python"].as_str(), Some(">=3.10"));
        captures.push(json!({
            "mode": mode,
            "files": omitted.iter().map(|file| json!({
                "path": file.path, "header": file.generated_header, "body": file.content,
            })).collect::<Vec<_>>(),
        }));
    }
    println!(
        "ALEF_PYTHON_MINIMUM_DEFAULT_CAPTURE {}",
        json!({"kind": "e2e", "modes": captures})
    );
}

#[test]
fn registered_template_311_uses_the_resolved_python_configuration() {
    let rendered = template(Some("3.11"));
    let value: toml::Value = toml::from_str(&rendered).expect("actual template TOML");
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

#[test]
fn registered_template_312_does_not_hardcode_311() {
    let rendered = template(Some("3.12"));
    let value: toml::Value = toml::from_str(&rendered).expect("actual template TOML");
    assert_eq!(value["project"]["requires-python"].as_str(), Some(">=3.12"));
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

#[expect(
    clippy::print_stdout,
    reason = "Qualification captures complete generated output bodies."
)]
#[test]
fn omitted_and_explicit_310_registered_template_bytes_remain_identical() {
    let omitted = template(None);
    assert_eq!(omitted, template(Some("3.10")));
    let value: toml::Value = toml::from_str(&omitted).expect("actual default template TOML");
    assert_eq!(value["project"]["requires-python"].as_str(), Some(">=3.10"));
    println!(
        "ALEF_PYTHON_MINIMUM_DEFAULT_CAPTURE {}",
        json!({"kind": "registered_template", "body": omitted})
    );
}
