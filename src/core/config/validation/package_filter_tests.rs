use super::validate_resolved;
use crate::core::config::new_config::NewAlefConfig;

fn resolve_first(toml: &str) -> crate::core::config::ResolvedCrateConfig {
    let config: NewAlefConfig = toml::from_str(toml).expect("config should parse");
    config.resolve().expect("config should resolve").remove(0)
}

fn base_config() -> &'static str {
    r#"
[workspace]
languages = ["python"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
"#
}

#[test]
fn cargo_package_include_and_exclude_are_mutually_exclusive() {
    let toml = format!(
        "{base}\n[crates.package_metadata.cargo]\ninclude = [\"src/**\"]\nexclude = [\"tests/**\"]\n",
        base = base_config()
    );
    let config = resolve_first(&toml);
    let error = validate_resolved(&config).expect_err("Alef rejects ambiguous include and exclude filters");
    assert!(
        error.to_string().contains("package_metadata.cargo.include")
            && error.to_string().contains("mutually exclusive"),
        "got: {error}"
    );
}

#[test]
fn cargo_package_filters_reject_blank_and_control_character_patterns() {
    for pattern in ["", "   ", "src/**\nsecret"] {
        let toml = format!(
            "{base}\n[crates.package_metadata.cargo]\ninclude = [{pattern:?}]\n",
            base = base_config()
        );
        let config = resolve_first(&toml);
        let error = validate_resolved(&config).expect_err("unsafe Cargo package pattern must fail");
        assert!(error.to_string().contains("empty or control-character"), "got: {error}");
    }
}
