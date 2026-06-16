use alef::core::config::{Language, PackageMetadataConfig, ResolvedCrateConfig};
use alef::core::ir::ApiSurface;
use alef::scaffold::scaffold;
use std::path::PathBuf;

#[test]
fn scaffold_elixir_nif_cargo_pins_brotli_allocator_crates_as_direct_deps() {
    // Regression test for the brotli 8.0.x transitive-conflict pin.
    //
    // The fix uses DIRECT dependency entries in [dependencies], NOT
    // [patch.crates-io], because a `name = { version = "=X" }` patch with no
    // path/git/url is a no-op cargo rejects with
    // `patch points to the same source`. Direct deps with `=X` constraints
    // force cargo's resolver to pick the named versions for the whole tree.
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
    let cargo_toml_file = result
        .iter()
        .find(|file| file.path.to_string_lossy().ends_with("native/demo_nif/Cargo.toml"))
        .expect("Elixir scaffold should generate native/<nif>/Cargo.toml");

    let content = &cargo_toml_file.content;

    assert!(
        content.contains("alloc-no-stdlib = \"=2.0.4\""),
        "NIF Cargo.toml must pin alloc-no-stdlib = 2.0.4 as a direct dep, got:\n{content}"
    );
    assert!(
        content.contains("alloc-stdlib = \"=0.2.2\""),
        "NIF Cargo.toml must pin alloc-stdlib = 0.2.2 as a direct dep, got:\n{content}"
    );
    assert!(
        content.contains("brotli-decompressor = \"=5.0.1\""),
        "NIF Cargo.toml must pin brotli-decompressor = 5.0.1 as a direct dep, got:\n{content}"
    );

    assert!(
        !content.contains("[patch.crates-io]"),
        "NIF Cargo.toml MUST NOT emit a [patch.crates-io] block. \
         A patch entry with only `version` (no path/git/url) is a no-op and \
         cargo rejects it with `patch points to the same source`. Use direct \
         deps in [dependencies] instead. Got:\n{content}"
    );

    let deps_pos = content
        .find("[dependencies]")
        .expect("[dependencies] section must exist");
    let deps_section = &content[deps_pos..];
    for pin in [
        "alloc-no-stdlib = \"=2.0.4\"",
        "alloc-stdlib = \"=0.2.2\"",
        "brotli-decompressor = \"=5.0.1\"",
    ] {
        assert!(
            deps_section.contains(pin),
            "{pin} must appear inside [dependencies], got:\n{content}"
        );
    }

    assert!(
        content.contains("[package.metadata.cargo-machete]"),
        "NIF Cargo.toml must include cargo-machete metadata so the unused-dep \
         lint ignores the version-pin direct deps. Got:\n{content}"
    );
    for pin in ["alloc-no-stdlib", "alloc-stdlib", "brotli-decompressor"] {
        let needle = format!("\"{pin}\"");
        let machete_pos = content
            .find("[package.metadata.cargo-machete]")
            .expect("machete section must exist");
        let after_machete = &content[machete_pos..];
        assert!(
            after_machete.contains(&needle),
            "cargo-machete ignored list must contain {needle} — the NIF wrapper \
             never references the version-pin crate, so machete would otherwise \
             flag it. Got:\n{content}"
        );
    }
}
