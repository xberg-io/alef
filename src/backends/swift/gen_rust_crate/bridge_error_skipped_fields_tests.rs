use super::bridge_error::definition;
use crate::core::ir::{ApiSurface, TypeRef};
use crate::test_support::cargo_artifacts::{add_dependency_search_paths, dependency_rlibs};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const SOURCE: &str = include_str!("bridge_error_skipped_fields_fixture.rs.txt");

fn extracted_api(directory: &Path) -> ApiSurface {
    let source = directory.join("lib.rs");
    std::fs::write(&source, SOURCE).unwrap();
    let mut api = crate::extract::extractor::extract(&[source.as_path()], "sample_core", "0.0.0", None).unwrap();
    assert_eq!(api.errors.len(), 1, "the fixture must extract one real error enum");
    crate::cli::pipeline::sanitize_binding_projection(&mut api);
    let skipped: Vec<_> = api.errors[0]
        .variants
        .iter()
        .flat_map(|variant| &variant.fields)
        .filter(|field| field.binding_excluded)
        .collect();
    assert_eq!(
        skipped.len(),
        5,
        "all five source exclusion markers must survive extraction"
    );
    assert!(skipped.iter().all(|field| field.ty == TypeRef::String));
    assert!(skipped.iter().any(|field| field.sanitized));
    api
}

fn serde_json_library() -> PathBuf {
    dependency_rlibs("serde_json")
        .into_iter()
        .next()
        .expect("compiled serde_json dependency")
}

fn compile(directory: &Path, bridge: &str) -> (Output, PathBuf) {
    let source = ["mod sample_core {", SOURCE, "}", bridge, MAIN].join("\n");
    let source_path = directory.join("probe.rs");
    let binary = directory.join(if cfg!(windows) { "probe.exe" } else { "probe" });
    std::fs::write(&source_path, source).unwrap();
    let library = serde_json_library();
    let output = add_dependency_search_paths(&mut Command::new("rustc"))
        .current_dir(directory)
        .arg("--edition=2024")
        .arg("--extern")
        .arg(format!("serde_json={}", library.display()))
        .arg(&source_path)
        .arg("-o")
        .arg(&binary)
        .output()
        .unwrap();
    (output, binary)
}

#[test]
fn skipped_non_serializable_error_fields_compile_and_keep_visible_payloads() {
    let directory = tempfile::tempdir().unwrap();
    let api = extracted_api(directory.path());
    let bridge = definition(&api, "sample_core");
    let (compiled, binary) = compile(directory.path(), &bridge);
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let run = Command::new(binary).current_dir(directory.path()).output().unwrap();
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    let rows = String::from_utf8(run.stdout).unwrap();
    assert_eq!(
        rows.lines().collect::<Vec<_>>(),
        ["Hidden", "Named", "Mixed", "Secrets"]
    );
}

#[test]
fn compile_probe_rejects_the_previous_skipped_field_payload_selection() {
    let directory = tempfile::tempdir().unwrap();
    let mut api = extracted_api(directory.path());
    let expected = definition(&api, "sample_core");
    for field in api
        .errors
        .iter_mut()
        .flat_map(|error| &mut error.variants)
        .flat_map(|variant| &mut variant.fields)
    {
        field.binding_excluded = false;
    }
    let legacy = definition(&api, "sample_core");
    assert_ne!(
        legacy, expected,
        "the negative control must reintroduce serialization of skipped fields"
    );
    let (compiled, _) = compile(directory.path(), &legacy);
    assert!(
        !compiled.status.success(),
        "the old field selection must fail to compile"
    );
    let stderr = String::from_utf8_lossy(&compiled.stderr);
    assert!(
        stderr.contains("E0277") && stderr.contains("serde_json::Error") && stderr.contains("Serialize"),
        "{stderr}"
    );
}

#[test]
fn swift_error_cases_expose_only_visible_fields() {
    use crate::core::backend::Backend;
    let directory = tempfile::tempdir().unwrap();
    let api = extracted_api(directory.path());
    let raw: crate::core::config::new_config::NewAlefConfig = toml::from_str(
        r#"[workspace]
languages = ["swift"]
[[crates]]
name = "sample_core"
sources = ["src/lib.rs"]
"#,
    )
    .unwrap();
    let config = raw.resolve().unwrap().remove(0);
    let files = crate::backends::swift::SwiftBackend
        .generate_bindings(&api, &config)
        .unwrap();
    let swift = &files[0].content;
    for declaration in [
        "    case hidden\n",
        "    case named(message: String)\n",
        "    case mixed(message: String, field0: String)\n",
        "    case secrets(message: String)\n",
    ] {
        assert!(
            swift.contains(declaration),
            "{declaration} missing from generated Swift:\n{swift}"
        );
    }
    assert!(
        !swift.contains("token:"),
        "a skipped string must not enter the public error case"
    );
}

const MAIN: &str = r#"
fn broken_json() -> serde_json::Error {
    serde_json::from_str::<serde_json::Value>("{").unwrap_err()
}

fn main() {
    let cases = [
        (sample_core::SampleFailure::Hidden(broken_json()), "Hidden", None),
        (sample_core::SampleFailure::Named { message: "named value".into(), source: broken_json() }, "Named", Some(("message", "named value"))),
        (sample_core::SampleFailure::Mixed(broken_json(), "mixed value".into(), broken_json()), "Mixed", Some(("_1", "mixed value"))),
        (sample_core::SampleFailure::Secrets { token: "private value".into(), message: "public value".into() }, "Secrets", Some(("message", "public value"))),
    ];
    for (error, expected_kind, visible) in cases {
        let bridge = alef_bridge_error(error);
        assert_eq!(bridge.kind(), expected_kind);
        let payload: std::collections::BTreeMap<String, String> = serde_json::from_str(&bridge.payload()).unwrap();
        assert!(!payload.contains_key("source") && !payload.contains_key("token"));
        assert!(!payload.contains_key("_0") && !payload.contains_key("_2"));
        if let Some((key, expected)) = visible {
            assert_eq!(payload.len(), 1);
            assert_eq!(serde_json::from_str::<String>(&payload[key]).unwrap(), expected);
        } else {
            assert!(payload.is_empty());
        }
        println!("{expected_kind}");
    }
}
"#;
