use alef::backends::napi::NapiBackend;
use alef::core::backend::Backend;
use alef::core::config::NewAlefConfig;

fn generated(source: &str) -> String {
    let directory = tempfile::tempdir().expect("source fixture");
    let source_path = directory.path().join("lib.rs");
    std::fs::write(&source_path, format!("use std::collections::HashMap;\n{source}")).expect("source");
    let api = alef::extract::extractor::extract(&[source_path.as_path()], "sample_core", "0.1.0", None)
        .expect("public extraction");
    assert!(!api.types.is_empty() || !api.functions.is_empty() || !api.enums.is_empty());
    let raw: NewAlefConfig =
        toml::from_str("[workspace]\nlanguages=['node']\n[[crates]]\nname='sample-core'\nsources=['src/lib.rs']\n")
            .expect("configuration");
    let config = raw.resolve().expect("resolved configuration").remove(0);
    let bindings = NapiBackend.generate_bindings(&api, &config).expect("complete bindings");
    assert_eq!(bindings.len(), 1);
    bindings[0].content.clone()
}

#[test]
fn native_map_types_import_hashmap_at_every_container_and_signature_position() {
    let cases = [
        "pub struct Packet { pub rows: Vec<HashMap<String, u32>> }",
        "pub struct Packet { pub rows: Option<Vec<HashMap<String, u32>>> }",
        "pub fn accept(value: HashMap<String, u32>) { let _ = value; }",
        "pub fn emit() -> Vec<HashMap<String, u32>> { vec![] }",
        "pub struct Engine; impl Engine { pub fn accept(&self, value: HashMap<String, u32>) { let _ = value; } }",
        "pub struct Engine; impl Engine { pub fn records(&self) -> HashMap<String, u32> { HashMap::new() } }",
        r#"#[derive(Clone, serde::Serialize, serde::Deserialize)]
           #[serde(tag = "kind")] pub enum Choice { Row { data: HashMap<String, u32> }, Empty }"#,
    ];
    assert_eq!(cases.len(), 7);
    for source in cases {
        let output = generated(source);
        assert!(
            output.contains("HashMap<String, u32>"),
            "real map type must be emitted: {source}\n{output}"
        );
        assert_eq!(
            output.matches("use std::collections::HashMap;").count(),
            1,
            "{source}\n{output}"
        );
    }
}

#[test]
fn json_passthrough_enum_payloads_do_not_import_unemitted_native_maps() {
    let output = generated(
        r#"#[derive(Clone, serde::Serialize, serde::Deserialize)]
           #[serde(untagged)] pub enum Choice { Rows(Vec<HashMap<String, u32>>), Text(String) }"#,
    );
    assert!(
        output.contains("pub struct JsChoice(pub serde_json::Value);"),
        "{output}"
    );
    assert!(!output.contains("HashMap"), "{output}");
}
