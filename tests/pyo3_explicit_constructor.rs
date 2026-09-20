use alef::backends::pyo3::Pyo3Backend;
use alef::core::backend::Backend;
use alef::core::config::NewAlefConfig;
use alef::core::ir::{ApiSurface, MethodDef, TypeDef, TypeRef};

fn generated(has_default: bool, method: MethodDef) -> String {
    let config: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["python"]
[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
[crates.python]
module_name = "_test_lib"
"#,
    )
    .unwrap();
    let config = config.resolve().unwrap().remove(0);
    let typ = TypeDef {
        name: "Handle".into(),
        rust_path: "test_lib::Handle".into(),
        is_opaque: true,
        is_clone: true,
        has_default,
        methods: vec![method],
        ..Default::default()
    };
    let api = ApiSurface {
        crate_name: "test_lib".into(),
        version: "0.1.0".into(),
        types: vec![typ],
        ..Default::default()
    };
    let files = Pyo3Backend.generate_bindings(&api, &config).unwrap();
    files
        .into_iter()
        .find(|file| file.path.ends_with("lib.rs"))
        .unwrap()
        .content
}

fn constructor() -> MethodDef {
    MethodDef {
        name: "new".into(),
        is_static: true,
        return_type: TypeRef::Named("Handle".into()),
        ..Default::default()
    }
}

#[test]
fn explicit_constructor_preserves_no_default_contract() {
    for has_default in [false, true] {
        let content = generated(has_default, constructor());
        assert_eq!(content.contains("impl Default for Handle"), has_default);
        assert_eq!(content.contains("#[allow(clippy::new_without_default)]"), !has_default);
        assert!(!content.contains("#![allow(clippy::new_without_default)]"));
        let syntax = syn::parse_file(&content).unwrap();
        let wrapper = syntax
            .items
            .iter()
            .find_map(|item| match item {
                syn::Item::Impl(item)
                    if item.trait_.is_none() && item.attrs.iter().any(|a| a.path().is_ident("pymethods")) =>
                {
                    Some(item)
                }
                _ => None,
            })
            .unwrap();
        assert_eq!(wrapper.attrs.iter().any(|a| a.path().is_ident("allow")), !has_default);
    }
}

#[test]
fn unrelated_factories_do_not_receive_the_allowance() {
    let mut renamed = constructor();
    renamed.name = "create".into();
    let mut unit = constructor();
    unit.return_type = TypeRef::Unit;
    let mut fallible = constructor();
    fallible.error_type = Some("String".into());
    for method in [renamed, unit, fallible] {
        let content = generated(false, method);
        assert!(!content.contains("clippy::new_without_default"));
        assert!(!content.contains("impl Default for Handle"));
    }
}
