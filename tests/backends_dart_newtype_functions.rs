use alef::backends::dart::DartBackend;
use alef::core::backend::Backend;
use alef::core::config::NewAlefConfig;
use quote::ToTokens;

#[path = "support/mise_task.rs"]
mod mise_task;

const SOURCE: &str = r#"
        #[derive(Clone)] pub struct Wide(pub u64);
        pub struct Narrow(pub i32);
        pub fn scalar(value: Wide) -> Wide { value }
        pub fn narrow(value: Narrow) -> Narrow { value }
        pub fn borrowed(value: &Wide) -> Wide { value.clone() }
        pub fn optional(value: Option<Wide>) -> Option<Wide> { value }
        pub fn vector(value: Vec<Wide>) -> Vec<Wide> { value }
        pub fn map(value: std::collections::HashMap<String, Wide>) -> std::collections::HashMap<String, Wide> { value }
        pub fn tree(value: std::collections::BTreeMap<String, Wide>) -> std::collections::BTreeMap<String, Wide> { value }
        pub fn narrow_map(value: std::collections::HashMap<String, Narrow>) -> std::collections::HashMap<String, Narrow> { value }
        pub fn nested(value: Vec<Option<Narrow>>) -> Vec<Option<Narrow>> { value }
        pub struct Location(pub std::path::PathBuf);
        pub fn path(value: Location) -> Location { value }
        pub struct Delay(pub std::time::Duration);
        pub fn duration(value: Delay) -> Delay { value }
        pub struct Label(pub String);
        pub fn borrowed_label(value: &Label) -> &Label { value }
        pub fn borrowed_labels(value: &[Label]) -> &[Label] { value }
        pub fn borrowed_label_map(value: &std::collections::HashMap<String, Label>) -> &std::collections::HashMap<String, Label> { value }
        pub fn fallible_optional(value: Option<Wide>) -> Result<Option<Wide>, String> { Ok(value) }
        pub async fn fallible(value: Wide) -> Result<Wide, String> { Ok(value) }
    "#;

