use super::*;
use std::path::Path;

const ALEF_TOML: &str = r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "sample"
sources = ["src/lib.rs"]

[[crates.component_contracts]]
name = "ocr"
trait_path = "sample_core::OcrBackend"

[[crates.components]]
name = "tesseract"
provides = [{ contract = "ocr", implementation = "sample_components::TesseractBackend" }]
features = ["ocr-tesseract"]
source_manifest = "components/crates/x-web/Cargo.toml"
producer_dir = "components/crates/x-web-component"
"#;

fn resolved() -> Vec<crate::core::config::ResolvedCrateConfig> {
    toml::from_str::<crate::core::config::NewAlefConfig>(ALEF_TOML)
        .unwrap()
        .resolve()
        .unwrap()
}

fn rebased(config_path: &str, cwd: &str) -> (String, String) {
    let mut resolved = resolved();
    rebase_component_paths(&mut resolved, Path::new(config_path), Path::new(cwd));
    let component = &resolved[0].components[0];
    (
        component.source_manifest.clone().unwrap(),
        component.producer_dir.clone().unwrap(),
    )
}

#[test]
fn every_spelling_of_a_config_in_the_working_directory_leaves_paths_unchanged() {
    for spelling in ["alef.toml", "./alef.toml", "/repo/alef.toml"] {
        assert_eq!(
            rebased(spelling, "/repo"),
            (
                "components/crates/x-web/Cargo.toml".to_string(),
                "components/crates/x-web-component".to_string()
            ),
            "{spelling}"
        );
    }
}

#[test]
fn config_in_a_subdirectory_roots_paths_there_for_every_spelling() {
    for spelling in [
        "sub/alef.toml",
        "./sub/alef.toml",
        "/repo/sub/alef.toml",
        "sub/../sub/alef.toml",
    ] {
        assert_eq!(
            rebased(spelling, "/repo"),
            (
                "sub/components/crates/x-web/Cargo.toml".to_string(),
                "sub/components/crates/x-web-component".to_string()
            ),
            "{spelling}"
        );
    }
}

#[test]
fn config_outside_the_working_directory_uses_parent_segments() {
    assert_eq!(
        rebased("../alef.toml", "/repo/sub").1,
        "../components/crates/x-web-component"
    );
    assert_eq!(
        rebased("/repo/alef.toml", "/repo/sub").1,
        "../components/crates/x-web-component"
    );
}

#[test]
fn load_config_rebases_component_paths_exactly_once() {
    let dir = tempfile::tempdir().unwrap();
    let sub = dir.path().join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    let config_path = sub.join("alef.toml");
    std::fs::write(&config_path, ALEF_TOML).unwrap();

    let (_, resolved) = load_config(&config_path).unwrap();

    let config_dir =
        crate::core::config::component::config_dir_relative_to(&config_path, &std::env::current_dir().unwrap());
    let expected = |tail: &str| config_dir.join(tail).to_string_lossy().replace('\\', "/");
    let component = &resolved[0].components[0];
    assert_eq!(
        component.source_manifest.as_deref(),
        Some(expected("components/crates/x-web/Cargo.toml").as_str())
    );
    assert_eq!(
        component.producer_dir.as_deref(),
        Some(expected("components/crates/x-web-component").as_str())
    );
    assert!(
        component
            .producer_dir
            .as_deref()
            .unwrap()
            .ends_with("sub/components/crates/x-web-component"),
        "{:?}",
        component.producer_dir
    );
}
