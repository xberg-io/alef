use alef::core::config::Language;
/// Test that the PHP scaffold emits correct PIE download URL templates with:
/// - {Version} placeholder (preserves leading `v` from Composer version field)
/// - {OSLower} placeholder (generates lowercase OS names: linux, darwin)
///
/// This ensures `pie install kreuzberg-dev/html-to-markdown` can resolve pre-packaged
/// extension binaries from GitHub Release assets.
use alef::core::config::new_config::NewAlefConfig;
use alef::core::ir::ApiSurface;
use alef::scaffold::scaffold;

#[test]
fn test_php_scaffold_pie_url_template_with_repository() {
    let toml = r#"
[workspace]
languages = ["php"]

[[crates]]
name = "html-to-markdown"
sources = ["src/lib.rs"]

[crates.package_metadata]
repository = "https://github.com/kreuzberg-dev/html-to-markdown"

[crates.php]
extension_name = "html_to_markdown"
"#;

    let cfg: NewAlefConfig = toml::from_str(toml).expect("config must parse");
    let resolved = cfg.resolve().expect("config must resolve");
    let config = &resolved[0];

    let api = ApiSurface {
        crate_name: config.name.clone(),
        version: "3.6.9".into(),
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

    let files = scaffold(&api, config, &[Language::Php]).expect("PHP scaffold must succeed");

    // Find composer.json in the scaffolded files
    let composer_json = files
        .iter()
        .find(|f| f.path.ends_with("composer.json"))
        .expect("composer.json must be scaffolded");

    let content = &composer_json.content;

    // Verify the URL template exists with correct placeholders
    assert!(
        content.contains("\"url-template\": \"https://github.com/kreuzberg-dev/html-to-markdown/releases/download/{Version}/php_html_to_markdown-{Version}_php{PhpVersion}-{Arch}-{OSLower}-{Libc}-{TSMode}.tgz\""),
        "composer.json must contain PIE URL template with {{Version}} and {{OSLower}} placeholders.\nActual content:\n{}",
        content
    );

    // Ensure old broken pattern is NOT present
    assert!(
        !content.contains("{OS}-"),
        "composer.json must NOT contain unquoted {{OS}} placeholder (should be {{OSLower}})\nActual content:\n{}",
        content
    );
}

#[test]
fn test_php_scaffold_pie_url_omitted_without_repository() {
    let toml = r#"
[workspace]
languages = ["php"]

[[crates]]
name = "html-to-markdown"
sources = ["src/lib.rs"]

[crates.php]
extension_name = "html_to_markdown"
"#;

    let cfg: NewAlefConfig = toml::from_str(toml).expect("config must parse");
    let resolved = cfg.resolve().expect("config must resolve");
    let config = &resolved[0];

    let api = ApiSurface {
        crate_name: config.name.clone(),
        version: "3.6.9".into(),
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

    let files = scaffold(&api, config, &[Language::Php]).expect("PHP scaffold must succeed");

    let composer_json = files
        .iter()
        .find(|f| f.path.ends_with("composer.json"))
        .expect("composer.json must be scaffolded");

    let content = &composer_json.content;

    // When no repository is configured, extra.pie should be omitted entirely
    assert!(
        !content.contains("\"pie\":"),
        "composer.json must omit extra.pie block when no repository is configured"
    );
}
