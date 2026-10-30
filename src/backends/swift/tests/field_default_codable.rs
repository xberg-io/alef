use crate::backends::swift::SwiftBackend;
use crate::core::backend::Backend;
use crate::core::config::NewAlefConfig;
use crate::core::ir::ApiSurface;
use crate::test_support::{spawn_from_stable_dir, toolchain};

fn extracted_api() -> ApiSurface {
    let directory = tempfile::tempdir().expect("create source directory");
    let path = directory.path().join("lib.rs");
    std::fs::write(&path, include_str!("fixtures/field_default_types.rs")).expect("write real Rust source");
    crate::extract::extractor::extract(&[path.as_path()], "sample", "0.1.0", None).expect("extract real source")
}

fn generated_swift() -> String {
    let configuration: NewAlefConfig = toml::from_str(
        "[workspace]\nlanguages = [\"swift\"]\n[[crates]]\nname = \"sample\"\nsources = [\"src/lib.rs\"]\n",
    )
    .expect("parse fixture configuration");
    let files = SwiftBackend
        .generate_bindings(
            &extracted_api(),
            &configuration.resolve().expect("resolve configuration")[0],
        )
        .expect("generate Swift from real extracted API");
    let matches: Vec<_> = files
        .iter()
        .filter(|file| file.path.extension().is_some_and(|extension| extension == "swift"))
        .filter(|file| file.content.contains("public struct Options:"))
        .collect();
    assert_eq!(matches.len(), 1, "select the actual DTO facade");
    matches[0].content.clone()
}

fn declaration<'a>(source: &'a str, marker: &str) -> &'a str {
    let start = source.find(marker).expect("generated public struct");
    let opening = start + source[start..].find('{').expect("struct body");
    let mut depth = 0;
    for (offset, character) in source[opening..].char_indices() {
        match character {
            '{' => depth += 1,
            '}' => depth -= 1,
            _ => {}
        }
        if depth == 0 {
            return &source[start..=opening + offset];
        }
    }
    panic!("unterminated generated public struct");
}

#[test]
fn should_preserve_a_bare_field_default_without_fabricating_an_owner_default() {
    let api = extracted_api();
    assert_eq!(api.types.len(), 2, "the real fixture has exactly two structs");
    let owner = api.types.iter().find(|value| value.name == "Options").unwrap();
    let child = api.types.iter().find(|value| value.name == "Budget").unwrap();
    let field = owner.fields.iter().find(|value| value.name == "budget").unwrap();
    assert!(!owner.has_default, "the caller must still provide model and retries");
    assert!(child.has_default, "the actual nested type implements Default");
    assert!(field.has_bare_serde_default(), "retain the source field's wire default");
    assert!(
        field.typed_default.is_none(),
        "the existing extractor records the bare serde marker"
    );
}

#[test]
fn should_emit_a_nested_field_default_decoder_without_an_owner_default() {
    let source = generated_swift();
    let owner = declaration(&source, "public struct Options:");
    assert!(
        owner.contains("public init(from decoder: any Decoder) throws"),
        "{owner}"
    );
    assert!(
        owner.contains("Budget()"),
        "use the nested type's actual default initializer: {owner}"
    );
}

#[test]
fn should_emit_coding_keys_for_a_defaulted_field_with_unchanged_host_names() {
    let source = generated_swift();
    let owner = declaration(&source, "public struct Options:");
    assert!(owner.contains("private enum CodingKeys: String, CodingKey"), "{owner}");
    for name in ["model", "budget", "retries"] {
        assert!(owner.contains(&format!("case {name} = \"{name}\"")), "{owner}");
    }
}

#[test]
fn should_keep_required_siblings_strict_when_a_nested_field_has_a_default() {
    let source = generated_swift();
    let owner = declaration(&source, "public struct Options:");
    assert!(
        owner.contains("try container.decode(String.self, forKey: .model)"),
        "{owner}"
    );
    assert!(
        owner.contains("try container.decode(UInt32.self, forKey: .retries)"),
        "{owner}"
    );
    assert!(
        !owner.contains("decodeIfPresent(String.self, forKey: .model)"),
        "{owner}"
    );
    assert!(
        !owner.contains("decodeIfPresent(UInt32.self, forKey: .retries)"),
        "{owner}"
    );
}

fn swift_program() -> String {
    let source = generated_swift();
    let mut templates = minijinja::Environment::new();
    templates
        .add_template("probe", include_str!("fixtures/field_default_codable.swift.jinja"))
        .expect("parse serialization probe template");
    templates
        .get_template("probe")
        .unwrap()
        .render(crate::alef_context! {
            budget_declaration => declaration(&source, "public struct Budget:"),
            options_declaration => declaration(&source, "public struct Options:"),
        })
        .expect("render exact generated struct declarations")
}

#[test]
fn should_execute_the_generated_swift_field_default_json_contract() {
    let Some(swift) = toolchain::SWIFT.open() else { return };
    let directory = tempfile::tempdir().expect("create Swift probe directory");
    let path = directory.path().join("probe.swift");
    std::fs::write(&path, swift_program()).expect("write actual generated Swift DTO bodies");
    let output = spawn_from_stable_dir(swift)
        .arg(&path)
        .output()
        .expect("run Swift serialization probe");
    let stdout = String::from_utf8(output.stdout).expect("UTF-8 probe output");
    let stderr = String::from_utf8_lossy(&output.stderr);
    let report: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|error| panic!("Swift probe did not emit case results: {error}\n{stdout}\n{stderr}"));
    assert_eq!(report["collected"], 6, "{report}");
    assert_eq!(report["cases"].as_array().unwrap().len(), 6, "{report}");
    assert_eq!(report["failed"], 0, "{report}");
    assert_eq!(report["passed"], 6, "{report}");
    assert!(output.status.success(), "{report}\n{stderr}");
}
