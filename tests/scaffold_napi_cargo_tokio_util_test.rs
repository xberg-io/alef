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
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
    }
}

#[test]
fn scaffold_napi_cargo_includes_tokio_util_with_rt_feature_when_trait_bridges_present() {
    let api = ApiSurface {
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

    let config = ResolvedCrateConfig {
        name: "demo".to_string(),
        languages: vec![Language::Node],
        workspace_root: Some(std::path::PathBuf::from("/workspace")),
        trait_bridges: vec![TraitBridgeConfig {
            register_fn: Some("register_my_trait".to_string()),
            trait_name: "MyTrait".to_string(),
            ..TraitBridgeConfig::default()
        }],
        ..ResolvedCrateConfig::default()
    };

    let result = scaffold(&api, &config, &[Language::Node]).expect("scaffold failed");
    let cargo_file = result
        .iter()
        .find(|file| file.path.to_string_lossy().ends_with("crates/demo-node/Cargo.toml"))
        .expect("Node scaffold should generate Cargo.toml");

    let content = &cargo_file.content;

    assert!(
        content.contains("tokio-util"),
        "Cargo.toml must include tokio-util when trait bridges are present"
    );
    assert!(
        content.contains(r#"features = ["rt"]"#)
            || content.contains("tokio-util = { version = \"0.7\", features = [\"rt\"] }"),
        "tokio-util must include the 'rt' feature"
    );

    assert!(
        content.contains("tokio-util") && content.contains("[package.metadata.cargo-machete]"),
        "tokio-util should be in the cargo-machete ignored list"
    );

    assert!(
        content.contains("async-trait = \"0.1\""),
        "async-trait must still be present for trait bridges"
    );
}

#[test]
fn scaffold_napi_cargo_excludes_tokio_util_when_no_trait_bridges() {
    let api = ApiSurface {
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
        trait_bridges: vec![],
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
