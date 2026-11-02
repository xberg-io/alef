use alef::backends::dart::DartBackend;
use alef::core::backend::Backend;
use alef::core::config::NewAlefConfig;
use alef::core::ir::{DefaultValue, PrimitiveType, TypeRef};

fn extracted_surface(root: &std::path::Path) -> alef::core::ir::ApiSurface {
    let path = root.join("lib.rs");
    std::fs::write(
        &path,
        r#"
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct NestedConfig { pub rows: Vec<Vec<u64>> }
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub enum Mode { #[default] Normal }
impl Default for NestedConfig {
    fn default() -> Self { Self { rows: vec![vec![1]] } }
}
pub fn inspect(config: NestedConfig) -> Vec<Vec<u64>> { config.rows }
"#,
    )
    .unwrap();
    alef::extract::extractor::extract(&[&path], "nested-defaults", "1.0.0", None).unwrap()
}

fn render(api: &alef::core::ir::ApiSurface) -> String {
    let config: NewAlefConfig = toml::from_str(
        "[workspace]\nlanguages=[\"dart\"]\n[[crates]]\nname=\"nested-defaults\"\nsources=[\"src/lib.rs\"]\n",
    )
    .unwrap();
    DartBackend
        .generate_bindings(api, &config.resolve().unwrap().remove(0))
        .unwrap()
        .into_iter()
        .find(|file| file.path.ends_with("nested_defaults.dart") && file.content.contains("config ??"))
        .expect("actual generated facade")
        .content
}

fn controls() -> Vec<(TypeRef, DefaultValue, &'static str)> {
    let u64_type = TypeRef::Primitive(PrimitiveType::U64);
    let number = DefaultValue::IntLiteral(1);
    vec![
        (
            TypeRef::Vec(Box::new(u64_type.clone())),
            DefaultValue::ListLiteral(vec![number.clone()]),
            "Uint64List.fromList([BigInt.parse('1')])",
        ),
        (
            TypeRef::Vec(Box::new(TypeRef::String)),
            DefaultValue::ListLiteral(vec![DefaultValue::StringLiteral("one".into())]),
            "const ['one']",
        ),
        (
            TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::Bool))),
            DefaultValue::ListLiteral(vec![DefaultValue::BoolLiteral(true)]),
            "const [true]",
        ),
        (
            TypeRef::Vec(Box::new(TypeRef::Vec(Box::new(TypeRef::String)))),
            DefaultValue::ListLiteral(vec![DefaultValue::ListLiteral(vec![DefaultValue::StringLiteral(
                "one".into(),
            )])]),
            "const [const ['one']]",
        ),
        (
            TypeRef::Vec(Box::new(TypeRef::Vec(Box::new(u64_type.clone())))),
            DefaultValue::ListLiteral(vec![DefaultValue::Empty]),
            "[Uint64List(0)]",
        ),
        (
            TypeRef::Vec(Box::new(TypeRef::Vec(Box::new(TypeRef::Vec(Box::new(
                u64_type.clone(),
            )))))),
            DefaultValue::ListLiteral(vec![DefaultValue::ListLiteral(vec![DefaultValue::ListLiteral(vec![
                number,
            ])])]),
            "[[Uint64List.fromList([BigInt.parse('1')])]]",
        ),
        (
            TypeRef::Vec(Box::new(u64_type.clone())),
            DefaultValue::Empty,
            "Uint64List(0)",
        ),
        (u64_type, DefaultValue::IntLiteral(1), "BigInt.parse('1')"),
        (TypeRef::String, DefaultValue::StringLiteral("one".into()), "'one'"),
        (
            TypeRef::Vec(Box::new(TypeRef::Named("Mode".into()))),
            DefaultValue::ListLiteral(vec![DefaultValue::EnumVariant("Normal".into())]),
            "const [Mode.normal]",
        ),
        (
            TypeRef::Vec(Box::new(TypeRef::Named("Mode".into()))),
            DefaultValue::ListLiteral(vec![DefaultValue::Empty]),
            "const [Mode.normal]",
        ),
        (
            TypeRef::Primitive(PrimitiveType::Bool),
            DefaultValue::BoolLiteral(true),
            "true",
        ),
        (TypeRef::Optional(Box::new(TypeRef::String)), DefaultValue::None, "null"),
    ]
}

#[test]
fn nested_typed_list_defaults_preserve_values_without_invalid_const_context() {
    let directory = tempfile::tempdir().unwrap();
    let api = extracted_surface(directory.path());
    let owner = api.types.iter().find(|ty| ty.name == "NestedConfig").unwrap();
    assert_eq!(
        owner.fields[0].typed_default,
        Some(DefaultValue::ListLiteral(vec![DefaultValue::ListLiteral(vec![
            DefaultValue::IntLiteral(1)
        ])]))
    );
    let facade = render(&api);
    if let Some(path) = std::env::var_os("ALEF_DART_NESTED_DEFAULT_ARTIFACT") {
        std::fs::write(path, &facade).unwrap();
    }
    let expected = "NestedConfig(rows: [Uint64List.fromList([BigInt.parse('1')])])";
    assert!(
        facade.contains(expected),
        "nested runtime default must remain non-const: expected {expected}\n{facade}"
    );
    let controls = controls();
    assert_eq!(controls.len(), 13);
    for (ty, default, expected) in controls {
        let mut control = api.clone();
        let owner = control.types.iter_mut().find(|ty| ty.name == "NestedConfig").unwrap();
        owner.fields[0].ty = ty;
        owner.fields[0].typed_default = Some(default);
        let facade = render(&control);
        assert!(
            facade.contains(&format!("NestedConfig(rows: {expected})")),
            "default control {expected}:\n{facade}"
        );
    }
}
