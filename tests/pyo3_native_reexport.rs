use alef::{
    backends::pyo3::Pyo3Backend,
    core::{backend::Backend, config::NewAlefConfig, ir::*},
};

#[test]
fn native_reexport_does_not_emit_a_shadow_default_dataclass() {
    let config: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["python"]
[[crates]]
name = "sample-core"
sources = ["src/lib.rs"]
[crates.python]
reexported_types = ["Counts"]
"#,
    )
    .unwrap();
    let config = config.resolve().unwrap().remove(0);
    let api = ApiSurface {
        types: vec![
            TypeDef {
                name: "Counts".into(),
                rust_path: "sample_core::Counts".into(),
                has_default: true,
                fields: vec![FieldDef {
                    name: "value".into(),
                    ty: TypeRef::String,
                    ..Default::default()
                }],
                ..Default::default()
            },
            TypeDef {
                name: "Request".into(),
                rust_path: "sample_core::Request".into(),
                fields: vec![FieldDef {
                    name: "counts".into(),
                    ty: TypeRef::Named("Counts".into()),
                    ..Default::default()
                }],
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let files = Pyo3Backend.generate_public_api(&api, &config).unwrap();
    let options = &files
        .iter()
        .find(|file| file.path.ends_with("options.py"))
        .unwrap()
        .content;
    assert!(!options.contains("class Counts:"), "{options}");
    assert!(!options.contains("class Request:"), "{options}");
    let init = &files
        .iter()
        .find(|file| file.path.ends_with("__init__.py"))
        .unwrap()
        .content;
    assert!(!init.contains("from .options import Counts"), "{init}");
    assert!(init.contains("Counts"), "{init}");
    let mut defaults = config.clone();
    defaults.python.as_mut().unwrap().reexported_types.clear();
    let files = Pyo3Backend.generate_public_api(&api, &defaults).unwrap();
    let options = &files
        .iter()
        .find(|file| file.path.ends_with("options.py"))
        .unwrap()
        .content;
    assert!(options.contains("class Counts:"), "{options}");
    assert!(options.contains("class Request:"), "{options}");
}
