use alef::core::config::{Language, ResolvedCrateConfig, TraitBridgeConfig};
use alef::core::ir::{ApiSurface, TypeDef};
use alef::scaffold::scaffold;

fn make_type(name: &str, is_trait: bool) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: format!("demo::{name}"),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![],
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        doc: String::new(),
        cfg: None,
        is_trait,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        serde_container_default: false,
        serde_container_conversion: Default::default(),
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
    }
}

/// `tokio-util` had exactly one consumer in generated Node output: the trait-bridge wrapper's
/// `cancellation_token` field, which was written and cancelled but never read (see #1636's drop-
/// hazard cleanup, which removed the field entirely). No trait-bridge shape needs `tokio-util`
/// today, present or absent, so the dependency and its cargo-machete ignore entry must never be
/// emitted — this test replaces a same-named test that asserted the opposite (its presence).
#[test]
fn scaffold_napi_cargo_never_includes_tokio_util() {
    let api = ApiSurface {
        unresolved_modules: Vec::new(),
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![make_type("MyTrait", true)],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: Default::default(),
        excluded_trait_names: Default::default(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: vec![],
    };

    let mut config = ResolvedCrateConfig {
        name: "demo".to_string(),
        languages: vec![Language::Node],
        workspace_root: Some(std::path::PathBuf::from("/workspace")),
        ..ResolvedCrateConfig::default()
    };
    config.replace_trait_bridges(vec![TraitBridgeConfig {
        register_fn: Some("register_my_trait".to_string()),
        trait_name: "MyTrait".to_string(),
        ..TraitBridgeConfig::default()
    }]);

    let result = scaffold(&api, &config, &[Language::Node]).expect("scaffold failed");
    let cargo_file = result
        .iter()
        .find(|file| file.path.to_string_lossy().ends_with("crates/demo-node/Cargo.toml"))
        .expect("Node scaffold should generate Cargo.toml");

    let content = &cargo_file.content;

    assert!(
        !content.contains("tokio-util"),
        "Cargo.toml must not include tokio-util: it has no remaining consumer;\nactual:\n{content}"
    );
    assert!(
        content.contains("async-trait = \"0.1\""),
        "async-trait must still be present for trait bridges"
    );
    assert!(
        content.contains("serde-json"),
        "napi 'serde-json' feature must be present for trait bridges: AlefJsReply's fallback \
         decode path needs ToNapiValue/FromNapiValue for serde_json::Value;\nactual:\n{content}"
    );
}

#[test]
fn scaffold_napi_cargo_excludes_tokio_util_when_no_trait_bridges() {
    let api = ApiSurface {
        unresolved_modules: Vec::new(),
        crate_name: "demo".into(),
        version: "0.1.0".into(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: Default::default(),
        excluded_trait_names: Default::default(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: vec![],
    };

    let config = ResolvedCrateConfig {
        name: "demo".to_string(),
        languages: vec![Language::Node],
        workspace_root: Some(std::path::PathBuf::from("/workspace")),
        ..ResolvedCrateConfig::default()
    };

    let result = scaffold(&api, &config, &[Language::Node]).expect("scaffold failed");
    let cargo_file = result
        .iter()
        .find(|file| file.path.to_string_lossy().ends_with("crates/demo-node/Cargo.toml"))
        .expect("Node scaffold should generate Cargo.toml");

    let content = &cargo_file.content;

    assert!(
        !content.contains("tokio-util"),
        "tokio-util should not be included when there are no trait bridges"
    );
}
