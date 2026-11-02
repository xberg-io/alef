use alef::backends::napi::NapiBackend;
use alef::core::backend::Backend;
use alef::core::config::{Language, NewAlefConfig, ResolvedCrateConfig};
use alef::core::ir::ApiSurface;

fn extract(source: &str) -> ApiSurface {
    let directory = tempfile::tempdir().expect("source fixture");
    let source_path = directory.path().join("lib.rs");
    std::fs::write(&source_path, source).expect("fixture source");
    let api = alef::extract::extractor::extract(&[source_path.as_path()], "sample_core", "0.1.0", None)
        .expect("public extraction");
    assert_eq!(api.enums.len(), 1);
    assert!(api.functions.is_empty());
    api
}

fn config() -> ResolvedCrateConfig {
    let raw: NewAlefConfig =
        toml::from_str("[workspace]\nlanguages=['node']\n[[crates]]\nname='sample-core'\nsources=['src/lib.rs']\n")
            .expect("configuration");
    raw.resolve().expect("resolved configuration").remove(0)
}

fn napi_features(api: &ApiSurface) -> Vec<String> {
    let files = alef::scaffold::scaffold(api, &config(), &[Language::Node]).expect("canonical scaffold");
    let manifests: Vec<_> = files
        .iter()
        .filter(|file| file.path.ends_with("crates/sample-core-node/Cargo.toml"))
        .collect();
    assert_eq!(manifests.len(), 1);
    let manifest: toml::Value = toml::from_str(&manifests[0].content).expect("generated manifest");
    manifest["dependencies"]["napi"]["features"]
        .as_array()
        .expect("explicit napi features")
        .iter()
        .map(|feature| feature.as_str().expect("feature name").to_owned())
        .collect()
}

#[test]
fn passthrough_enum_shapes_enable_their_generated_value_wrapper_dependency() {
    let cases = [
        r#"#[derive(Clone, serde::Serialize, serde::Deserialize)]
           #[serde(untagged)] pub enum Choice { Text(String), Number(u32) }"#,
        r#"#[derive(Clone, serde::Serialize, serde::Deserialize)]
           pub enum Choice { Plain, #[serde(untagged)] Custom(String) }"#,
        r#"#[derive(Clone, serde::Serialize, serde::Deserialize)]
           #[serde(tag = "kind", content = "payload")]
           pub enum Choice { Text(String), Number(u32) }"#,
        r#"#[derive(Clone, serde::Serialize, serde::Deserialize)]
           pub struct Payload { pub retry_count: u32 }
           #[derive(Clone, serde::Serialize, serde::Deserialize)]
           #[serde(tag = "kind")] pub enum Choice { Retry(Payload), Empty }"#,
    ];
    for source in cases {
        let api = extract(source);
        let bindings = NapiBackend
            .generate_bindings(&api, &config())
            .expect("complete NAPI bindings");
        assert_eq!(bindings.len(), 1);
        assert!(
            bindings[0]
                .content
                .contains("pub struct JsChoice(pub serde_json::Value);"),
            "the actual emitter must select the Value wrapper for {source}"
        );
        assert_eq!(napi_features(&api), ["async", "serde-json"], "{source}");
    }
}

#[test]
fn plain_unit_enums_do_not_enable_unused_json_support() {
    let api = extract("pub enum Choice { First, Second }");
    let bindings = NapiBackend
        .generate_bindings(&api, &config())
        .expect("plain enum bindings");
    assert_eq!(bindings.len(), 1);
    assert!(bindings[0].content.contains("pub enum JsChoice {"));
    assert_eq!(napi_features(&api), ["async"]);
}

#[test]
fn excluded_passthrough_enums_do_not_enable_unused_json_support() {
    let api = extract(
        r#"#[doc(hidden)]
           #[derive(Clone, serde::Serialize, serde::Deserialize)]
           #[serde(untagged)] pub enum Choice { Text(String), Number(u32) }"#,
    );
    assert!(api.enums[0].binding_excluded);
    assert_eq!(napi_features(&api), ["async"]);
}
