use crate::backends::swift::SwiftBackend;
use crate::core::backend::Backend;
use crate::core::config::NewAlefConfig;
use crate::core::ir::ApiSurface;
use crate::test_support::{spawn_from_stable_dir, toolchain};

fn extracted_api(source: &str) -> ApiSurface {
    let directory = tempfile::tempdir().expect("create source directory");
    let path = directory.path().join("lib.rs");
    std::fs::write(&path, source).expect("write real Rust source");
    crate::extract::extractor::extract(&[path.as_path()], "sample", "0.1.0", None).expect("extract actual Rust source")
}

fn fixture_api() -> ApiSurface {
    extracted_api(include_str!("fixtures/serde_contract_types.rs"))
}

fn configuration() -> crate::core::config::ResolvedCrateConfig {
    let config: NewAlefConfig = toml::from_str(
        "[workspace]\nlanguages = [\"swift\"]\n[[crates]]\nname = \"sample\"\nsources = [\"src/lib.rs\"]\n",
    )
    .expect("parse fixture configuration");
    config.resolve().expect("resolve configuration").remove(0)
}

fn generated_swift() -> String {
    let files = SwiftBackend
        .generate_bindings(&fixture_api(), &configuration())
        .unwrap();
    let matches: Vec<_> = files
        .iter()
        .filter(|file| file.path.extension().is_some_and(|extension| extension == "swift"))
        .filter(|file| file.content.contains("public struct Options:"))
        .collect();
    assert_eq!(matches.len(), 1, "select the actual generated DTO facade");
    matches[0].content.clone()
}

fn declaration<'a>(source: &'a str, name: &str) -> &'a str {
    let marker = format!("public struct {name}:");
    let start = source.find(&marker).expect("generated public struct");
    let end = start + source[start..].find("\n}").expect("top-level closing brace") + 2;
    &source[start..end]
}

#[test]
fn should_retain_real_container_unknown_field_policy_in_extracted_metadata() {
    let api = fixture_api();
    assert_eq!(api.types.len(), 3);
    for (name, expected) in [("Budget", true), ("Options", true), ("Permissive", false)] {
        let ty = api.types.iter().find(|ty| ty.name == name).unwrap();
        let metadata = serde_json::to_value(ty).unwrap();
        assert_eq!(
            metadata["serde_deserialization"]["deny_unknown_fields"]
                .as_bool()
                .unwrap_or(false),
            expected,
            "{name}: {metadata}"
        );
    }
}

#[test]
fn should_distinguish_absent_and_null_defaults_in_existing_owner_decoders() {
    let source = generated_swift();
    let budget = declaration(&source, "Budget");
    for field in ["maxTokens", "optionalLimit"] {
        assert!(budget.contains(&format!("if container.contains(.{field})")), "{budget}");
    }
    assert!(budget.contains("try container.decode(UInt32.self, forKey: .maxTokens)"));
    assert!(budget.contains("try container.decodeIfPresent(UInt32.self, forKey: .optionalLimit)"));
    assert!(!budget.contains("forKey: .maxTokens) ?? 512"), "{budget}");
    assert!(!budget.contains("forKey: .optionalLimit) ?? 5"), "{budget}");
}

#[test]
fn should_leave_ordinary_struct_unknown_key_policy_permissive() {
    let source = generated_swift();
    let ordinary = declaration(&source, "Permissive");
    assert!(!ordinary.contains("public init(from decoder:"), "{ordinary}");
}

#[test]
fn should_reject_strict_alias_or_flatten_shapes_instead_of_claiming_incorrect_keys() {
    for (attribute, field, diagnostic) in [
        ("#[serde(alias = \"old_model\")]", "pub model: String", "alias"),
        (
            "#[serde(flatten)]",
            "pub extra: std::collections::HashMap<String, String>",
            "flatten",
        ),
    ] {
        let source = format!(
            "#[derive(Clone, serde::Serialize, serde::Deserialize)]\n#[serde(deny_unknown_fields)]\n\
             pub struct Unsupported {{ {attribute} {field} }}"
        );
        let result = SwiftBackend.generate_bindings(&extracted_api(&source), &configuration());
        let error = result.expect_err("unsupported strict wire contracts must fail generation");
        assert!(error.to_string().contains(diagnostic), "{error}");
    }
}

fn swift_program() -> String {
    let source = generated_swift();
    let mut templates = minijinja::Environment::new();
    templates
        .add_template("probe", include_str!("fixtures/serde_contract.swift.jinja"))
        .expect("parse serialization probe template");
    templates
        .get_template("probe")
        .unwrap()
        .render(crate::alef_context! {
            budget_declaration => declaration(&source, "Budget"),
            options_declaration => declaration(&source, "Options"),
            permissive_declaration => declaration(&source, "Permissive"),
        })
        .expect("render exact generated public declarations")
}

#[test]
fn should_execute_real_generated_swift_serde_contract_cases() {
    let Some(swift) = toolchain::SWIFT.open() else { return };
    let directory = tempfile::tempdir().expect("create Swift probe directory");
    let path = directory.path().join("probe.swift");
    std::fs::write(&path, swift_program()).expect("write actual generated Swift DTO bodies");
    let output = spawn_from_stable_dir(swift).arg(&path).output().unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    let report: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|error| panic!("Swift probe emitted no JSON results: {error}\n{stdout}\n{stderr}"));
    assert_eq!(report["collected"], 16, "{report}");
    assert_eq!(report["cases"].as_array().unwrap().len(), 16, "{report}");
    assert_eq!(report["failed"], 0, "{report}");
    assert_eq!(report["passed"], 16, "{report}");
    assert!(output.status.success(), "{report}\n{stderr}");
}
