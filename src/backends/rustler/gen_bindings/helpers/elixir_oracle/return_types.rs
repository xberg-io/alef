use crate::backends::rustler::RustlerBackend;
use crate::core::backend::Backend;
use crate::core::config::NewAlefConfig;
use crate::core::ir::{ApiSurface, FieldDef, FunctionDef, ParamDef, TypeDef, TypeRef};

fn generated_public_api(api: &ApiSurface) -> String {
    let config: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["elixir"]
[[crates]]
name = "sample-shelf"
sources = ["src/lib.rs"]
"#,
    )
    .unwrap();
    let files = RustlerBackend
        .generate_public_api(api, &config.resolve().unwrap().remove(0))
        .unwrap();
    files
        .into_iter()
        .find(|file| file.path.ends_with("sample_shelf.ex"))
        .unwrap()
        .content
}

fn fixture_api(has_default: bool) -> ApiSurface {
    let list_books = FunctionDef {
        name: "list_books".to_string(),
        rust_path: "sample_shelf::list_books".to_string(),
        params: vec![ParamDef {
            name: "author".to_string(),
            ty: TypeRef::String,
            optional: true,
            ..Default::default()
        }],
        return_type: TypeRef::Vec(Box::new(TypeRef::Named("Book".to_string()))),
        ..Default::default()
    };
    let all_books = FunctionDef {
        name: "all_books".to_string(),
        rust_path: "sample_shelf::all_books".to_string(),
        params: vec![],
        error_type: Some("String".to_string()),
        ..list_books.clone()
    };
    ApiSurface {
        crate_name: "sample_shelf".to_string(),
        types: vec![TypeDef {
            name: "Book".to_string(),
            rust_path: "sample_shelf::Book".to_string(),
            has_default,
            fields: vec![FieldDef {
                name: "title".to_string(),
                ty: TypeRef::String,
                ..Default::default()
            }],
            ..Default::default()
        }],
        functions: vec![list_books, all_books],
        ..Default::default()
    }
}

#[test]
fn should_describe_struct_list_returns_with_the_generated_public_type() {
    for has_default in [false, true] {
        let source = generated_public_api(&fixture_api(has_default));
        assert_eq!(source.matches("[SampleShelf.Book.t()]").count(), 3, "{source}");
        assert!(source.contains("list_books(String.t() | nil)"), "{source}");
        assert!(!source.contains("[String.t() | nil]"), "{source}");
    }
}

#[test]
#[ignore = "requires the Elixir compiler; selected by the CI elixir_oracle gate"]
fn generated_struct_list_return_specs_execute_in_elixir() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("sample_shelf.ex");
    std::fs::write(&source, generated_public_api(&fixture_api(true))).unwrap();
    let script = directory.path().join("check.exs");
    std::fs::write(&script, include_str!("return_types.exs")).unwrap();
    let result = std::process::Command::new("elixir")
        .arg(&script)
        .arg(&source)
        .output()
        .unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    assert_eq!(
        String::from_utf8(result.stdout).unwrap().trim(),
        "RETURN_SPECS=3 INPUT_CONTROLS=1"
    );
}

#[test]
fn should_preserve_nested_return_shapes_and_opaque_handles() {
    use super::super::params_returns::elixir_return_typespec;
    let book = TypeRef::Named("Book".to_string());
    let opaque = TypeRef::Named("Handle".to_string());
    let public = ahash::AHashSet::from_iter(["Book".to_string(), "Handle".to_string()]);
    let opaque_types = ahash::AHashSet::from_iter(["Handle".to_string()]);
    let cases = [
        (book.clone(), "SampleShelf.Book.t()"),
        (TypeRef::Optional(Box::new(book.clone())), "SampleShelf.Book.t() | nil"),
        (TypeRef::Vec(Box::new(book.clone())), "[SampleShelf.Book.t()]"),
        (
            TypeRef::Vec(Box::new(TypeRef::Optional(Box::new(book.clone())))),
            "[SampleShelf.Book.t() | nil]",
        ),
        (
            TypeRef::Optional(Box::new(TypeRef::Vec(Box::new(book)))),
            "[SampleShelf.Book.t()] | nil",
        ),
        (TypeRef::Named("Unknown".to_string()), "map()"),
        (opaque.clone(), "reference()"),
        (TypeRef::Vec(Box::new(opaque)), "[reference()]"),
        (TypeRef::String, "String.t()"),
    ];
    assert_eq!(cases.len(), 9);
    for (ty, expected) in cases {
        assert_eq!(
            elixir_return_typespec(&ty, false, &opaque_types, "SampleShelf", &public),
            expected
        );
    }
}

#[test]
fn should_preserve_nested_public_return_types_at_the_generated_api_boundary() {
    let book = TypeRef::Named("Book".to_string());
    let optional = |inner| TypeRef::Optional(Box::new(inner));
    let list = |inner| TypeRef::Vec(Box::new(inner));
    let cases = [
        (book.clone(), "SampleShelf.Book.t()"),
        (optional(book.clone()), "SampleShelf.Book.t() | nil"),
        (list(book.clone()), "[SampleShelf.Book.t()]"),
        (list(optional(book.clone())), "[SampleShelf.Book.t() | nil]"),
        (optional(list(book)), "[SampleShelf.Book.t()] | nil"),
        (TypeRef::Named("Unknown".to_string()), "map()"),
        (TypeRef::Named("Handle".to_string()), "reference()"),
        (list(TypeRef::Named("Handle".to_string())), "[reference()]"),
        (TypeRef::String, "String.t()"),
    ];
    assert_eq!(cases.len(), 9);
    for (ty, expected) in cases {
        let mut api = fixture_api(true);
        api.types.push(TypeDef {
            name: "Handle".to_string(),
            rust_path: "sample_shelf::Handle".to_string(),
            is_opaque: true,
            ..Default::default()
        });
        api.functions.truncate(1);
        api.functions[0].return_type = ty;
        let source = generated_public_api(&api);
        assert!(
            source.contains(&format!("@spec list_books(String.t() | nil) :: {expected}")),
            "expected {expected}: {source}"
        );
    }
}
