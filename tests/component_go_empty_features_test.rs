use alef::core::config::{ComponentProducerLanguage, NewAlefConfig};

fn component_config(producer: &str, implementation: &str) -> NewAlefConfig {
    serde_json::from_value(serde_json::json!({
        "workspace": { "languages": ["ffi"] },
        "crates": [{
            "name": "sample", "sources": ["src/lib.rs"],
            "component_contracts": [{ "name": "codec", "trait_path": "sample::Codec" }],
            "components": [{
                "name": "reverse", "producer_language": producer,
                "provides": [{ "contract": "codec", "implementation": implementation }],
                "features": []
            }]
        }]
    }))
    .expect("component configuration must deserialize")
}

#[test]
fn go_producer_resolves_without_cargo_features() {
    let resolved = component_config("go", "NewCodec")
        .resolve()
        .expect("Go producers do not require Cargo features");
    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0].components.len(), 1);
    assert_eq!(
        resolved[0].components[0].producer_language,
        ComponentProducerLanguage::Go
    );
    assert_eq!(resolved[0].components[0].features, Vec::<String>::new());
}

#[test]
fn rust_producer_still_requires_nonempty_cargo_features() {
    let error = component_config("rust", "sample::Codec")
        .resolve()
        .unwrap_err()
        .to_string();
    assert_eq!(
        error,
        "crate `sample`: component `reverse` must declare non-empty features"
    );
}

#[test]
fn go_producer_rejects_empty_feature_names() {
    for feature in ["", " "] {
        let mut config = component_config("go", "NewCodec");
        config.crates[0].components[0].features.push(feature.into());
        let error = config.resolve().unwrap_err().to_string();
        assert_eq!(
            error,
            "crate `sample`: component `reverse` must declare non-empty features"
        );
    }
}
