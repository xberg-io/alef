use alef::codegen::error_gen::{NAPI_ERROR_INFO_MARKER, gen_napi_error_converter};
use alef::core::ir::{ErrorDef, ErrorVariant, FieldDef, MethodDef, PrimitiveType, TypeRef};
#[path = "../src/test_support/cargo_artifacts.rs"]
mod cargo_artifacts;

use cargo_artifacts::{add_dependency_search_paths, dependency_rlibs};
use std::path::PathBuf;
use std::process::Command;

fn error_definition() -> ErrorDef {
    ErrorDef {
        exception_base: None,
        name: "SampleError".to_string(),
        rust_path: "sample_core::SampleError".to_string(),
        original_rust_path: String::new(),
        variants: vec![ErrorVariant {
            name: "Failure".to_string(),
            fields: vec![FieldDef {
                name: "_0".to_string(),
                ty: TypeRef::String,
                ..Default::default()
            }],
            is_tuple: true,
            ..Default::default()
        }],
        doc: String::new(),
        methods: [
            ("status_code", TypeRef::Primitive(PrimitiveType::U16)),
            ("is_transient", TypeRef::Primitive(PrimitiveType::Bool)),
            ("error_type", TypeRef::String),
            ("retry_after", TypeRef::Optional(Box::new(TypeRef::Duration))),
            ("timeout", TypeRef::Duration),
        ]
        .into_iter()
        .map(|(name, return_type)| MethodDef {
            name: name.into(),
            return_type,
            ..Default::default()
        })
        .collect(),
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

fn serde_json_library() -> PathBuf {
    dependency_rlibs("serde_json")
        .into_iter()
        .next()
        .expect("compiled serde_json dependency")
}

fn run_converter(converter: &str) -> Vec<String> {
    let root = tempfile::tempdir().unwrap();
    let source = include_str!("fixtures/node_error_json_harness.rs").replace("/* CONVERTER */", converter);
    let source_path = root.path().join("probe.rs");
    let binary = root.path().join(if cfg!(windows) { "probe.exe" } else { "probe" });
    std::fs::write(&source_path, source).unwrap();
    let library = serde_json_library();
    let compiled = add_dependency_search_paths(&mut Command::new("rustc"))
        .current_dir(root.path())
        .arg("--edition=2024")
        .arg("--extern")
        .arg(format!("serde_json={}", library.display()))
        .arg(&source_path)
        .arg("-o")
        .arg(&binary)
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let result = Command::new(&binary).current_dir(root.path()).output().unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    String::from_utf8(result.stdout)
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect()
}

#[test]
fn generated_node_error_metadata_round_trips_control_characters() {
    let converter = gen_napi_error_converter(&error_definition(), "sample_core");
    let rows = run_converter(&converter);
    let expected = ["plain", "nul\0value", "control\u{1f}value", "line\nquote\"slash\\"];
    assert_eq!(
        rows.len(),
        expected.len(),
        "every generated converter case must execute"
    );
    for (row, expected) in rows.into_iter().zip(expected) {
        let payload = row.strip_prefix(NAPI_ERROR_INFO_MARKER).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(payload).unwrap_or_else(|error| {
            panic!("the generated Rust converter must emit JSON for {expected:?}: {error}; {payload:?}")
        });
        assert_eq!(parsed["errorType"], expected);
        assert_eq!(parsed["kind"], "Failure");
        assert!(parsed["errorCode"].is_number(), "{parsed}");
        assert_eq!(parsed["statusCode"], 429);
        assert_eq!(parsed["isTransient"], true);
        assert_eq!(
            parsed["timeout"],
            if expected == "control\u{1f}value" { -1 } else { 1250 }
        );
        assert_eq!(
            parsed["retryAfter"],
            if expected == "plain" {
                serde_json::Value::Null
            } else if expected == "control\u{1f}value" {
                serde_json::json!(-1)
            } else {
                serde_json::json!(5000)
            }
        );
        assert_eq!(parsed.as_object().unwrap().len(), 7);
    }
}

#[test]
fn runtime_probe_rejects_the_previous_rust_debug_escape_format() {
    let converter = gen_napi_error_converter(&error_definition(), "sample_core");
    let legacy = converter.replace(
        "serde_json::Value::String(e.error_type().to_string())",
        "format!(\"{:?}\", e.error_type())",
    );
    assert_ne!(
        legacy, converter,
        "the negative control must mutate the generated converter"
    );
    let rows = run_converter(&legacy);
    assert_eq!(rows.len(), 4);
    let payload = rows[1].strip_prefix(NAPI_ERROR_INFO_MARKER).unwrap();
    let error = serde_json::from_str::<serde_json::Value>(payload).unwrap_err();
    assert!(error.to_string().contains("invalid escape"), "{error}");
}

fn lint_converter(converter: &str) -> std::process::Output {
    let root = tempfile::tempdir().unwrap();
    let source = include_str!("fixtures/node_error_json_harness.rs").replace("/* CONVERTER */", converter);
    let source_path = root.path().join("probe.rs");
    std::fs::write(&source_path, source).unwrap();
    let library = serde_json_library();
    add_dependency_search_paths(&mut Command::new("clippy-driver"))
        .arg("--edition=2024")
        .arg("--extern")
        .arg(format!("serde_json={}", library.display()))
        .args(["--emit=metadata", "--error-format=json", "-D", "warnings"])
        .arg("--out-dir")
        .arg(root.path())
        .arg(&source_path)
        .current_dir(root.path())
        .output()
        .unwrap()
}

#[test]
fn generated_node_error_converter_passes_clippy_with_display_arguments() {
    let converter = gen_napi_error_converter(&error_definition(), "sample_core");
    let legacy = converter
        .replace(
            "serde_json::Value::String(kind.to_string()),",
            "serde_json::Value::String(kind.to_string()).to_string(),",
        )
        .replace("        error_code,", "        error_code.to_string(),");
    assert_ne!(
        legacy, converter,
        "the negative control must change the emitted converter"
    );
    let rejected = lint_converter(&legacy);
    assert!(
        !rejected.status.success(),
        "the original Display conversions must be rejected"
    );
    let diagnostics: Vec<serde_json::Value> = String::from_utf8_lossy(&rejected.stderr)
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    let redundant_conversions = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic["code"]["code"] == "clippy::to_string_in_format_args")
        .count();
    assert_eq!(
        redundant_conversions,
        2,
        "{}",
        String::from_utf8_lossy(&rejected.stderr)
    );
    let accepted = lint_converter(&converter);
    assert!(
        accepted.status.success(),
        "{}",
        String::from_utf8_lossy(&accepted.stderr)
    );
}
