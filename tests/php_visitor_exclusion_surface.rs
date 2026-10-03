use alef::backends::php::PhpBackend;
use alef::core::backend::Backend;
use alef::core::config::{Language, NewAlefConfig, ReadmeConfig};
use alef::core::ir::{ApiSurface, TypeDef};
use std::collections::HashMap;

fn excluded_visitor_config() -> alef::core::config::ResolvedCrateConfig {
    let raw: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["php"]

[[crates]]
name = "sample"
sources = ["src/lib.rs"]

[crates.php]
extension_name = "sample"
exclude_functions = ["visitor"]

[[crates.trait_bridges]]
trait_name = "HtmlVisitor"
type_alias = "VisitorHandle"
param_name = "visitor"
bind_via = "options_field"
options_type = "ConversionOptions"
context_type = "NodeContext"
result_type = "VisitResult"
exclude_languages = ["php"]
"#,
    )
    .expect("PHP visitor exclusion config must parse");
    raw.resolve()
        .expect("PHP visitor exclusion config must resolve")
        .remove(0)
}

fn visitor_api() -> ApiSurface {
    ApiSurface {
        crate_name: "sample".to_string(),
        version: "1.0.0".to_string(),
        types: vec![
            TypeDef {
                name: "HtmlVisitor".to_string(),
                rust_path: "sample::HtmlVisitor".to_string(),
                is_trait: true,
                ..TypeDef::default()
            },
            TypeDef {
                name: "VisitorHandle".to_string(),
                rust_path: "sample::VisitorHandle".to_string(),
                is_opaque: true,
                ..TypeDef::default()
            },
            TypeDef {
                name: "ConversionResult".to_string(),
                rust_path: "sample::ConversionResult".to_string(),
                is_opaque: true,
                ..TypeDef::default()
            },
        ],
        ..ApiSurface::default()
    }
}

#[test]
fn php_visitor_exclusion_removes_runtime_stub_and_reference_surfaces() {
    let config = excluded_visitor_config();
    let api = visitor_api();
    let backend = PhpBackend;
    let mut files = backend.generate_bindings(&api, &config).expect("generate PHP runtime");
    files.extend(backend.generate_public_api(&api, &config).expect("generate PHP facade"));
    files.extend(backend.generate_type_stubs(&api, &config).expect("generate PHP stubs"));
    let docs = alef::docs::generate_docs(&api, &config, &[Language::Php], "docs").expect("generate PHP docs");
    files.extend(docs.into_iter().filter(|file| file.path.ends_with("api-php.md")));

    let output = files.iter().map(|file| file.content.as_str()).collect::<String>();
    assert!(
        output.contains("ConversionResult"),
        "the negative control must prove PHP output was generated"
    );
    for excluded in ["HtmlVisitor", "VisitorHandle"] {
        assert!(
            !output.contains(excluded),
            "PHP's visitor pseudo-function exclusion must remove `{excluded}` from runtime registration, stubs, and docs:\n{output}"
        );
    }
}

#[test]
fn php_visitor_exclusion_overrides_readme_visitor_feature_flag() {
    let temp = tempfile::tempdir().expect("create README template directory");
    std::fs::write(
        temp.path().join("php.md"),
        "visitor={{ features.visitor_pattern }}\n{% if features.visitor_pattern %}visitor API{% endif %}\n",
    )
    .expect("write README template");

    let mut config = excluded_visitor_config();
    config.workspace_root = Some(temp.path().to_path_buf());
    config.readme = Some(ReadmeConfig {
        template_dir: Some(".".into()),
        snippets_dir: None,
        config: None,
        output_pattern: None,
        discord_url: None,
        banner_url: None,
        languages: HashMap::from([(
            "php".to_string(),
            serde_json::json!({
                "template": "php.md",
                "features": { "visitor_pattern": true }
            }),
        )]),
        targets: HashMap::new(),
    });

    let files = alef::readme::generate_readmes(&visitor_api(), &config, &[Language::Php]).expect("generate PHP README");
    assert_eq!(files.len(), 1, "one PHP README must be generated");
    assert!(
        files[0].content.contains("visitor=False"),
        "the configured feature must be overridden:\n{}",
        files[0].content
    );
    assert!(
        !files[0].content.contains("visitor API"),
        "the visitor feature block must be omitted"
    );
}
