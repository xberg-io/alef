use super::*;

#[test]
fn should_preserve_static_string_error_fields_as_php_attributes() {
    let mut error = sample_error();
    let mut reason = named_field("reason");
    reason.core_wrapper = CoreWrapper::StaticStr;
    error.variants = vec![ErrorVariant {
        name: "DeniedByPolicy".to_string(),
        fields: vec![reason],
        ..Default::default()
    }];
    let api = crate::core::ir::ApiSurface {
        errors: vec![error.clone()],
        ..Default::default()
    };
    let generated =
        gen_php_error_types(&api, "Sample", None).unwrap() + &gen_php_error_converter(&error, "sample_markup_rs");
    let stubs = gen_php_error_stubs(&api, None).unwrap();
    assert!(stubs.contains("public string $reason;"), "{stubs}");
    assert!(generated.contains("pub reason: String"), "{generated}");
    assert!(generated.contains("reason: ToString::to_string(reason)"), "{generated}");
    syn::parse_file(&generated).expect("PHP exception conversion emits valid Rust");
    compile_payload_probe(
        r#"
struct Core { reason: &'static str }
struct Payload { reason: String }
fn convert(e: Core) -> Payload {
    let Core { reason } = &e;
    Payload { reason: ToString::to_string(reason) }
}
fn main() { assert_eq!(convert(Core { reason: "loopback" }).reason, "loopback"); }
"#,
    );
}

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
        .current_dir(directory.path())
        .args(["--edition=2024", "-D", "warnings"])
        .arg(&path)
        .arg("-o")
        .arg(&binary)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(
        std::process::Command::new(binary)
            .current_dir(directory.path())
            .status()
            .unwrap()
            .success()
    );
}

// ~keep Run the lost-CWD scenario in a child so it cannot change the parent suite's shared CWD.
#[cfg(unix)]
#[test]
fn should_run_payload_probes_after_inherited_working_directory_is_removed() {
    const CHILD_FLAG: &str = "ALEF_PHP_ERROR_PAYLOAD_LOST_CWD_CHILD";
    const TEST_NAME: &str = concat!(
        "codegen::error_gen::tests::php_attributes::",
        "should_run_payload_probes_after_inherited_working_directory_is_removed"
    );
    if std::env::var_os(CHILD_FLAG).is_some() {
        let directory = tempfile::tempdir().unwrap();
        std::env::set_current_dir(directory.path()).unwrap();
        drop(directory);
        assert!(std::env::current_dir().is_err());
        compile_payload_probe("fn main() { assert!(std::env::current_dir().is_ok()); }");
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([TEST_NAME, "--exact", "--test-threads=1"])
        .env(CHILD_FLAG, "1")
        .current_dir(directory.path())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("1 passed") && stdout.contains("0 failed"), "{stdout}");
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
    let output =
        gen_php_error_types(&api, "Sample", None).unwrap() + &gen_php_error_converter(&error, "sample_markup_rs");
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
