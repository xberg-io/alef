use alef::core::config::{Language, PackageMetadataConfig, ResolvedCrateConfig};
use alef::core::ir::ApiSurface;
use alef::scaffold::scaffold;
use std::path::PathBuf;

#[test]
fn scaffold_elixir_mix_exs_omits_missing_native_src_directory() {
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
        languages: vec![Language::Elixir],
        workspace_root: Some(PathBuf::from("/workspace")),
        package_metadata: Some(PackageMetadataConfig {
            license: Some("MIT".to_string()),
            ..PackageMetadataConfig::default()
        }),
        explicit_output: Default::default(),
        ..ResolvedCrateConfig::default()
    };

    let result = scaffold(&api, &config, &[Language::Elixir]).expect("scaffold failed");
    let mix_exs_file = result
        .iter()
        .find(|file| file.path.to_string_lossy().ends_with("mix.exs"))
        .expect("Elixir scaffold should generate mix.exs");

    let content = &mix_exs_file.content;

    assert!(
        !content.contains("native/demo_nif/src"),
        "mix.exs files: list must not include non-existent native/<nif>/src directory"
    );

    assert!(
        content.contains("lib"),
        "mix.exs files: list must include lib directory when native/<nif>/src does not exist"
    );

    assert!(
        content.contains("files:"),
        "mix.exs package() must include files: keyword"
    );
}
