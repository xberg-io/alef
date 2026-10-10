use super::*;

#[test]
fn shared_rust_scaffolds_emit_configured_cargo_package_filters() {
    let config = test_config_from_toml(
        r#"
[crates.package_metadata.cargo]
include = ["src/**", "!src/**/tests.rs", "src/required/tests.rs"]
"#,
    );
    let languages = [
        Language::Ffi,
        Language::Elixir,
        Language::R,
        Language::Jni,
        Language::Ruby,
        Language::Php,
        Language::Node,
        Language::Python,
    ];

    let files = scaffold(&test_api(), &config, &languages).expect("shared Rust scaffolds");
    let manifests: Vec<_> = files.iter().filter(|file| file.path.ends_with("Cargo.toml")).collect();

    assert_eq!(
        manifests.len(),
        languages.len(),
        "every shared Rust scaffold must be examined"
    );
    for manifest in manifests {
        let parsed = toml::from_str::<toml::Value>(&manifest.content).expect("valid generated Cargo.toml");
        let include = parsed["package"]["include"].as_array().expect("package.include");
        assert_eq!(
            include,
            &vec![
                toml::Value::String("src/**".into()),
                toml::Value::String("!src/**/tests.rs".into()),
                toml::Value::String("src/required/tests.rs".into()),
            ],
            "{} must preserve Cargo filter order",
            manifest.path.display()
        );
    }
}
