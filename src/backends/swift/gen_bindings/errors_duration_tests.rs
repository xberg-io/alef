use super::*;

fn extracted_error() -> ErrorDef {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("lib.rs");
    std::fs::write(
        &path,
        r#"
        #[derive(thiserror::Error, Debug)]
        pub enum SampleError {
            #[error("delayed")]
            Delayed { retry_after: Option<std::time::Duration> },
        }
        impl SampleError {
            pub fn retry_after(&self) -> Option<std::time::Duration> { None }
        }
        "#,
    )
    .unwrap();
    let api = crate::extract::extractor::extract(&[path.as_path()], "sample_core", "0.0.0", None).unwrap();
    assert_eq!(api.errors.len(), 1);
    api.errors.into_iter().next().unwrap()
}

#[test]
fn optional_duration_metadata_preserves_bridge_absence() {
    let mut error = extracted_error();
    let field = &error.variants[0].fields[0];
    assert_eq!(field.ty, TypeRef::Duration);
    assert!(field.optional);
    for (ty, optional, expected) in [
        (
            TypeRef::Duration,
            true,
            "retryAfterMs < 0 ? nil : TimeInterval(retryAfterMs) / 1000",
        ),
        (
            TypeRef::Optional(Box::new(TypeRef::Duration)),
            false,
            "retryAfterMs < 0 ? nil : TimeInterval(retryAfterMs) / 1000",
        ),
        (TypeRef::Duration, false, "TimeInterval(max(retryAfterMs, 0)) / 1000"),
    ] {
        error.variants[0].fields[0].ty = ty;
        error.variants[0].fields[0].optional = optional;
        let values = associated_values(&error.variants[0], &SwiftMapper);
        let duration = values
            .iter()
            .find(|value| value.field.is_some_and(|field| field.name == "retry_after"))
            .unwrap();
        assert_eq!(
            duration.swift_type,
            if expected.contains("nil") {
                "TimeInterval?"
            } else {
                "TimeInterval"
            }
        );
        assert_eq!(bridge_value_expression(duration, &error).as_deref(), Some(expected));
    }
    let api = crate::core::ir::ApiSurface {
        errors: vec![extracted_error()],
        ..Default::default()
    };
    let producer = crate::backends::swift::gen_rust_crate::bridge_error::definition(&api, "sample_core");
    assert!(
        producer.contains("e.retry_after().map_or(-1, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))"),
        "{producer}"
    );
}

fn compile_probe(directory: &std::path::Path, swiftc: &std::path::Path, declaration: &str) -> std::process::Output {
    let source = crate::backends::swift::template_env::render(
        "error_duration_probe.swift.jinja",
        crate::alef_context! { declaration => declaration },
    );
    let path = directory.join("main.swift");
    let binary = directory.join("probe");
    std::fs::write(&path, source).unwrap();
    let mut command = std::process::Command::new(swiftc);
    command.current_dir(directory).arg("-warnings-as-errors");
    // On macOS, type-check against the generated floor, so an API newer than the floor fails
    // here. A Linux toolchain has no Apple standard library to target.
    if cfg!(target_os = "macos") {
        command.arg("-target").arg(format!(
            "{}-apple-macos{}",
            if cfg!(target_arch = "aarch64") {
                "arm64"
            } else {
                "x86_64"
            },
            crate::core::template_versions::toolchain::SWIFT_MIN_MACOS
        ));
    }
    let compilation = command.arg(&path).arg("-o").arg(&binary).output().unwrap();
    assert!(
        compilation.status.success(),
        "{}",
        String::from_utf8_lossy(&compilation.stderr)
    );
    std::process::Command::new(binary)
        .current_dir(directory)
        .output()
        .unwrap()
}

#[test]
fn generated_optional_duration_initializer_executes_in_swift() {
    let directory = tempfile::tempdir().unwrap();
    let Ok(swiftc) = which::which_in("swiftc", std::env::var_os("PATH"), directory.path()) else {
        assert!(std::env::var_os("ALEF_REQUIRE_SWIFT").is_none(), "swiftc required");
        return;
    };
    let mut declaration = String::new();
    emit_errors(&[extracted_error()], "SampleCore", &mut declaration, &SwiftMapper);
    let old = declaration.replace(
        "retryAfterMs < 0 ? nil : TimeInterval(retryAfterMs) / 1000",
        "TimeInterval(max(retryAfterMs, 0)) / 1000",
    );
    assert_ne!(old, declaration);
    let negative = compile_probe(directory.path(), &swiftc, &old);
    assert!(!negative.status.success());
    assert!(String::from_utf8_lossy(&negative.stderr).contains("missing duration must remain nil"));
    let fixed = compile_probe(directory.path(), &swiftc, &declaration);
    assert!(fixed.status.success(), "{}", String::from_utf8_lossy(&fixed.stderr));
    assert_eq!(
        String::from_utf8_lossy(&fixed.stdout).trim(),
        "verified 5 duration cases"
    );
}
