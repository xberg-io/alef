use alef::backends::magnus::MagnusBackend;
use alef::core::backend::Backend;
use alef::core::config::new_config::NewAlefConfig;
use alef::core::ir::*;

fn generated_field(renamed: Option<&str>, rename_all: Option<&str>) -> syn::Field {
    let config: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["ruby"]
[[crates]]
name = "fixture"
sources = ["src/lib.rs"]
"#,
    )
    .unwrap();
    let api = ApiSurface {
        crate_name: "fixture".into(),
        types: vec![TypeDef {
            name: "Tool".into(),
            rust_path: "fixture::Tool".into(),
            serde_rename_all: rename_all.map(str::to_owned),
            fields: vec![FieldDef {
                name: "tool_type".into(),
                ty: TypeRef::String,
                serde_rename: renamed.map(str::to_owned),
                ..Default::default()
            }],
            has_serde: true,
            ..Default::default()
        }],
        ..Default::default()
    };
    let files = MagnusBackend
        .generate_bindings(&api, &config.resolve().unwrap().remove(0))
        .unwrap();
    files
        .iter()
        .filter(|file| file.path.ends_with("lib.rs"))
        .flat_map(|file| syn::parse_file(&file.content).unwrap().items)
        .find_map(|item| match item {
            syn::Item::Struct(item) if item.ident == "Tool" => item.fields.into_iter().next(),
            _ => None,
        })
        .expect("actual backend must emit Tool field")
}

#[test]
fn actual_backend_preserves_explicit_and_container_field_wire_names() {
    for (renamed, rename_all, expected) in [
        (Some("type"), None, "type"),
        (None, Some("camelCase"), "toolType"),
        (Some("type"), Some("camelCase"), "type"),
        (Some("quoted\"key"), None, "quoted\"key"),
    ] {
        let field = generated_field(renamed, rename_all);
        assert_eq!(field.ident.as_ref().unwrap(), "tool_type");
        let attr = field
            .attrs
            .iter()
            .find(|attr| attr.path().is_ident("serde"))
            .expect("wire rename required");
        let arg = attr.parse_args::<syn::MetaNameValue>().unwrap();
        assert!(arg.path.is_ident("rename"));
        let syn::Expr::Lit(value) = arg.value else {
            panic!("rename must be literal")
        };
        let syn::Lit::Str(value) = value.lit else {
            panic!("rename must be string")
        };
        assert_eq!(value.value(), expected);
    }
}

#[test]
fn actual_backend_keeps_unrenamed_field_without_redundant_serde_attribute() {
    let field = generated_field(None, None);
    assert_eq!(field.ident.unwrap(), "tool_type");
    assert!(field.attrs.iter().all(|attr| !attr.path().is_ident("serde")));
}
