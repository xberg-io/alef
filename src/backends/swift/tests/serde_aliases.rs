use std::collections::BTreeSet;

use crate::backends::swift::SwiftBackend;
use crate::core::backend::Backend;
use crate::core::config::NewAlefConfig;
use crate::core::ir::ApiSurface;
use crate::test_support::{spawn_from_stable_dir, toolchain};

#[path = "fixtures/serde_alias_types.rs"]
mod oracle;

const CASES: &str = include_str!("fixtures/serde_alias_cases.json");
const DECLARATIONS: [&str; 12] = [
    "enum Device",
    "enum ReaderVariant",
    "enum VisionTask",
    "enum LayoutTask",
    "enum LayoutMode",
    "enum WeightPrecision",
    "struct ReaderOptions",
    "struct VisionOptions",
    "struct LayoutOptions",
    "struct DecoderOptions",
    "struct RequiredOptions",
    "struct PermissiveOptions",
];

fn extract(source: &str) -> ApiSurface {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("lib.rs");
    std::fs::write(&path, source).unwrap();
    crate::extract::extractor::extract(&[path.as_path()], "sample", "0.1.0", None).unwrap()
}

fn fixture_api() -> ApiSurface {
    extract(include_str!("fixtures/serde_alias_types.rs"))
}

fn generate(api: &ApiSurface) -> anyhow::Result<String> {
    let config: NewAlefConfig = toml::from_str(
        "[workspace]\nlanguages = [\"swift\"]\n[[crates]]\nname = \"sample\"\nsources = [\"src/lib.rs\"]\n",
    )?;
    let files = SwiftBackend.generate_bindings(api, &config.resolve()?[0])?;
    let matches: Vec<_> = files
        .iter()
        .filter(|file| file.content.contains("public struct ReaderOptions:"))
        .collect();
    assert_eq!(matches.len(), 1, "select the actual public Swift DTO facade");
    Ok(matches[0].content.clone())
}

fn declaration<'a>(source: &'a str, name: &str) -> &'a str {
    let start = source
        .find(&format!("public {name}:"))
        .expect("actual public declaration");
    let end = start + source[start..].find("\n}").expect("top-level closing brace") + 2;
    &source[start..end]
}

#[test]
fn should_retain_per_field_aliases_for_four_existing_optional_option_shapes() {
    let api = fixture_api();
    assert_eq!(api.types.len(), 6);
    assert_eq!(api.enums.len(), 6);
    let expected = serde_json::json!({
        "ReaderOptions": {"cache_dir": ["cache-dir"], "hf_revision": ["hf-revision", "revision"]},
        "VisionOptions": {"model_path": ["model-path"], "model_id": ["model-id"],
            "hf_revision": ["hf-revision", "revision"], "cache_dir": ["cache-dir"]},
        "LayoutOptions": {"layout_mode": ["layout-mode"], "enable_chart_understanding": ["enable-chart-understanding"],
            "cache_dir": ["cache-dir"]},
        "DecoderOptions": {"model_path": ["model-path"], "model_id": ["model-id"],
            "hf_revision": ["hf-revision", "revision"], "cache_dir": ["cache-dir"]}
    });
    for (name, aliases) in expected.as_object().unwrap() {
        let ty = api.types.iter().find(|ty| &ty.name == name).unwrap();
        assert!(ty.has_default && ty.has_serde);
        assert!(ty.fields.iter().all(|field| field.optional));
        let metadata = serde_json::to_value(ty).unwrap();
        assert_eq!(&metadata["serde_deserialization"]["field_aliases"], aliases, "{name}");
    }
    let permissive = api.types.iter().find(|ty| ty.name == "PermissiveOptions").unwrap();
    let metadata = serde_json::to_value(permissive).unwrap();
    assert_eq!(
        metadata["serde_deserialization"]["field_aliases"]["model"],
        serde_json::json!(["legacy_model"])
    );
    assert_eq!(metadata["serde_deserialization"]["deny_unknown_fields"], false);
    let renamed = api.types.iter().find(|ty| ty.name == "RequiredOptions").unwrap();
    let metadata = serde_json::to_value(renamed).unwrap();
    let aliases = metadata["serde_deserialization"]["field_aliases"]["model"]
        .as_array()
        .unwrap();
    assert_eq!(
        aliases
            .iter()
            .map(|alias| alias.as_str().unwrap())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from(["legacy_model", "legacy-model"])
    );
}

#[test]
fn should_preserve_all_four_public_optional_option_structs_with_aliases() {
    let source = generate(&fixture_api()).expect("existing public DTO aliases must not block generation");
    for name in ["ReaderOptions", "VisionOptions", "LayoutOptions", "DecoderOptions"] {
        assert!(source.contains(&format!("public struct {name}: Codable")), "{name}");
    }
}

