use alef::core::config::NewAlefConfig;
use alef::{ApiSurface, Language};
use semver::{Version, VersionReq};
use std::path::Path;

fn scaffold_php_manifest(existing_requirement: Option<&str>) -> String {
    let workspace = tempfile::tempdir().expect("create isolated consumer workspace");
    std::fs::write(workspace.path().join("Cargo.toml"), "[workspace]\nmembers = []\n")
        .expect("write isolated workspace manifest");
    let manifest_path = Path::new("crates/sample-core-php/Cargo.toml");
    if let Some(requirement) = existing_requirement {
        let path = workspace.path().join(manifest_path);
        std::fs::create_dir_all(path.parent().expect("manifest parent")).expect("create PHP crate directory");
        std::fs::write(&path, format!("[dependencies]\next-php-rs = \"{requirement}\"\n"))
            .expect("write existing consumer requirement");
    }
    let parsed: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["php"]

[[crates]]
name = "sample-core"
sources = ["src/lib.rs"]

[crates.php]
extension_name = "sample_core"
"#,
    )
    .expect("parse consumer configuration");
    let mut config = parsed.resolve().expect("resolve consumer configuration").remove(0);
    config.workspace_root = Some(workspace.path().to_path_buf());
    let api = ApiSurface {
        crate_name: "sample-core".to_string(),
        version: "1.0.0".to_string(),
        ..ApiSurface::default()
    };
    let files = alef::scaffold::scaffold(&api, &config, &[Language::Php]).expect("scaffold PHP files");
    let manifests: Vec<_> = files.iter().filter(|file| file.path == manifest_path).collect();
    assert_eq!(
        manifests.len(),
        1,
        "PHP scaffold must emit exactly one binding manifest"
    );
    manifests[0].content.clone()
}

fn dependency_requirement(manifest: &str) -> String {
    let parsed: toml::Value = toml::from_str(manifest).expect("generated manifest must parse");
    parsed["dependencies"]["ext-php-rs"]
        .as_str()
        .expect("generated PHP dependency must carry a version requirement")
        .to_owned()
}

#[test]
fn new_php_manifest_requires_ext_php_rs_with_the_fixed_zip_dependency() {
    let manifest = scaffold_php_manifest(None);
    let requirement = dependency_requirement(&manifest);
    assert_eq!(requirement, "0.16.4", "generated PHP manifest:\n{manifest}");
    let requirement = VersionReq::parse(&requirement).expect("valid Cargo requirement");
    for patch in [1, 2, 3] {
        assert!(!requirement.matches(&Version::new(0, 16, patch)));
    }
    assert!(requirement.matches(&Version::new(0, 16, 4)));
    assert!(requirement.matches(&Version::new(0, 16, 5)));
}

#[test]
fn regenerated_php_manifest_raises_an_older_ext_php_rs_requirement() {
    let manifest = scaffold_php_manifest(Some("0.16.3"));
    assert_eq!(dependency_requirement(&manifest), "0.16.4", "{manifest}");
}

#[test]
fn regenerated_php_manifest_preserves_a_newer_ext_php_rs_requirement() {
    let manifest = scaffold_php_manifest(Some("0.16.9"));
    assert_eq!(dependency_requirement(&manifest), "0.16.9", "{manifest}");
}
