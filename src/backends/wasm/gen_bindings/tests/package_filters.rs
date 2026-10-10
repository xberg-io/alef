use super::{empty_api, gen_cargo_toml};
use crate::core::config::NewAlefConfig;

#[test]
fn cargo_toml_emits_configured_package_exclude() {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["wasm"]
[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
[crates.package_metadata.cargo]
exclude = ["tests/**", "benches/**"]
"#,
    )
    .unwrap();
    let config = cfg.resolve().unwrap().remove(0);
    let manifest = toml::from_str::<toml::Value>(&gen_cargo_toml(&empty_api(), &config))
        .expect("generated Cargo.toml must be valid TOML");
    assert_eq!(
        manifest["package"]["exclude"].as_array().expect("exclude"),
        &vec![
            toml::Value::String("tests/**".into()),
            toml::Value::String("benches/**".into()),
        ]
    );
    assert!(manifest["package"].get("include").is_none());
}

#[test]
fn cargo_toml_emits_configured_authors() {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["wasm"]
[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
[crates.package_metadata]
authors = ["Ada Lovelace <ada@example.com>"]
"#,
    )
    .unwrap();
    let config = cfg.resolve().unwrap().remove(0);
    let manifest = toml::from_str::<toml::Value>(&gen_cargo_toml(&empty_api(), &config))
        .expect("generated Cargo.toml must be valid TOML");
    assert_eq!(
        manifest["package"]["authors"].as_array().expect("authors"),
        &vec![toml::Value::String("Ada Lovelace <ada@example.com>".into())]
    );
}