#[test]
fn should_keep_encoding_keys_canonical_while_decoding_aliases() {
    let source = generate(&fixture_api()).unwrap();
    let owner = declaration(&source, "struct ReaderOptions");
    let start = owner.find("private enum CodingKeys:").expect("canonical encoder keys");
    let end = start + owner[start..].find("\n    }").expect("encoder keys closing brace");
    let keys = &owner[start..end];
    assert!(keys.contains("\"cache_dir\"") && keys.contains("\"hf_revision\""));
    for alias in ["\"cache-dir\"", "\"hf-revision\"", "\"revision\""] {
        assert!(
            !keys.contains(alias),
            "input alias cannot become a synthesized encoder key: {keys}"
        );
    }
}

#[test]
fn should_diagnose_ambiguous_cross_field_aliases() {
    for fields in [
        "#[serde(alias = \"other\")] pub model: String, pub other: String",
        "#[serde(alias = \"legacy\")] pub model: String, #[serde(alias = \"legacy\")] pub other: String",
    ] {
        let source = format!(
            "#[derive(Clone, serde::Serialize, serde::Deserialize)]\n#[serde(deny_unknown_fields)]\n\
             pub struct Collision {{ {fields} }}"
        );
        let mut api = fixture_api();
        api.types.extend(extract(&source).types);
        let error = generate(&api).expect_err("cross-field aliases have no unambiguous target");
        assert!(error.to_string().contains("ambiguous"), "{error}");
    }
}

fn rust_case<T: serde::de::DeserializeOwned + serde::Serialize>(
    payload: &serde_json::Value,
) -> serde_json::Result<serde_json::Value> {
    serde_json::to_value(serde_json::from_value::<T>(payload.clone())?)
}

#[test]
fn should_confirm_frozen_alias_vectors_with_the_actual_rust_serde_fixture() {
    let cases: Vec<serde_json::Value> = serde_json::from_str(CASES).unwrap();
    assert_eq!(cases.len(), 46);
    let mut types = BTreeSet::new();
    for case in &cases {
        let dto = case["dto"].as_str().unwrap();
        types.insert(dto);
        let actual = match dto {
            "ReaderOptions" => rust_case::<oracle::ReaderOptions>(&case["payload"]),
            "VisionOptions" => rust_case::<oracle::VisionOptions>(&case["payload"]),
            "LayoutOptions" => rust_case::<oracle::LayoutOptions>(&case["payload"]),
            "DecoderOptions" => rust_case::<oracle::DecoderOptions>(&case["payload"]),
            "RequiredOptions" => rust_case::<oracle::RequiredOptions>(&case["payload"]),
            "PermissiveOptions" => rust_case::<oracle::PermissiveOptions>(&case["payload"]),
            _ => panic!("unrecognized fixture DTO: {dto}"),
        };
        if case["accept"] == true {
            assert_eq!(actual.unwrap(), case["canonical"], "{}", case["name"]);
        } else {
            assert!(actual.is_err(), "{}: {actual:?}", case["name"]);
        }
    }
    assert_eq!(types.len(), 6, "all option shapes and semantic controls must execute");
}

fn swift_program(api: &ApiSurface) -> String {
    let source = generate(api).expect("generate the actual alias-bearing public DTO definitions");
    let declarations = DECLARATIONS
        .iter()
        .map(|name| declaration(&source, name))
        .collect::<Vec<_>>()
        .join("\n");
    let mut templates = minijinja::Environment::new();
    templates
        .add_template("probe", include_str!("fixtures/serde_alias.swift.jinja"))
        .unwrap();
    templates
        .get_template("probe")
        .unwrap()
        .render(crate::alef_context! { declarations => declarations })
        .unwrap()
}

#[test]
fn should_execute_alias_json_cases_and_deduplicate_identical_metadata_names() {
    let Some(swift) = toolchain::SWIFT.open() else { return };
    let api = fixture_api();
    let mut injected = serde_json::to_value(&api).unwrap();
    let owner = injected["types"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|ty| ty["name"] == "RequiredOptions")
        .unwrap();
    owner["serde_deserialization"]["field_aliases"]["model"] =
        serde_json::json!(["model_name", "legacy_model", "legacy_model", "legacy-model"]);
    let deduplicated: ApiSurface = serde_json::from_value(injected).unwrap();
    for (name, api) in [("source", &api), ("duplicate_metadata", &deduplicated)] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("probe.swift");
        let cases_path = directory.path().join("cases.json");
        std::fs::write(&path, swift_program(api)).unwrap();
        std::fs::write(&cases_path, CASES).unwrap();
        let output = spawn_from_stable_dir(&swift)
            .arg(&path)
            .arg(&cases_path)
            .output()
            .unwrap();
        let stdout = String::from_utf8(output.stdout).unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        let report: serde_json::Value = serde_json::from_str(&stdout)
            .unwrap_or_else(|error| panic!("{name}: Swift probe emitted no results: {error}\n{stdout}\n{stderr}"));
        assert_eq!(report["collected"], 46, "{name}: {report}");
        assert_eq!(report["cases"].as_array().unwrap().len(), 46, "{name}: {report}");
        assert_eq!(report["passed"], 46, "{name}: {report}");
        assert_eq!(report["failed"], 0, "{name}: {report}");
        assert!(output.status.success(), "{name}: {report}\n{stderr}");
    }
}
