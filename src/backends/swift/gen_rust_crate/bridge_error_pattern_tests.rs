use super::*;
use crate::core::ir::ErrorVariant;

fn variant(name: &str, fields: &[(&str, bool)]) -> ErrorVariant {
    ErrorVariant {
        name: name.to_string(),
        fields: fields
            .iter()
            .map(|(name, excluded)| FieldDef {
                name: (*name).to_string(),
                ty: TypeRef::String,
                binding_excluded: *excluded,
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    }
}

fn compile_and_run_pattern(variant: &ErrorVariant, value: &str, selected: &str, expected: &str) {
    let pattern = payload_pattern("Failure", variant);
    let source = format!(
        r#"
pub enum Failure {{
    Named {{ message: String, source: Option<String> }},
    Hidden {{ source: Option<String> }},
    Tuple(String, Option<String>),
}}
fn selected(value: &Failure) -> Option<&str> {{
    match value {{ {pattern} => {selected}, _ => Some("wrong variant") }}
}}
fn main() {{ assert_eq!(selected(&{value}), {expected}); }}
"#
    );
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("probe.rs");
    let binary = directory.path().join(if cfg!(windows) { "probe.exe" } else { "probe" });
    std::fs::write(&path, source).unwrap();
    let output = std::process::Command::new("clippy-driver")
        .args(["--edition=2024", "-D", "warnings"])
        .arg(&path)
        .arg("-o")
        .arg(&binary)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(std::process::Command::new(binary).status().unwrap().success());
}

#[test]
fn should_compile_named_error_patterns_without_redundant_hidden_field_wildcards() {
    compile_and_run_pattern(
        &variant("Named", &[("message", false), ("source", true)]),
        "Failure::Named { message: \"visible\".into(), source: Some(\"private\".into()) }",
        "Some(__alef_field_0.as_str())",
        "Some(\"visible\")",
    );
}

#[test]
fn should_compile_entirely_hidden_named_error_patterns_without_empty_field_syntax() {
    compile_and_run_pattern(
        &variant("Hidden", &[("source", true)]),
        "Failure::Hidden { source: Some(\"private\".into()) }",
        "None",
        "None",
    );
}

#[test]
fn should_preserve_positional_hidden_fields_in_tuple_error_patterns() {
    compile_and_run_pattern(
        &variant("Tuple", &[("_0", false), ("_1", true)]),
        "Failure::Tuple(\"visible\".into(), Some(\"private\".into()))",
        "Some(__alef_field_0.as_str())",
        "Some(\"visible\")",
    );
}
