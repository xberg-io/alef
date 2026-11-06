use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;

fn configuration(language: &str) -> NewAlefConfig {
    toml::from_str(&format!(
        r#"
[workspace]
languages = ["{language}"]
[[crates]]
name = "demo_client"
sources = ["src/lib.rs"]
[crates.e2e]
fixtures = "fixtures"
output = "e2e"
[crates.e2e.call]
function = "chat"
module = "demo_client"
result_is_simple = true
[crates.e2e.call.overrides.{language}]
function = "chat"
client_factory = "create_client"
php_client_factory = "createClient"
client_factory_from_json = "configured_client_json"
"#
    ))
    .expect("configuration parses")
}

#[test]
fn swift_e2e_preserves_configured_minimum_in_local_and_registry_modes() {
    for mode in ["local", "registry"] {
        let mut cfg = configuration("swift");
        cfg.crates[0].e2e.as_mut().unwrap().dep_mode = match mode {
            "local" => alef::e2e::config::DependencyMode::Local,
            _ => alef::e2e::config::DependencyMode::Registry,
        };
        let mut source = toml::to_string(&cfg).unwrap();
        source.push_str("\n[crates.swift]\nmin_macos_version = \"14.0\"\n");
        let cfg: NewAlefConfig = toml::from_str(&source).unwrap();
        let e2e = cfg.crates[0].e2e.clone().unwrap();
        let resolved = cfg.resolve().unwrap().remove(0);
        let files = alef::e2e::codegen::swift::SwiftE2eCodegen
            .generate(&[], &e2e, &resolved, &[], &[], &[], &[])
            .unwrap();
        let manifest = files
            .iter()
            .find(|f| f.path.file_name().is_some_and(|name| name == "Package.swift"))
            .unwrap();
        assert!(
            manifest.content.contains(".macOS(.v14)"),
            "{mode}: {}",
            manifest.content
        );
        assert!(!manifest.content.contains(".macOS(.v15)"), "{mode}: override ignored");
    }
}

#[test]
fn swift_e2e_default_minimum_is_the_declared_language_floor() {
    let cfg = configuration("swift");
    let e2e = cfg.crates[0].e2e.clone().unwrap();
    let resolved = cfg.resolve().unwrap().remove(0);
    let files = alef::e2e::codegen::swift::SwiftE2eCodegen
        .generate(&[], &e2e, &resolved, &[], &[], &[], &[])
        .unwrap();
    let manifest = files
        .iter()
        .find(|f| f.path.file_name().is_some_and(|name| name == "Package.swift"))
        .unwrap();
    assert!(manifest.content.contains(".macOS(.v15)"), "{}", manifest.content);
}

#[test]
fn swift_e2e_preserves_configured_ios_minimum_in_local_and_registry_modes() {
    for mode in [
        alef::e2e::config::DependencyMode::Local,
        alef::e2e::config::DependencyMode::Registry,
    ] {
        let mut cfg = configuration("swift");
        cfg.crates[0].e2e.as_mut().unwrap().dep_mode = mode;
        let mut source = toml::to_string(&cfg).unwrap();
        source.push_str("\n[crates.swift]\nmin_ios_version = \"17.0\"\n");
        let cfg: NewAlefConfig = toml::from_str(&source).unwrap();
        let e2e = cfg.crates[0].e2e.clone().unwrap();
        let resolved = cfg.resolve().unwrap().remove(0);
        let files = alef::e2e::codegen::swift::SwiftE2eCodegen
            .generate(&[], &e2e, &resolved, &[], &[], &[], &[])
            .unwrap();
        let manifest = files
            .iter()
            .find(|f| f.path.file_name().is_some_and(|name| name == "Package.swift"))
            .unwrap();
        assert!(
            manifest.content.contains(".iOS(.v17)"),
            "{mode:?}: the e2e package must declare the library's iOS minimum:\n{}",
            manifest.content
        );
    }
}
