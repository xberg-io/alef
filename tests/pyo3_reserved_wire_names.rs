use alef::{
    backends::pyo3::Pyo3Backend,
    core::{backend::Backend, config::NewAlefConfig, ir::*},
};

#[test]
fn non_raw_rust_keywords_are_legal_constructor_parameters() {
    let config: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["python"]
[[crates]]
name = "sample-core"
sources = ["src/lib.rs"]
"#,
    )
    .unwrap();
    let config = config.resolve().unwrap().remove(0);
    let api = ApiSurface {
        types: vec![TypeDef {
            name: "Coordinate".into(),
            rust_path: "sample_core::Coordinate".into(),
            fields: ["crate", "self", "Self", "super"]
                .into_iter()
                .enumerate()
                .map(|(index, name)| FieldDef {
                    name: format!("field_{index}"),
                    serde_rename: Some(name.into()),
                    ty: TypeRef::String,
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let files = Pyo3Backend.generate_bindings(&api, &config).unwrap();
    let code = &files.iter().find(|file| file.path.ends_with("lib.rs")).unwrap().content;
    for name in ["crate", "self", "Self", "super"] {
        assert!(!code.contains(&format!("r#{name}")), "{code}");
        assert!(code.contains(&format!("{name}_: String")), "{code}");
    }
    syn::parse_file(code).expect("generated Rust must parse");
}
