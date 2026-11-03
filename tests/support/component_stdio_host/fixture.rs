use alef::codegen::component::resolve_components;
use alef::component::artifact::{
    PackageInput, ProvidedContractInput, build_lock, create_manifest, feature_hash, sign_manifest_with_key,
    write_package,
};
use alef::core::config::{NewAlefConfig, ResolvedCrateConfig};
use alef::core::ir::{ApiSurface, MethodDef, ParamDef, ReceiverKind, TypeDef, TypeRef};
use alef_component_runtime::{ComponentLock, ComponentManifest};
use base64::Engine as _;
use ed25519_dalek::SigningKey;
use serde_json::json;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

pub fn fixture() -> (ApiSurface, ResolvedCrateConfig) {
    (api_surface(), config())
}

fn api_surface() -> ApiSurface {
    ApiSurface {
        crate_name: "demo_core".into(),
        version: "1.0.0".into(),
        types: vec![TypeDef {
            name: "Codec".into(),
            rust_path: "demo_core::Codec".into(),
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
    }
}

fn config() -> ResolvedCrateConfig {
    let raw: NewAlefConfig = serde_json::from_value(json!({
        "workspace":{"languages":["ffi"]},
        "crates":[{
            "name":"demo-core","sources":["src/lib.rs"],
            "component_contracts":[{"name":"codec","trait_path":"demo_core::Codec","interface_version":1}],
            "components":[{
                "name":"reverse","provides":[{"contract":"codec","implementation":"demo_core::ReverseCodec"}],
                "features":["native"],"default_features":false,"targets":[target()],"bundled_on":[]
            }],
            "component_host":{
                "output":"src/components_gen.rs","lock_path":"../components.lock.json","stdio_components":["reverse"]
            }
        }]
    }))
    .expect("deserialize sidecar host configuration");
    let mut resolved = raw
        .resolve()
        .expect("sidecar fixture must pass public configuration validation");
    assert_eq!(resolved.len(), 1, "fixture must resolve exactly one core crate");
    resolved.remove(0)
}

fn target() -> &'static str {
    alef_component_runtime::target::host_target().expect("fixture requires a supported Unix host target")
}

fn write_executable(root: &Path, api: &ApiSurface, config: &ResolvedCrateConfig) -> (PathBuf, String) {
    let resolved = resolve_components(api, config).expect("resolve the actual bytes contract");
    assert_eq!(resolved.len(), 1, "fixture must exercise exactly one component");
    assert_eq!(
        resolved[0].provides.len(),
        1,
        "fixture must exercise exactly one contract"
    );
    let provided = &resolved[0].provides[0];
    let contract_hash = provided.contract.hash_hex().expect("hash the resolved contract");
    let features = feature_hash(&resolved[0].features, resolved[0].default_features);
    let descriptor = json!({
        "protocol_version":1,"abi_major":1,"abi_minor":0,"component_id":"reverse","component_version":"1.0.0",
        "feature_set_hash":features,"provides":[{"contract":"codec","interface_version":1,
            "contract_hash":contract_hash,"implementation":provided.implementation}]
    });
    let executable = root.join("reverse-sidecar");
    fs::write(
        &executable,
        include_str!("sidecar.py").replace("__DESCRIPTOR__", &descriptor.to_string()),
    )
    .expect("write the real sidecar executable");
    (executable, contract_hash)
}

fn manifest(root: &Path, api: &ApiSurface, config: &ResolvedCrateConfig) -> (PathBuf, ComponentManifest) {
    let (executable, hash) = write_executable(root, api, config);
    let provides = [ProvidedContractInput {
        contract: "codec",
        interface_version: 1,
        contract_hash: &hash,
        implementation: &config.components[0].provides[0].implementation,
    }];
    let feature_hash = feature_hash(&config.components[0].features, config.components[0].default_features);
    let manifest = create_manifest(
        &executable,
        PackageInput {
            crate_name: "demo-core",
            component: "reverse",
            version: "1.0.0",
            target: target(),
            provides: &provides,
            features: &config.components[0].features,
            default_features: config.components[0].default_features,
            feature_hash: &feature_hash,
        },
    )
    .expect("create manifest using public artifact API");
    (executable, manifest)
}

pub fn signed_package(root: &Path, api: &ApiSurface, config: &ResolvedCrateConfig) -> ComponentLock {
    let (executable, manifest) = manifest(root, api, config);
    let key = SigningKey::from_bytes(&[19; 32]);
    let signature = sign_manifest_with_key(&manifest, &key, "fixture").expect("sign fixture manifest");
    let package = root.join("package");
    let record = write_package(&executable, &package, manifest, Some(signature)).expect("package signed sidecar");
    let keys = BTreeMap::from([(
        "fixture".into(),
        base64::engine::general_purpose::STANDARD.encode(key.verifying_key().to_bytes()),
    )]);
    let lock = build_lock(&[record], &format!("file://{}/{{artifact}}", package.display()), &keys)
        .expect("build signature-required local lock");
    assert_eq!(lock.artifacts.len(), 1);
    assert_eq!(lock.artifacts[0].key_id, "fixture");
    assert_eq!(lock.public_keys.len(), 1);
    lock
}
