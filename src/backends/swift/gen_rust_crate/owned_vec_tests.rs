//! A getter that bridges a `Vec` of handles or strings must reach Swift as an owned array.
//!
//! swift-bridge returns such a getter as a `RustVec`, and an element read from it by index
//! points into the vector's buffer without keeping the vector alive. The chained form
//! `reply.choices()[0].message()` then reads freed memory. These tests drive the real `emit`
//! path and check both halves of the fix: the extern getter gets a distinct Swift name, and a
//! Swift extension declares the original name on top of it.

use super::emit;
use crate::core::backend::GeneratedFile;
use crate::core::config::{NewAlefConfig, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, FieldDef, PrimitiveType, TypeDef, TypeRef};

fn config() -> ResolvedCrateConfig {
    let toml_src = "[workspace]\nlanguages = [\"swift\"]\n[[crates]]\nname = \"test-lib\"\n\
         sources = [\"src/lib.rs\"]\n";
    let cfg: NewAlefConfig = toml::from_str(toml_src).unwrap();
    cfg.resolve().unwrap().remove(0)
}

fn field(name: &str, ty: TypeRef) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        ..Default::default()
    }
}

fn handle(name: &str, fields: Vec<FieldDef>) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: format!("test_lib::{name}"),
        has_serde: false,
        fields,
        ..Default::default()
    }
}

fn api(reply_fields: Vec<FieldDef>) -> ApiSurface {
    ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![
            handle("Reply", reply_fields),
            handle("Choice", vec![field("index", TypeRef::Primitive(PrimitiveType::U32))]),
        ],
        ..Default::default()
    }
}

fn file_named<'a>(files: &'a [GeneratedFile], suffix: &str) -> Option<&'a GeneratedFile> {
    files.iter().find(|f| f.path.to_string_lossy().ends_with(suffix))
}

fn vec_of(inner: TypeRef) -> TypeRef {
    TypeRef::Vec(Box::new(inner))
}

#[test]
fn handle_and_string_vectors_are_read_as_owned_arrays() {
    let api = api(vec![
        field("choices", vec_of(TypeRef::Named("Choice".into()))),
        field("tags", vec_of(TypeRef::String)),
        field("scores", vec_of(TypeRef::Primitive(PrimitiveType::F32))),
    ]);
    let files = emit(&api, &config()).unwrap();

    let swift = file_named(&files, "ZSwiftOwnedVecAccessors.swift")
        .expect("a getter returning a vector of handles needs an owned-array accessor")
        .content
        .clone();
    assert!(swift.contains("extension ReplyRef {"), "{swift}");
    assert!(swift.contains("public func choices() -> [Choice] {"), "{swift}");
    assert!(
        swift.contains("self.choicesRustVec().alefIntoOwnedArray()"),
        "the accessor must move the elements out of the vector: {swift}"
    );
    assert!(swift.contains("public func tags() -> [RustString] {"), "{swift}");
    assert!(
        !swift.contains("scores"),
        "primitive elements are indexed by value and keep the bridge shape: {swift}"
    );
}

#[test]
fn the_bridge_getter_gets_a_distinct_swift_name() {
    let api = api(vec![
        field("choices", vec_of(TypeRef::Named("Choice".into()))),
        field("scores", vec_of(TypeRef::Primitive(PrimitiveType::F32))),
    ]);
    let files = emit(&api, &config()).unwrap();
    let lib_rs = &file_named(&files, "lib.rs").unwrap().content;

    assert!(
        lib_rs.contains("swift_name = \"choicesRustVec\""),
        "the extern getter must not take the public name: {lib_rs}"
    );
    assert!(!lib_rs.contains("scoresRustVec"), "{lib_rs}");
}

#[test]
fn no_accessor_file_without_a_handle_or_string_vector() {
    let api = api(vec![field("scores", vec_of(TypeRef::Primitive(PrimitiveType::F32)))]);
    let files = emit(&api, &config()).unwrap();
    assert!(file_named(&files, "ZSwiftOwnedVecAccessors.swift").is_none());
}
