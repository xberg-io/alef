//! Executable producers: alef generates no bridge for them, and `alef verify` accepts the config.

use alef::codegen::component_producer::generate_component_producers;
use alef::core::config::{ComponentContractConfig, ResolvedCrateConfig};
use alef::core::ir::{ApiSurface, MethodDef, ParamDef, ReceiverKind, TypeDef, TypeRef};

fn fixture() -> (ApiSurface, ResolvedCrateConfig) {
    let api = ApiSurface {
        crate_name: "demo".into(),
        version: "1.2.3".into(),
        types: vec![TypeDef {
            name: "Codec".into(),
            rust_path: "demo::Codec".into(),
            is_trait: true,
            is_opaque: true,
            methods: vec![MethodDef {
                name: "transform".into(),
                params: vec![ParamDef {
                    name: "input".into(),
                    ty: TypeRef::Bytes,
                    is_ref: true,
                    ..ParamDef::default()
                }],
                return_type: TypeRef::Bytes,
                receiver: Some(ReceiverKind::Ref),
                error_type: Some("Error".into()),
                ..MethodDef::default()
            }],
            ..TypeDef::default()
        }],
        ..ApiSurface::default()
    };
    let component = serde_json::from_value(serde_json::json!({
        "name": "reverse", "producer_language": "executable", "producer_dir": "sidecars/reverse",
        "executable": { "build": "make {out}" },
        "provides": [{"contract": "codec", "implementation": "NewCodec"}]
    }))
    .expect("executable producer language must be accepted by the public component configuration");
    let config = ResolvedCrateConfig {
        name: "demo".into(),
        component_contracts: vec![ComponentContractConfig {
            name: "codec".into(),
            trait_path: "demo::Codec".into(),
            interface_version: 1,
        }],
        components: vec![component],
        ..ResolvedCrateConfig::default()
    };
    (api, config)
}

#[test]
fn executable_producer_generates_no_bridge_files() {
    let (api, mut config) = fixture();
    let files = generate_component_producers(&api, &config).expect("executable producers need no generation");
    assert!(
        files.is_empty(),
        "no bridge may be emitted, got {} file(s)",
        files.len()
    );

    // The same contract through a Rust component still generates, so the empty result is the
    // executable kind's doing and not an empty fixture.
    let mut rust = serde_json::to_value(&config.components[0]).unwrap();
    rust.as_object_mut().unwrap().remove("executable");
    rust.as_object_mut().unwrap().remove("producer_dir");
    rust["producer_language"] = serde_json::json!("rust");
    rust["features"] = serde_json::json!(["x"]);
    rust["provides"][0]["implementation"] = serde_json::json!("demo::ReverseCodec");
    config.components.push(serde_json::from_value(rust).unwrap());
    config.components[1].name = "rusty".into();
    let files = generate_component_producers(&api, &config).unwrap();
    assert!(!files.is_empty());
    assert!(
        files.iter().all(|file| !file.path.starts_with("sidecars/reverse")),
        "the executable's producer_dir must stay untouched"
    );
}
