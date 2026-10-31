use alef::backends::dart::DartBackend;
use alef::core::backend::Backend;
use alef::core::config::NewAlefConfig;
use quote::ToTokens;

fn functions() -> Vec<syn::ItemFn> {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("lib.rs");
    std::fs::write(
        &source,
        r#"
        #[derive(Clone)] pub struct Wide(pub u64);
        pub struct Narrow(pub i32);
        pub fn scalar(value: Wide) -> Wide { value }
        pub fn narrow(value: Narrow) -> Narrow { value }
        pub fn borrowed(value: &Wide) -> Wide { value.clone() }
        pub fn optional(value: Option<Wide>) -> Option<Wide> { value }
        pub fn vector(value: Vec<Wide>) -> Vec<Wide> { value }
        pub fn map(value: std::collections::HashMap<String, Wide>) -> std::collections::HashMap<String, Wide> { value }
        pub fn tree(value: std::collections::BTreeMap<String, Wide>) -> std::collections::BTreeMap<String, Wide> { value }
        pub async fn fallible(value: Wide) -> Result<Wide, String> { Ok(value) }
    "#,
    )
    .unwrap();
    let api = alef::extract::extractor::extract(&[&source], "sample", "1.0.0", None).unwrap();
    assert_eq!(api.functions.len(), 8);
    assert!(
        api.functions
            .iter()
            .all(|f| f.params[0].newtype_wrapper.is_some() && f.return_newtype_wrapper.is_some())
    );
    let config: NewAlefConfig = toml::from_str(
        "[workspace]\nlanguages = [\"dart\"]\n[[crates]]\nname = \"sample\"\nsources = [\"src/lib.rs\"]\n",
    )
    .unwrap();
    let files = DartBackend
        .generate_bindings(&api, &config.resolve().unwrap().remove(0))
        .unwrap();
    let bridge = files
        .into_iter()
        .find(|file| file.path.ends_with("rust/src/lib.rs"))
        .unwrap();
    syn::parse_file(&bridge.content)
        .unwrap()
        .items
        .into_iter()
        .filter_map(|item| match item {
            syn::Item::Fn(f) => Some(f),
            _ => None,
        })
        .collect()
}

fn function(name: &str) -> syn::ItemFn {
    functions().into_iter().find(|f| f.sig.ident == name).unwrap()
}

#[test]
fn dart_function_scalar_newtypes_wrap_arguments_and_unwrap_results() {
    for (name, carrier, wrapper) in [("scalar", "u64", "Wide"), ("narrow", "i64", "Narrow")] {
        let f = function(name);
        let syn::FnArg::Typed(param) = f.sig.inputs.first().unwrap() else {
            panic!("typed parameter")
        };
        assert_eq!(param.ty.to_token_stream().to_string(), carrier);
        let syn::ReturnType::Type(_, result) = &f.sig.output else {
            panic!("typed return")
        };
        assert_eq!(result.to_token_stream().to_string(), carrier);
        let body = f.block.to_token_stream().to_string();
        assert!(
            body.contains(&format!("sample :: {wrapper} (")),
            "missing core constructor: {body}"
        );
        assert!(body.contains(". 0"), "missing returned newtype projection: {body}");
    }
}

#[test]
fn dart_function_container_newtypes_convert_both_sides() {
    for name in ["optional", "vector", "map", "tree"] {
        let f = function(name);
        let body = f.block.to_token_stream().to_string();
        assert!(body.contains("sample :: Wide"), "missing container constructor: {body}");
        assert!(body.contains(". 0"), "missing container newtype projection: {body}");
        assert!(body.contains(". map"), "missing per-element conversion: {body}");
    }
}

#[test]
fn dart_function_borrowed_newtype_borrows_the_wrapped_core_value() {
    let f = function("borrowed");
    let body = f.block.to_token_stream().to_string();
    assert!(
        body.contains("sample :: Wide ("),
        "missing borrowed core constructor: {body}"
    );
    assert!(body.contains("&"), "missing borrow: {body}");
    assert!(
        !body.contains("sample :: Wide (&"),
        "borrowed the primitive instead of the newtype: {body}"
    );
}

#[test]
fn dart_function_fallible_async_newtype_maps_only_the_success_value() {
    let f = function("fallible");
    let body = f.block.to_token_stream().to_string();
    assert!(
        body.contains("sample :: Wide ("),
        "missing argument constructor: {body}"
    );
    assert!(body.contains(". await"), "lost async call: {body}");
    assert!(body.contains(". map ("), "missing success mapping: {body}");
    assert!(body.contains(". map_err ("), "lost error conversion: {body}");
    assert!(body.contains(". 0"), "missing success newtype projection: {body}");
}
