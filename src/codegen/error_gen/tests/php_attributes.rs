use super::*;

#[test]
fn should_own_borrowed_and_owned_php_error_strings_at_runtime() {
    for normalized in [false, true] {
        let mut error = sample_error();
        let mut body = named_field("body");
        body.optional = normalized;
        let mut status = named_field("status");
        status.ty = TypeRef::Primitive(PrimitiveType::U16);
        status.optional = normalized;
        if !normalized {
            body.ty = TypeRef::Optional(Box::new(body.ty));
            status.ty = TypeRef::Optional(Box::new(status.ty));
        }
        error.variants = vec![ErrorVariant {
            name: "Rejected".into(),
            fields: vec![named_field("vendor"), body, status],
            ..Default::default()
        }];
        let generated = gen_php_error_converter(&error, "sample");
        let assignments = ["vendor", "body", "status"]
            .map(|name| {
                let prefix = format!("{name}:");
                let lines = generated
                    .lines()
                    .map(str::trim)
                    .filter(|line| line.starts_with(&prefix))
                    .collect::<Vec<_>>();
                assert_eq!(lines.len(), 1, "{generated}");
                lines[0].to_owned()
            })
            .join("\n");
        for borrowed in [false, true] {
            compile_payload_probe(&payload_source(&assignments, borrowed));
        }
    }
}

fn payload_source(assignments: &str, borrowed: bool) -> String {
    let ty = if borrowed { "&'static str" } else { "String" };
    let value = if borrowed {
        "\"blocked\""
    } else {
        "\"blocked\".to_owned()"
    };
    let empty = if borrowed { "\"\"" } else { "String::new()" };
    format!(
        r#"
struct Core {{ vendor: {ty}, body: Option<{ty}>, status: Option<u16> }}
struct Payload {{ vendor: String, body: Option<String>, status: Option<u16> }}
fn convert(e: Core) -> Payload {{
    let Core {{ vendor, body, status }} = &e;
    Payload {{ {assignments} }}
}}
fn main() {{
    let a = convert(Core {{ vendor: {value}, body: None, status: None }});
    assert_eq!(a.vendor, "blocked"); assert_eq!(a.body, None); assert_eq!(a.status, None);
    let b = convert(Core {{ vendor: {value}, body: Some({empty}), status: Some(0) }});
    assert_eq!(b.body.as_deref(), Some("")); assert_eq!(b.status, Some(0));
    let c = convert(Core {{ vendor: {value}, body: Some({value}), status: Some(u16::MAX) }});
    assert_eq!(c.body.as_deref(), Some("blocked")); assert_eq!(c.status, Some(65535));
}}
"#
    )
}

fn compile_payload_probe(source: &str) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("probe.rs");
    let binary = directory.path().join(if cfg!(windows) { "probe.exe" } else { "probe" });
    std::fs::write(&path, source).unwrap();
    let output = std::process::Command::new("rustc")
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
fn should_preserve_php_exception_class_and_nullable_payloads() {
    assert_nullable_payloads(false);
    assert_nullable_payloads(true);
}

fn assert_nullable_payloads(normalized: bool) {
    let mut error = sample_error();
    let mut status = named_field("status");
    status.ty = if normalized {
        TypeRef::Primitive(PrimitiveType::U16)
    } else {
        TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::U16)))
    };
    status.optional = normalized;
    let mut body = named_field("body");
    body.ty = if normalized {
        TypeRef::String
    } else {
        TypeRef::Optional(Box::new(TypeRef::String))
    };
    body.optional = normalized;
    error.variants = vec![ErrorVariant {
        name: "Rejected".to_string(),
        fields: vec![named_field("vendor"), status, body],
        ..Default::default()
    }];
    let api = crate::core::ir::ApiSurface {
        errors: vec![error.clone()],
        ..Default::default()
    };
    let output = gen_php_error_types(&api, "Sample", None).unwrap() + &gen_php_error_converter(&error, "sample_markup_rs");
    let stubs = gen_php_error_stubs(&api, None).unwrap();
    assert!(stubs.contains("public ?int $status;"), "{stubs}");
    assert!(stubs.contains("public ?string $body;"), "{stubs}");
    assert!(output.contains("#[php(extends(PhpConversionError))]"), "{output}");
    assert!(output.contains("pub status: Option<u16>"), "{output}");
    assert!(output.contains("vendor: ToString::to_string(vendor)"), "{output}");
    assert!(
        output.contains("body: body.as_ref().map(ToString::to_string)"),
        "{output}"
    );
    assert!(output.contains("status: *status"), "{output}");
    assert!(output.contains(".with_object(value)"), "{output}");
    syn::parse_file(&output).expect("PHP exception conversion emits valid Rust");
}
