use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::{E2eCodegen, python::PythonE2eCodegen};
use alef::e2e::config::E2eConfig;
use std::process::Command;

fn generated_probe() -> String {
    let config = toml::from_str::<NewAlefConfig>(
        "[workspace]\nlanguages=['python']\n[[crates]]\nname='sample-core'\nsources=[]\n\
         [crates.python]\nmodule_name='_sample'\n[crates.python.stubs]\noutput='sample'\n",
    )
    .unwrap()
    .resolve()
    .unwrap()
    .remove(0);
    let mut e2e_config = E2eConfig::default();
    e2e_config.call.module = "sample".into();
    let files = PythonE2eCodegen
        .generate(&[], &e2e_config, &config, &[], &[], &[], &[])
        .unwrap();
    let probes: Vec<_> = files
        .iter()
        .filter(|file| file.path.ends_with("test_stub_conformance.py"))
        .collect();
    assert_eq!(probes.len(), 1);
    probes[0].content.clone()
}

fn run_probe(stub: &str, expected_missing: &str) {
    let root = tempfile::tempdir().unwrap();
    let package = root.path().join("sample");
    std::fs::create_dir(&package).unwrap();
    std::fs::write(package.join("__init__.py"), "").unwrap();
    std::fs::write(package.join("_sample.pyi"), stub).unwrap();
    std::fs::write(
        package.join("_sample.py"),
        "class Callback:\n    pass\nclass Native:\n    actual_field = 'present'\n",
    )
    .unwrap();
    std::fs::write(root.path().join("conformance.py"), generated_probe()).unwrap();
    let output = Command::new("python3")
        .args(["-c", DRIVER, expected_missing])
        .current_dir(root.path())
        .output()
        .expect("Python must execute the rendered conformance regression");
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "checked=1");
}

// The stub exists, so the emitted test's pytest-only installation skip is unreachable. ~keep
const DRIVER: &str = r#"
import ast
import sys
from pathlib import Path

source = Path('conformance.py')
tree = ast.parse(source.read_text(encoding='utf-8'))
tree.body = [node for node in tree.body if not (
    isinstance(node, ast.Import) and any(alias.name == 'pytest' for alias in node.names)
)]
scope = {}
exec(compile(tree, str(source), 'exec'), scope)
check = scope['test_stub_declares_only_members_present_at_runtime']
expected = sys.argv[1]
try:
    check()
except AssertionError as error:
    assert expected and str(error) == expected, str(error)
else:
    assert not expected, f'accepted missing member: {expected}'
print('checked=1')
"#;

fn protocol_stub(base: &str) -> String {
    format!(
        "class Callback({base}):\n    def visit_text(self) -> None: ...\n\
         class Native:\n    actual_field: str\n"
    )
}

#[test]
fn protocol_members_are_structural() {
    run_probe(&protocol_stub("Protocol"), "");
}

#[test]
fn qualified_protocol_members_are_structural() {
    run_probe(&protocol_stub("typing.Protocol"), "");
}

#[test]
fn generic_protocol_members_are_structural() {
    run_probe(&protocol_stub("Protocol[T]"), "");
}

#[test]
fn qualified_generic_protocol_members_are_structural() {
    run_probe(&protocol_stub("typing.Protocol[T]"), "");
}

fn rejects_native_member(declaration: &str, name: &str) {
    let stub = format!("class Native:\n    actual_field: str\n    {declaration}\n");
    run_probe(
        &stub,
        &format!("declared in the stub but absent at runtime: ['Native.{name}']"),
    );
}

#[test]
fn missing_native_methods_still_fail() {
    rejects_native_member("def missing_method(self) -> None: ...", "missing_method");
}

#[test]
fn missing_native_fields_still_fail() {
    rejects_native_member("missing_field: str", "missing_field");
}

#[test]
fn missing_native_class_variables_still_fail() {
    rejects_native_member("missing_constant: typing.ClassVar[int]", "missing_constant");
}

#[test]
fn unrelated_protocol_names_still_require_native_members() {
    run_probe(
        &protocol_stub("custom.Protocol"),
        "declared in the stub but absent at runtime: ['Callback.visit_text']",
    );
}

#[test]
fn protocols_alone_cannot_make_the_gate_pass_without_native_checks() {
    run_probe(
        "class Callback(Protocol):\n    def visit_text(self) -> None: ...\n",
        "the stub declares no class members to check",
    );
}
