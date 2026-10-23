use super::*;

fn error_with_hidden_source() -> ErrorDef {
    ErrorDef {
        exception_base: None,
        name: "CrawlError".to_string(),
        rust_path: "sample_core::CrawlError".to_string(),
        original_rust_path: String::new(),
        variants: vec![ErrorVariant {
            name: "NotFound".to_string(),
            has_source: true,
            fields: vec![
                FieldDef {
                    name: "message".to_string(),
                    ty: TypeRef::String,
                    ..Default::default()
                },
                FieldDef {
                    name: "source".to_string(),
                    ty: TypeRef::String,
                    optional: true,
                    sanitized: true,
                    binding_excluded: true,
                    binding_exclusion_reason: Some("alef(skip)".to_string()),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }],
        doc: String::new(),
        methods: Vec::new(),
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

#[test]
fn should_preserve_optional_hidden_error_source_case_arity_without_transporting_it() {
    let error = error_with_hidden_source();
    let mut output = String::new();
    emit_errors(std::slice::from_ref(&error), "SampleCore", &mut output, &SwiftMapper);
    assert!(
        output.contains("case notFound(message: String, source: String?)"),
        "{output}"
    );
    assert!(output.contains("source: nil)"), "{output}");
    assert!(!output.contains("alefBridgeErrorField(error, \"source\""), "{output}");
    let api = crate::core::ir::ApiSurface {
        errors: vec![error.clone()],
        ..Default::default()
    };
    let bridge = crate::backends::swift::gen_rust_crate::bridge_error::definition(&api, "sample_core");
    assert!(!bridge.contains("source: _"), "{bridge}");
    assert!(!bridge.contains("serde_json::to_string(__alef_field_1)"), "{bridge}");
    let mut private = error;
    private.variants[0].fields[1].sensitive = true;
    let mut declaration = String::new();
    emit_error(&private, "SampleCore", &mut declaration, &SwiftMapper);
    assert!(declaration.contains("case notFound(message: String)"), "{declaration}");
}

#[test]
fn should_preserve_swift_error_source_constructor_and_switch_patterns() {
    let Ok(swiftc) = which::which("swiftc") else {
        assert!(std::env::var_os("ALEF_REQUIRE_SWIFT").is_none(), "swiftc required");
        return;
    };
    let mut declaration = String::new();
    emit_error(
        &error_with_hidden_source(),
        "SampleCore",
        &mut declaration,
        &SwiftMapper,
    );
    let source = format!(
        r#"
{declaration}
let failure = CrawlError.notFound(message: "missing", source: nil)
switch failure {{
case .notFound(let message, let source):
precondition(message == "missing" && source == nil)
default: preconditionFailure("wrong error case")
}}
"#
    );
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("main.swift");
    let binary = directory.path().join("probe");
    std::fs::write(&path, source).unwrap();
    let output = std::process::Command::new(swiftc)
        .args(["-warnings-as-errors"])
        .arg(&path)
        .arg("-o")
        .arg(&binary)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(std::process::Command::new(binary).status().unwrap().success());
}
