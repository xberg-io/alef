use crate::backends::swift::SwiftBackend;
use crate::core::backend::Backend;
use crate::core::config::{NewAlefConfig, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, TypeRef};
use crate::test_support::{spawn_from_stable_dir, toolchain};

const GENERATED_TYPES: [&str; 3] = ["AlefWireKey", "StrictOwner", "AliasOwner"];
const EXPECTED_SWIFT_CASES: usize = 7;

fn fixture_api() -> ApiSurface {
    let directory = tempfile::tempdir().expect("create real Rust source directory");
    let path = directory.path().join("lib.rs");
    std::fs::write(&path, include_str!("fixtures/serde_policy_edges.rs")).unwrap();
    let api = crate::extract::extractor::extract(&[path.as_path()], "sample", "0.1.0", None).unwrap();
    assert_eq!(api.types.len(), 4, "extract every real fixture owner");
    for name in ["StrictOwner", "AliasOwner"] {
        let owner = api.types.iter().find(|ty| ty.name == name).unwrap();
        assert_eq!(owner.fields.len(), 1);
        assert_eq!(owner.fields[0].ty, TypeRef::Named("AlefWireKey".to_owned()));
    }
    api
}

fn configuration(exclusions: &[&str]) -> ResolvedCrateConfig {
    let encoded = serde_json::to_string(exclusions).unwrap();
    let source = format!(
        "[workspace]\nlanguages = [\"swift\"]\n[[crates]]\nname = \"sample\"\n\
         sources = [\"src/lib.rs\"]\n[crates.swift]\nexclude_fields = {encoded}\n"
    );
    let parsed: NewAlefConfig = toml::from_str(&source).expect("parse actual Swift exclusion configuration");
    let resolved = parsed.resolve().unwrap().remove(0);
    assert_eq!(
        resolved.swift.as_ref().unwrap().exclude_fields,
        exclusions.iter().map(|name| (*name).to_owned()).collect::<Vec<_>>()
    );
    resolved
}

fn facade(api: &ApiSurface) -> String {
    let files = SwiftBackend.generate_bindings(api, &configuration(&[])).unwrap();
    let matches: Vec<_> = files
        .iter()
        .filter(|file| file.path.extension().is_some_and(|extension| extension == "swift"))
        .filter(|file| file.content.contains("public struct StrictOwner:"))
        .collect();
    assert_eq!(matches.len(), 1, "select the actual generated DTO facade");
    matches[0].content.clone()
}

fn declaration<'a>(source: &'a str, name: &str) -> &'a str {
    let marker = format!("public struct {name}:");
    assert_eq!(source.matches(&marker).count(), 1, "one actual generated type {name}");
    let start = source.find(&marker).unwrap();
    let end = start
        + source[start..]
            .find("\n}")
            .expect("outer generated struct closing brace")
        + 2;
    &source[start..end]
}

#[test]
fn should_reject_qualified_exclusions_in_strict_source_dtos() {
    let api = fixture_api();
    let strict = api.types.iter().find(|ty| ty.name == "Strict").unwrap();
    assert!(strict.serde_deserialization.deny_unknown_fields);
    assert_eq!(strict.fields.len(), 2);
    assert!(strict.fields.iter().all(|field| !field.binding_excluded));
    assert!(
        facade(&api).contains("public struct Strict:"),
        "unexcluded source must generate"
    );
    let result = SwiftBackend.generate_bindings(&api, &configuration(&["Strict.omitted"]));
    let error = result.expect_err("qualified configured exclusions must not bypass strict wire-key validation");
    let message = error.to_string();
    assert!(
        message.contains("Strict") && message.contains("unexposed fields"),
        "{message}"
    );
}

fn swift_program(api: &ApiSurface) -> String {
    let source = facade(api);
    let declarations = GENERATED_TYPES
        .iter()
        .map(|name| declaration(&source, name))
        .collect::<Vec<_>>()
        .join("\n");
    let mut templates = minijinja::Environment::new();
    templates
        .add_template("probe", include_str!("fixtures/serde_policy_edges.swift.jinja"))
        .unwrap();
    templates
        .get_template("probe")
        .unwrap()
        .render(crate::alef_context! { declarations => declarations })
        .unwrap()
}

#[test]
fn should_preserve_source_dto_names_when_emitting_private_wire_key_helpers() {
    let swift = toolchain::SWIFT
        .open()
        .expect("managed Swift SDK required for the generated-name regression");
    let api = fixture_api();
    let strict = api.types.iter().find(|ty| ty.name == "StrictOwner").unwrap();
    let alias = api.types.iter().find(|ty| ty.name == "AliasOwner").unwrap();
    assert!(strict.serde_deserialization.deny_unknown_fields);
    assert!(!alias.serde_deserialization.deny_unknown_fields);
    assert_eq!(
        alias.serde_deserialization.field_aliases["payload"],
        vec!["legacy_payload".to_owned()]
    );
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("probe.swift");
    std::fs::write(&path, swift_program(&api)).unwrap();
    let output = spawn_from_stable_dir(&swift)
        .arg(&path)
        .output()
        .expect("invoke the real Swift compiler");
    eprintln!(
        "swift_wire_key_compiler_invocations=1 executable={swift:?} status={}",
        output.status
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "actual generated Swift must compile and execute\n{stdout}\n{stderr}"
    );
    let report: serde_json::Value = serde_json::from_str(&stdout).expect("actual nonempty Swift result report");
    assert_eq!(report["collected"], EXPECTED_SWIFT_CASES, "{report}");
    assert_eq!(report["cases"].as_array().unwrap().len(), EXPECTED_SWIFT_CASES);
    assert_eq!(report["passed"], EXPECTED_SWIFT_CASES, "{report}");
    assert_eq!(report["failed"], 0, "{report}");
}
