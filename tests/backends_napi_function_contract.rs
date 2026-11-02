use alef::backends::napi::NapiBackend;
use alef::core::backend::Backend;
use alef::core::config::{NewAlefConfig, ResolvedCrateConfig};
use alef::core::ir::ApiSurface;

const SOURCE: &str = r#"
use std::collections::BTreeMap;
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Packet {
    pub values: Option<Vec<BTreeMap<String, serde_json::Value>>>,
    pub fallbacks: Option<Option<Vec<BTreeMap<String, serde_json::Value>>>>,
}
pub fn json_owned(value: serde_json::Value) -> serde_json::Value { value }
pub fn json_borrowed(value: &serde_json::Value) -> &serde_json::Value { value }
pub fn json_with_packet(packet: Packet, value: &serde_json::Value) -> serde_json::Value {
    let _ = packet;
    value.clone()
}
pub fn echo_packet(value: Packet) -> Packet { value }
pub async fn consume_unit(value: String) { let _ = value; }
pub async fn consume_fallible(value: String) -> Result<(), String> { let _ = value; Ok(()) }
pub async fn consume_promoted(first: Option<String>, second: String) { let _ = (first, second); }
pub fn consume_sync(value: String) { let _ = value; }
"#;

fn fixture() -> (ApiSurface, ResolvedCrateConfig) {
    let dir = tempfile::tempdir().expect("fixture directory");
    let source = dir.path().join("lib.rs");
    std::fs::write(&source, SOURCE).expect("fixture source");
    let api = alef::extract::extractor::extract(&[source.as_path()], "sample_core", "0.1.0", None)
        .expect("public extraction");
    assert_eq!(api.types.len(), 1);
    assert_eq!(api.functions.len(), 8);
    let raw: NewAlefConfig =
        toml::from_str("[workspace]\nlanguages=['node']\n[[crates]]\nname='sample-core'\nsources=['src/lib.rs']\n")
            .expect("fixture configuration");
    let mut config = raw.resolve().expect("resolved configuration").remove(0);
    config
        .output_paths
        .insert("node".into(), env!("CARGO_MANIFEST_DIR").into());
    (api, config)
}

fn bindings() -> String {
    let (api, config) = fixture();
    let files = NapiBackend.generate_bindings(&api, &config).expect("bindings");
    assert_eq!(files.len(), 1);
    files[0].content.clone()
}

#[test]
fn extracted_async_unit_functions_have_matching_result_signatures() {
    let source = bindings();
    for name in ["consume_unit", "consume_fallible", "consume_promoted"] {
        let signature = source
            .lines()
            .find(|line| line.contains(&format!("fn {name}(")))
            .expect("wrapper");
        assert!(signature.contains("-> Result<()>"), "{signature}");
    }
    let sync = source
        .lines()
        .find(|line| line.contains("fn consume_sync("))
        .expect("sync wrapper");
    assert!(sync.contains("-> ()"), "{sync}");
}

#[test]
fn extracted_json_returns_preserve_owned_and_borrowed_values() {
    let source = bindings();
    assert!(source.contains("sample_core::json_owned(value)\n"), "{source}");
    assert!(
        source.contains("sample_core::json_borrowed(&value).clone()"),
        "{source}"
    );
    assert!(!source.contains("json_owned(value).to_string()"), "{source}");
}

#[test]
fn extracted_borrowed_json_is_borrowed_with_and_without_named_conversion_bindings() {
    let source = bindings();
    assert!(source.contains("sample_core::json_borrowed(&value)"), "{source}");
    assert!(
        source.contains("sample_core::json_with_packet(packet_core, &value)"),
        "{source}"
    );
}

#[test]
fn extracted_native_dto_nested_options_do_not_advertise_null() {
    let (api, config) = fixture();
    let files = NapiBackend.generate_type_stubs(&api, &config).expect("declarations");
    let dts = &files
        .iter()
        .find(|file| file.path.ends_with("index.d.ts"))
        .expect("declaration file")
        .content;
    for field in ["values", "fallbacks"] {
        let line = dts
            .lines()
            .find(|line| line.contains(&format!("readonly {field}?:")))
            .expect("optional field");
        assert_eq!(
            line.trim(),
            format!("readonly {field}?: Array<Record<string, JsonValue>>")
        );
    }
    assert!(
        dts.contains("export type JsonValue ="),
        "JSON values retain their own null contract"
    );
}
