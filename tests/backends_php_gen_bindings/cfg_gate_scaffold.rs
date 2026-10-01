use alef::core::config::Language;
use alef::core::config::new_config::NewAlefConfig;
use alef::core::ir::{ApiSurface, FunctionDef, ParamDef, PrimitiveType, TypeRef};
use alef::scaffold::scaffold;

#[test]
fn php_cargo_toml_requires_and_publicly_forwards_function_gated_core_features() {
    let toml = r#"
[workspace]
languages = ["php"]

[[crates]]
name = "demo-crate"
sources = ["src/lib.rs"]

[crates.php]
extension_name = "demo_crate"
"#;
    let cfg: NewAlefConfig = toml::from_str(toml).expect("config must parse");
    let resolved = cfg.resolve().expect("config must resolve");
    let config = &resolved[0];

    let api = ApiSurface {
        crate_name: config.name.clone(),
        version: "1.0.0".into(),
        functions: vec![FunctionDef {
            name: "count_tokens".to_string(),
            rust_path: "demo_crate::count_tokens".to_string(),
            params: vec![ParamDef {
                name: "text".to_string(),
                ty: TypeRef::String,
                ..ParamDef::default()
            }],
            return_type: TypeRef::Primitive(PrimitiveType::U32),
            cfg: Some(r#"feature = "tokenizer""#.to_string()),
            ..FunctionDef::default()
        }],
        ..Default::default()
    };

    let files = scaffold(&api, config, &[Language::Php]).expect("PHP scaffold must succeed");
    let cargo_toml = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("-php/Cargo.toml"))
        .expect("php crate Cargo.toml must be scaffolded");
    let content = &cargo_toml.content;

    assert!(
        content.contains(r#"features = ["tokenizer"]"#),
        "core dependency must unconditionally request the function-gated feature, got:\n{content}"
    );
    assert!(
        content.contains(r#"tokenizer = ["demo-crate/tokenizer"]"#),
        "function-gated feature must remain a public php-crate feature, got:\n{content}"
    );
    assert!(
        content.contains("default = [\"tokenizer\"]"),
        "function-gated feature must remain in the php crate's `default` feature list, got:\n{content}"
    );
}
