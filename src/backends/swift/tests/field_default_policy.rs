use crate::backends::swift::SwiftBackend;
use crate::core::backend::Backend;
use crate::core::config::NewAlefConfig;
use crate::core::ir::{ApiSurface, DefaultValue};
use crate::test_support::{spawn_from_stable_dir, toolchain};

fn extracted_api() -> ApiSurface {
    let directory = tempfile::tempdir().expect("create source directory");
    let path = directory.path().join("lib.rs");
    std::fs::write(&path, include_str!("fixtures/field_default_policy.rs")).expect("write real default functions");
    crate::extract::extractor::extract(&[path.as_path()], "sample", "0.1.0", None).expect("extract actual defaults")
}

fn generated_swift() -> String {
    let config: NewAlefConfig = toml::from_str(
        "[workspace]\nlanguages = [\"swift\"]\n[[crates]]\nname = \"sample\"\nsources = [\"src/lib.rs\"]\n",
    )
    .unwrap();
    let files = SwiftBackend
        .generate_bindings(&extracted_api(), &config.resolve().unwrap()[0])
        .unwrap();
    let matches: Vec<_> = files
        .iter()
        .filter(|file| file.path.extension().is_some_and(|extension| extension == "swift"))
        .filter(|file| file.content.contains("public struct ScalarOptions:"))
        .collect();
    assert_eq!(matches.len(), 1, "select the actual generated DTO facade");
    matches[0].content.clone()
}

fn declaration<'a>(source: &'a str, name: &str) -> &'a str {
    let marker = format!("public struct {name}:");
    let start = source.find(&marker).expect("generated public struct");
    let end = start
        + source[start..]
            .find("\n}")
            .expect("top-level public struct closing brace")
        + 2;
    &source[start..end]
}

#[test]
fn should_reach_literal_scalar_and_optional_defaults_from_real_rust_functions() {
    let api = extracted_api();
    assert_eq!(api.types.len(), 4);
    for (owner, field, expected, optional) in [
        ("ScalarOptions", "retries", 3, false),
        ("OptionalOptions", "limit", 5, true),
    ] {
        let owner = api.types.iter().find(|value| value.name == owner).unwrap();
        assert!(!owner.has_default);
        assert!(owner.fields.iter().all(|field| !field.serde_skip_serializing_if));
        let field = owner.fields.iter().find(|value| value.name == field).unwrap();
        assert_eq!(field.typed_default, Some(DefaultValue::IntLiteral(expected)));
        assert_eq!(field.optional, optional);
    }
    assert!(
        api.types
            .iter()
            .find(|value| value.name == "LegacyOptions")
            .unwrap()
            .has_default
    );
    let skipped = api.types.iter().find(|value| value.name == "SkippedOptions").unwrap();
    assert!(!skipped.has_default);
    assert!(skipped.fields.iter().any(|field| field.serde_skip_serializing_if));
}

#[test]
fn should_default_a_new_required_scalar_only_when_its_key_is_missing() {
    let source = generated_swift();
    let owner = declaration(&source, "ScalarOptions");
    assert!(owner.contains("if container.contains(.retries)"), "{owner}");
    assert!(
        owner.contains("try container.decode(UInt32.self, forKey: .retries)"),
        "{owner}"
    );
    assert!(owner.contains("self.retries = 3"), "{owner}");
    assert!(!owner.contains("forKey: .retries) ?? 3"), "{owner}");
}

#[test]
fn should_distinguish_a_new_optional_default_from_an_explicit_null() {
    let source = generated_swift();
    let owner = declaration(&source, "OptionalOptions");
    assert!(owner.contains("if container.contains(.limit)"), "{owner}");
    assert!(
        owner.contains("try container.decodeIfPresent(UInt32.self, forKey: .limit)"),
        "{owner}"
    );
    assert!(owner.contains("self.limit = 5"), "{owner}");
    assert!(!owner.contains("forKey: .limit) ?? 5"), "{owner}");
}

#[test]
fn should_keep_rust_null_semantics_in_preexisting_owner_decoders() {
    let source = generated_swift();
    for name in ["LegacyOptions", "SkippedOptions"] {
        let owner = declaration(&source, name);
        assert!(
            owner.contains("try container.decode(UInt32.self, forKey: .retries)"),
            "{owner}"
        );
        assert!(owner.contains("if container.contains(.limit)"), "{owner}");
    }
}

fn swift_program() -> String {
    let source = generated_swift();
    let mut templates = minijinja::Environment::new();
    templates
        .add_template("probe", include_str!("fixtures/field_default_policy.swift.jinja"))
        .expect("parse real JSON probe template");
    templates
        .get_template("probe")
        .unwrap()
        .render(crate::alef_context! {
            scalar_declaration => declaration(&source, "ScalarOptions"),
            optional_declaration => declaration(&source, "OptionalOptions"),
            legacy_declaration => declaration(&source, "LegacyOptions"),
            skipped_declaration => declaration(&source, "SkippedOptions"),
        })
        .expect("render actual generated DTO declarations")
}

#[test]
fn should_execute_new_and_legacy_default_policies_with_real_swift_json() {
    let Some(swift) = toolchain::SWIFT.open() else { return };
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("probe.swift");
    std::fs::write(&path, swift_program()).unwrap();
    let output = spawn_from_stable_dir(swift).arg(&path).output().unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    let report: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|error| panic!("Swift probe emitted no JSON results: {error}\n{stdout}\n{stderr}"));
    assert_eq!(report["collected"], 12, "{report}");
    assert_eq!(report["cases"].as_array().unwrap().len(), 12, "{report}");
    assert_eq!(report["failed"], 0, "{report}");
    assert_eq!(report["passed"], 12, "{report}");
    assert!(output.status.success(), "{report}\n{stderr}");
}
