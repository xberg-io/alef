use super::*;

const COMPONENT: &str = r#"
[[crates.component_contracts]]
name = "extractor"
trait_path = "my_lib::Extractor"
interface_version = 1

[[crates.components]]
name = "pdf"
provides = [{ contract = "extractor", implementation = "my_lib::PdfExtractor" }]
features = ["pdf"]
default_features = false
targets = ["x86_64-unknown-linux-gnu"]
"#;

#[test]
fn components_only_crate_scaffolds_nothing() {
    let cfg: NewAlefConfig = toml::from_str(&format!(
        "[workspace]\n\n[[crates]]\nname = \"my-lib\"\nsources = [\"src/lib.rs\"]\n{COMPONENT}"
    ))
    .expect("valid toml");
    let config = cfg.resolve().expect("resolve ok").remove(0);

    assert!(config.is_components_only());
    assert!(scaffold(&test_api(), &config, &[]).unwrap().is_empty());
}

#[test]
fn crate_with_a_language_and_components_still_gets_the_scaffold() {
    let config = minimal_config_from_toml(COMPONENT);

    assert!(!config.is_components_only());
    let files = scaffold(&test_api(), &config, &[Language::Python]).unwrap();
    for expected in ["poly.toml", ".gitattributes"] {
        assert!(
            files.iter().any(|file| file.path == Path::new(expected)),
            "{expected} must still be scaffolded when a binding language is configured"
        );
    }
}
