use alef::core::config::NewAlefConfig;
use alef::core::ir::{FunctionDef, ParamDef, PrimitiveType, TypeRef};
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::dart::DartE2eCodegen;
use alef::e2e::fixture::{Fixture, FixtureGroup};

const TOML: &str = r#"
[workspace]
languages = ["dart"]

[[crates]]
name = "sample_crate"
sources = ["src/lib.rs"]

[crates.dart]
pubspec_name = "sample_crate"

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "extract_with_external_redaction"
result_var = "result"

[[crates.e2e.call.args]]
name = "input"
field = "input.input"
type = "string"

[[crates.e2e.call.args]]
name = "findings_json"
field = "input.findings_json"
type = "string"

[[crates.e2e.call.args]]
name = "offset_encoding"
field = "input.offset_encoding"
type = "string"
optional = true

[[crates.e2e.call.args]]
name = "max_findings"
field = "input.max_findings"
type = "int"
optional = true
"#;

fn param(name: &str, ty: TypeRef, optional: bool) -> ParamDef {
    ParamDef {
        name: name.to_string(),
        ty,
        optional,
        ..ParamDef::default()
    }
}

fn target_function() -> FunctionDef {
    FunctionDef {
        name: "extract_with_external_redaction".to_string(),
        params: vec![
            param("input", TypeRef::Named("ExtractInput".to_string()), false),
            param("findings_json", TypeRef::String, false),
            param("offset_encoding", TypeRef::String, true),
            param("max_findings", TypeRef::Primitive(PrimitiveType::I64), true),
        ],
        return_type: TypeRef::Named("ExtractionResult".to_string()),
        ..FunctionDef::default()
    }
}

fn render(input: serde_json::Value) -> String {
    let cfg: NewAlefConfig = toml::from_str(TOML).expect("config parses");
    let resolved = cfg.clone().resolve().expect("config resolves").remove(0);
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    let fixture = Fixture {
        id: "external_redaction".to_string(),
        description: "external redaction".to_string(),
        input,
        ..Fixture::default()
    };
    let files = DartE2eCodegen
        .generate(
            &[FixtureGroup {
                category: "contract".to_string(),
                fixtures: vec![fixture],
            }],
            &e2e,
            &resolved,
            &[],
            &[],
            &[target_function()],
            &[],
        )
        .expect("generation succeeds");
    files
        .iter()
        .find(|file| file.path.to_string_lossy().contains("contract_test.dart"))
        .expect("contract test emitted")
        .content
        .clone()
}

#[test]
fn nullable_facade_parameters_are_positional_and_preserve_arity() {
    let rendered = render(serde_json::json!({
        "input": "document.txt",
        "findings_json": "[]",
        "offset_encoding": "unicode_code_points"
    }));

    assert!(
        rendered.contains("extractWithExternalRedaction('document.txt', '[]', 'unicode_code_points', null)"),
        "nullable positional parameters must fill omitted slots with null:\n{rendered}"
    );
    assert!(
        !rendered.contains("offsetEncoding:"),
        "facade parameter must not be named:\n{rendered}"
    );
}

#[test]
fn all_absent_nullable_facade_parameters_are_emitted_as_null() {
    let rendered = render(serde_json::json!({
        "input": "document.txt",
        "findings_json": "[]"
    }));

    assert!(
        rendered.contains("extractWithExternalRedaction('document.txt', '[]', null, null)"),
        "nullable positional parameters must preserve fixed facade arity:\n{rendered}"
    );
}
