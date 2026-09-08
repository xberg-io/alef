use alef::backends::magnus::MagnusBackend;
use alef::core::backend::Backend;
use alef::core::config::new_config::NewAlefConfig;
use alef::core::ir::*;

fn generated_message(tag: Option<&str>, content: Option<&str>, untagged: bool, tuple: bool) -> syn::ItemEnum {
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
        enums: vec![EnumDef {
            name: "Message".into(),
            rust_path: "fixture::Message".into(),
            serde_tag: tag.map(str::to_owned),
            serde_content: content.map(str::to_owned),
            serde_untagged: untagged,
            variants: vec![EnumVariant {
                name: "User".into(),
                serde_rename: Some("user".into()),
                is_tuple: tuple,
                fields: vec![FieldDef {
                    name: "_0".into(),
                    ty: TypeRef::Named("UserMessage".into()),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        }],
        types: vec![TypeDef {
            name: "UserMessage".into(),
            rust_path: "fixture::UserMessage".into(),
            fields: vec![FieldDef {
                name: "content".into(),
                ty: TypeRef::String,
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
            syn::Item::Enum(item) if item.ident == "Message" => Some(item),
            _ => None,
        })
        .expect("actual backend must emit Message")
}

fn has_flatten(field: &syn::Field) -> bool {
    field.attrs.iter().any(|attr| {
        attr.path().is_ident("serde") && attr.parse_args::<syn::Ident>().is_ok_and(|ident| ident == "flatten")
    })
}

#[test]
fn internally_tagged_newtype_preserves_flat_payload_in_actual_backend() {
    let item = generated_message(Some("role"), None, false, true);
    let field = item.variants[0].fields.iter().next().unwrap();
    assert!(matches!(item.variants[0].fields, syn::Fields::Named(_)));
    assert!(has_flatten(field), "synthetic _0 must not become a required wire key");
}

#[test]
fn other_enum_representations_do_not_flatten_payload() {
    for (tag, content, untagged, tuple) in [
        (Some("role"), Some("payload"), false, true),
        (None, None, true, true),
        (None, None, false, true),
        (Some("role"), None, false, false),
    ] {
        let item = generated_message(tag, content, untagged, tuple);
        assert_eq!(item.variants.len(), 1);
        assert!(item.variants[0].fields.iter().all(|field| !has_flatten(field)));
    }
}