fn functions() -> Vec<syn::ItemFn> {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("lib.rs");
    std::fs::write(&source, SOURCE).unwrap();
    let api = alef::extract::extractor::extract(&[&source], "sample", "1.0.0", None).unwrap();
    assert_eq!(api.functions.len(), 16);
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
    for name in ["optional", "vector", "map", "tree", "narrow_map", "nested"] {
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

#[test]
fn dart_borrowed_newtype_results_own_leaf_values_without_cloning_wrappers() {
    for name in ["borrowed_label", "borrowed_labels", "borrowed_label_map"] {
        let f = function(name);
        let body = f.block.to_token_stream().to_string();
        assert!(body.contains("to_owned"), "borrowed leaf was moved: {body}");
        assert!(
            !body.contains("sample :: Label as"),
            "the source wrapper does not implement Clone: {body}"
        );
    }
}

#[test]
fn dart_mutable_newtype_references_are_rejected_without_losing_writeback() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("lib.rs");
    std::fs::write(
        &source,
        "pub struct Wide(pub u64); pub fn bump(value: &mut Wide) { value.0 += 1; }",
    )
    .unwrap();
    let api = alef::extract::extractor::extract(&[&source], "sample", "1.0.0", None).unwrap();
    assert_eq!(api.functions.len(), 1);
    assert!(api.functions[0].params[0].is_mut && api.functions[0].params[0].newtype_wrapper.is_some());
    let config: NewAlefConfig = toml::from_str(
        "[workspace]\nlanguages = [\"dart\"]\n[[crates]]\nname = \"sample\"\nsources = [\"src/lib.rs\"]\n",
    )
    .unwrap();
    let error = DartBackend
        .generate_bindings(&api, &config.resolve().unwrap().remove(0))
        .unwrap_err();
    assert!(
        error.to_string().contains("newtype") && error.to_string().contains("writeback"),
        "{error:#}"
    );
}

#[test]
fn dart_newtype_source_and_generated_functions_typecheck_and_execute_together() {
    use alef::snippets::validators::run_command_streams;
    let directory = tempfile::tempdir().unwrap();
    let functions = functions();
    assert_eq!(functions.len(), 16);
    let declarations = functions
        .iter()
        .map(|f| f.to_token_stream().to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let program = format!(
        "mod sample {{ {SOURCE} }}\n{declarations}\nfn main() {{
        assert_eq!(scalar(u64::MAX), u64::MAX);
        assert_eq!(narrow(i32::MAX as i64), i32::MAX as i64);
        assert_eq!(optional(Some(u64::MAX)), Some(u64::MAX));
        assert_eq!(fallible_optional(Some(u64::MAX)).unwrap(), Some(u64::MAX));
        assert_eq!(vector(vec![u64::MAX]), vec![u64::MAX]);
        assert_eq!(borrowed_label(String::from(\"keep me\")), \"keep me\");
        assert_eq!(borrowed_labels(vec![String::from(\"keep me\")]), vec![String::from(\"keep me\")]);
        let values = std::collections::HashMap::from([(String::from(\"a\"), String::from(\"b\"))]);
        assert_eq!(borrowed_label_map(values.clone()), values);
        assert_eq!(narrow_map(std::collections::HashMap::from([(String::from(\"a\"), -1)]))[\"a\"], -1);
        assert_eq!(nested(vec![Some(-1), None]), vec![Some(-1), None]);
        assert_eq!(path(String::from(\"kept\")), \"kept\");
        assert_eq!(duration(42), 42);
    }}"
    );
    std::fs::write(directory.path().join("probe.rs"), program).unwrap();
    std::fs::write(
        directory.path().join("rust-toolchain.toml"),
        include_str!("../rust-toolchain.toml"),
    )
    .unwrap();
    std::fs::write(
        directory.path().join("mise.toml"),
        format!(
            "[tasks.compile]\nrun = 'rustc --edition=2024 probe.rs -o probe{0}'\n[tasks.execute]\nrun = './probe{0}'\n",
            std::env::consts::EXE_SUFFIX
        ),
    )
    .unwrap();
    for task in ["compile", "execute"] {
        let mut command = mise_task::mise_run(directory.path(), task);
        let output = run_command_streams(&mut command, 60).unwrap();
        assert!(output.success, "{task}: {}\n{}", output.stdout, output.stderr);
    }
}

#[test]
fn function_container_metadata_keeps_wire_types_and_legacy_field_method_policy() {
    use alef::core::ir::{NewtypeContainer, NewtypeWrapper, PrimitiveType, TypeRef};
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("lib.rs");
    std::fs::write(
        &source,
        r#"
        pub struct Wide(pub u64);
        pub struct Holder { pub values: Vec<Wide> }
        impl Holder { pub fn pass(value: Vec<Wide>) -> Vec<Wide> { value } }
        pub fn scalar(value: Wide) -> Wide { value }
        pub fn optional(value: Option<Wide>) -> Option<Wide> { value }
        pub fn nested(value: Vec<Option<Wide>>) -> Vec<Option<Wide>> { value }
        pub fn maps(value: std::collections::HashMap<Wide, Wide>) -> std::collections::HashMap<Wide, Wide> { value }
    "#,
    )
    .unwrap();
    let api = alef::extract::extractor::extract(&[&source], "sample", "1.0.0", None).unwrap();
    assert_eq!(api.functions.len(), 4);
    let scalar = api.functions.iter().find(|f| f.name == "scalar").unwrap();
    assert_eq!(scalar.params[0].newtype_wrapper.as_deref(), Some("sample::Wide"));
    assert_eq!(scalar.return_newtype_wrapper.as_deref(), Some("sample::Wide"));
    let nested = api.functions.iter().find(|f| f.name == "nested").unwrap();
    let expected = TypeRef::Vec(Box::new(TypeRef::Optional(Box::new(TypeRef::Primitive(
        PrimitiveType::U64,
    )))));
    assert_eq!(nested.params[0].ty, expected);
    assert_eq!(nested.return_type, expected);
    for name in ["optional", "nested", "maps"] {
        let f = api.functions.iter().find(|f| f.name == name).unwrap();
        for metadata in [&f.params[0].newtype_wrapper, &f.return_newtype_wrapper] {
            let paths = NewtypeWrapper::decode(metadata.as_deref().expect("container metadata")).unwrap();
            let expected = match name {
                "optional" => vec![vec![NewtypeContainer::Optional]],
                "nested" => vec![vec![NewtypeContainer::Vec, NewtypeContainer::Optional]],
                _ => vec![vec![NewtypeContainer::MapKey], vec![NewtypeContainer::MapValue]],
            };
            assert_eq!(
                paths
                    .explicit_paths()
                    .iter()
                    .map(|p| p.containers.clone())
                    .collect::<Vec<_>>(),
                expected
            );
        }
    }
    let holder = api.types.iter().find(|ty| ty.name == "Holder").unwrap();
    assert_eq!(holder.fields[0].newtype_wrapper.as_deref(), Some("sample::Wide"));
    assert!(holder.methods[0].params[0].newtype_wrapper.is_none());
    assert!(holder.methods[0].return_newtype_wrapper.is_none());
}

#[test]
fn borrowed_private_newtypes_require_extracted_clone_proof() {
    for clone in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("lib.rs");
        let derive = if clone { "#[derive(Clone)]" } else { "" };
        std::fs::write(
            &source,
            format!(
                r#"
            {derive}
            #[alef(transparent_string(from = "from", into = "into_inner"))]
            pub struct Secret(String);
            impl Secret {{
                pub fn from(value: String) -> Self {{ Self(value) }}
                pub fn into_inner(self) -> String {{ self.0 }}
            }}
            pub fn borrowed(value: &Secret) -> &Secret {{ value }}
        "#
            ),
        )
        .unwrap();
        let api = alef::extract::extractor::extract(&[&source], "sample", "1.0.0", None).unwrap();
        assert_eq!(api.functions.len(), 1);
        assert!(api.functions[0].returns_ref && api.functions[0].return_newtype_wrapper.is_some());
        let config: NewAlefConfig = toml::from_str(
            "[workspace]\nlanguages = [\"dart\"]\n[[crates]]\nname = \"sample\"\nsources = [\"src/lib.rs\"]\n",
        )
        .unwrap();
        let generated = DartBackend.generate_bindings(&api, &config.resolve().unwrap().remove(0));
        if clone {
            let bridge = generated
                .unwrap()
                .into_iter()
                .find(|f| f.path.ends_with("rust/src/lib.rs"))
                .unwrap();
            assert!(bridge.content.contains("as Clone>::clone"));
        } else {
            assert!(generated.unwrap_err().to_string().contains("proven Clone"));
        }
    }
}
