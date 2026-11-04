use crate::backends::swift::SwiftBackend;
use crate::core::backend::Backend;
use crate::core::config::NewAlefConfig;
use crate::core::ir::ApiSurface;
use crate::test_support::{spawn_from_stable_dir, toolchain};

#[path = "fixtures/serde_custom_input_types.rs"]
mod custom_input_oracle;

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
fn should_reject_strict_flatten_shapes_instead_of_claiming_incorrect_keys() {
    let source = "#[derive(Clone, serde::Serialize, serde::Deserialize)]\n\
        #[serde(deny_unknown_fields)]\n\
        pub struct Unsupported { #[serde(flatten)] pub extra: std::collections::HashMap<String, String> }";
    let result = SwiftBackend.generate_bindings(&extracted_api(source), &configuration());
    let error = result.expect_err("unsupported strict flatten contracts must fail generation");
    assert!(error.to_string().contains("flatten"), "{error}");
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

#[test]
fn should_confirm_custom_input_codecs_accept_strings_in_real_rust_serde() {
    use custom_input_oracle::{AliasRead, AliasWith, StrictRead, StrictWith};

    let canonical = serde_json::json!({"value": "7"});
    let alias = serde_json::json!({"legacy": "7"});
    assert_eq!(
        serde_json::from_value::<StrictRead>(canonical.clone()).unwrap().value,
        7
    );
    assert_eq!(serde_json::from_value::<StrictWith>(canonical).unwrap().value, 7);
    assert_eq!(serde_json::from_value::<AliasRead>(alias.clone()).unwrap().value, 7);
    assert_eq!(serde_json::from_value::<AliasWith>(alias).unwrap().value, 7);
    let number = serde_json::json!({"value": 7});
    let alias_number = serde_json::json!({"legacy": 7});
    assert!(serde_json::from_value::<StrictRead>(number.clone()).is_err());
    assert!(serde_json::from_value::<StrictWith>(number).is_err());
    assert!(serde_json::from_value::<AliasRead>(alias_number.clone()).is_err());
    assert!(serde_json::from_value::<AliasWith>(alias_number).is_err());
}

#[test]
fn should_reject_custom_input_codecs_only_at_the_guarded_swift_policy_boundary() {
    let api = extracted_api(include_str!("fixtures/serde_custom_input_types.rs"));
    assert_eq!(
        api.types.len(),
        4,
        "both input codecs in both guarded modes must be examined"
    );
    let mut accepted = Vec::new();
    for owner in &api.types {
        let mut selected = api.clone();
        selected.types = vec![owner.clone()];
        match SwiftBackend.generate_bindings(&selected, &configuration()) {
            Ok(_) => accepted.push(owner.name.clone()),
            Err(error) => {
                let attribute = if owner.name.ends_with("Read") {
                    "deserialize_with"
                } else {
                    "with"
                };
                assert!(
                    error
                        .to_string()
                        .contains(&format!("unsupported {attribute} attributes")),
                    "{}: {error}",
                    owner.name
                );
            }
        }
    }
    assert_eq!(
        accepted,
        Vec::<String>::new(),
        "custom Rust input codecs must fail closed"
    );
}

fn assert_policy_control_generates(container: &str, fields: &str) {
    let source =
        format!("#[derive(Clone, serde::Serialize, serde::Deserialize)] {container} pub struct Control {{ {fields} }}");
    let api = extracted_api(&source);
    assert_eq!(api.types.len(), 1);
    let files = SwiftBackend.generate_bindings(&api, &configuration()).unwrap();
    assert_eq!(
        files
            .iter()
            .filter(|file| file.content.contains("public struct Control:"))
            .count(),
        1,
        "positive control must produce the actual public Swift facade"
    );
}

#[test]
fn should_preserve_unguarded_custom_input_and_serialization_only_swift_behavior() {
    for attribute in ["deserialize_with = \"digits::deserialize\"", "with = \"digits\""] {
        assert_policy_control_generates("", &format!("#[serde({attribute})] pub value: u32"));
        assert_policy_control_generates(
            "",
            &format!("#[serde({attribute})] pub value: u32, #[serde(skip)] pub cache: Option<u32>"),
        );
    }
    for (container, alias) in [
        ("#[serde(deny_unknown_fields)]", ""),
        ("", "#[serde(alias = \"legacy\")]"),
    ] {
        assert_policy_control_generates(
            container,
            &format!("{alias} #[serde(serialize_with = \"digits::serialize\")] pub value: u32"),
        );
    }
}

#[test]
fn should_ignore_custom_input_codecs_on_fully_skipped_swift_caches() {
    for attribute in ["deserialize_with = \"digits::deserialize\"", "with = \"digits\""] {
        assert_policy_control_generates(
            "#[serde(deny_unknown_fields)]",
            &format!("pub value: u32, #[serde(skip, {attribute})] pub cache: Option<u32>"),
        );
    }
}

fn configuration_with_swift(swift: &str) -> crate::core::config::ResolvedCrateConfig {
    let config: NewAlefConfig = toml::from_str(&format!(
        "[workspace]\nlanguages = [\"swift\"]\n[[crates]]\nname = \"sample\"\nsources = [\"src/lib.rs\"]\n\
         [crates.swift]\n{swift}\n"
    ))
    .expect("parse fixture configuration");
    config.resolve().expect("resolve configuration").remove(0)
}

fn custom_input_type(name: &str) -> ApiSurface {
    let mut api = extracted_api(include_str!("fixtures/serde_custom_input_types.rs"));
    api.types.retain(|ty| ty.name == name);
    assert_eq!(api.types.len(), 1, "select {name}");
    api
}

#[test]
fn should_generate_a_listed_strict_custom_input_type_as_a_permissive_dto() {
    let unlisted = SwiftBackend
        .generate_bindings(&custom_input_type("StrictRead"), &configuration())
        .expect_err("control: the unlisted strict type fails closed");
    assert!(
        unlisted.to_string().contains("unsupported deserialize_with attributes"),
        "{unlisted}"
    );

    let config = configuration_with_swift("lenient_serde_types = [\"StrictRead\"]");
    let files = SwiftBackend
        .generate_bindings(&custom_input_type("StrictRead"), &config)
        .expect("a lenient strict type must generate");
    let file = files
        .iter()
        .find(|file| file.content.contains("public struct StrictRead:"))
        .expect("generated StrictRead DTO");
    let dto = declaration(&file.content, "StrictRead");
    assert!(dto.contains("public let value: UInt32"), "{dto}");
    assert!(!dto.contains("public init(from decoder:"), "{dto}");
}

#[test]
fn should_relax_only_the_listed_strict_type() {
    let mut api = extracted_api(include_str!("fixtures/serde_custom_input_types.rs"));
    api.types
        .retain(|ty| ty.name == "StrictRead" || ty.name == "StrictWith");
    let config = configuration_with_swift("lenient_serde_types = [\"StrictWith\"]");
    let error = SwiftBackend
        .generate_bindings(&api, &config)
        .expect_err("StrictRead stays strict");
    assert!(error.to_string().contains("'StrictRead'"), "{error}");
    api.types.retain(|ty| ty.name == "StrictWith");
    SwiftBackend
        .generate_bindings(&api, &config)
        .expect("the listed StrictWith generates");
}

#[test]
fn should_reject_lenient_entries_that_name_no_strict_type() {
    for (name, case) in [("AliasRead", "a permissive type"), ("Missing", "an unknown type")] {
        let config = configuration_with_swift(&format!("lenient_serde_types = [\"{name}\"]"));
        let error = SwiftBackend
            .generate_bindings(&custom_input_type("AliasRead"), &config)
            .expect_err(case);
        assert!(
            error.to_string().contains(&format!(
                "lenient_serde_types entry '{name}' names no type with serde(deny_unknown_fields)"
            )),
            "{case}: {error}"
        );
    }
}
