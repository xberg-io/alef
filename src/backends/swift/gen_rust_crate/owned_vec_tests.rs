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
use crate::core::ir::{ApiSurface, FieldDef, FunctionDef, MethodDef, ParamDef, PrimitiveType, ReceiverKind, TypeDef, TypeRef};
use std::collections::HashSet;

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
        is_opaque: true,
        has_serde: true,
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

fn param(name: &str, ty: TypeRef, optional: bool) -> ParamDef {
    ParamDef {
        name: name.to_string(),
        ty,
        optional,
        ..ParamDef::default()
    }
}

fn handles() -> HashSet<String> {
    ["Choice".to_string()].into_iter().collect()
}

#[test]
fn a_free_function_returning_strings_is_wrapped_to_an_owned_array() {
    let mut api = api(vec![field("index", TypeRef::Primitive(PrimitiveType::U32))]);
    api.functions.push(FunctionDef {
        name: "provider_names".to_string(),
        rust_path: "test_lib::provider_names".to_string(),
        return_type: vec_of(TypeRef::String),
        error_type: Some("TestError".to_string()),
        ..FunctionDef::default()
    });
    let files = emit(&api, &config()).unwrap();

    let lib_rs = &file_named(&files, "lib.rs").unwrap().content;
    assert!(lib_rs.contains("swift_name = \"providerNamesRustVec\""), "{lib_rs}");
    let swift = &file_named(&files, "ZSwiftOwnedVecAccessors.swift")
        .expect("a function returning a vector of strings needs a wrapper")
        .content;
    assert!(swift.contains("public func providerNames() throws -> [RustString] {"), "{swift}");
    assert!(
        swift.contains("try providerNamesRustVec().alefIntoOwnedArray()"),
        "the wrapper must move the elements out of the vector: {swift}"
    );
}

#[test]
fn an_async_method_returning_handles_keeps_its_parameters_in_the_wrapper() {
    let ty = TypeDef {
        name: "Client".to_string(),
        is_opaque: true,
        methods: vec![MethodDef {
            name: "list_models".to_string(),
            params: vec![param("provider", TypeRef::String, true), param("limit", TypeRef::Primitive(PrimitiveType::U32), false)],
            return_type: vec_of(TypeRef::Named("Choice".into())),
            error_type: Some("TestError".to_string()),
            is_async: true,
            receiver: Some(ReceiverKind::Ref),
            ..MethodDef::default()
        }],
        ..TypeDef::default()
    };
    let bridge_fn = super::extern_block::type_method_bridge_fn(&ty, &ty.methods[0], &handles(), &HashSet::new(), &HashSet::new());
    assert_eq!(bridge_fn.swift_name, "clientListModelsRustVec");

    let wrapper = super::owned_vec::owned_vec_function(&bridge_fn).unwrap().expect("a wrapper");
    assert_eq!(wrapper.swift_name, "clientListModels");
    assert_eq!(wrapper.generics, "<GenericIntoRustString: IntoRustString>");
    assert_eq!(
        wrapper.params,
        "_ client: ClientRef, _ provider: Optional<GenericIntoRustString>, _ limit: UInt32"
    );
    assert!(wrapper.is_async && wrapper.throws);
    assert_eq!(wrapper.return_type, "[Choice]");
    assert_eq!(
        wrapper.body,
        "try await clientListModelsRustVec(client, provider, limit).alefIntoOwnedArray()"
    );
}

#[test]
fn a_function_returning_primitives_keeps_its_bridge_shape() {
    let f = FunctionDef {
        name: "scores".to_string(),
        return_type: vec_of(TypeRef::Primitive(PrimitiveType::F32)),
        ..FunctionDef::default()
    };
    let bridge_fn = super::extern_block::function_bridge_fn(
        &f,
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &Default::default(),
        &Default::default(),
    )
    .unwrap();
    assert_eq!(bridge_fn.swift_name, "scores");
    assert!(super::owned_vec::owned_vec_function(&bridge_fn).unwrap().is_none());
}

#[test]
fn a_parameter_without_a_wrapper_form_is_an_error_not_an_unsafe_return() {
    let f = FunctionDef {
        name: "names_for".to_string(),
        params: vec![param("filter", TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)), false)],
        return_type: vec_of(TypeRef::String),
        ..FunctionDef::default()
    };
    let bridge_fn = super::extern_block::function_bridge_fn(
        &f,
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &Default::default(),
        &Default::default(),
    )
    .unwrap();
    // A map parameter crosses the bridge as a JSON `String`, which the wrapper forwards.
    assert!(super::owned_vec::owned_vec_function(&bridge_fn).unwrap().is_some());

    let mut odd = bridge_fn;
    odd.params = vec![("filter".to_string(), "Box<dyn Fn()>".to_string())];
    let error = super::owned_vec::owned_vec_function(&odd).unwrap_err().to_string();
    assert!(error.contains("namesFor") && error.contains("filter"), "{error}");
}
