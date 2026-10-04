use super::*;
use crate::core::ir::{FieldDef, NewtypeContainer, NewtypeConversion, NewtypeWrapper, NewtypeWrapperMetadata, TypeDef};

fn transparent_string_metadata(containers: Vec<NewtypeContainer>) -> String {
    NewtypeWrapper::encode_explicit(&[NewtypeWrapperMetadata::transparent_string(
        "sample_lib::SecretString",
        "from",
        "into_inner",
        containers,
    )])
}

#[test]
fn borrowed_transparent_wrapper_returns_are_rejected_before_codegen() {
    use NewtypeContainer::{Optional, Vec as VecContainer};

    let cases = [
        ("borrowed_secret", TypeRef::String, vec![]),
        (
            "borrowed_optional_secret",
            TypeRef::Optional(Box::new(TypeRef::String)),
            vec![Optional],
        ),
        (
            "borrowed_nested_secrets",
            TypeRef::Vec(Box::new(TypeRef::Optional(Box::new(TypeRef::String)))),
            vec![VecContainer, Optional],
        ),
    ];
    let functions = cases
        .into_iter()
        .map(|(name, return_type, containers)| {
            let mut function = function_def(name, vec![], return_type);
            function.returns_ref = true;
            function.return_newtype_wrapper = Some(transparent_string_metadata(containers));
            function
        })
        .collect();
    let api = ApiSurface {
        crate_name: "sample-lib".to_string(),
        functions,
        ..ApiSurface::default()
    };

    let report = validate_api_surface(&api);

    let diagnostics: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.reason.contains("borrowed transparent-wrapper return"))
        .collect();
    assert_eq!(diagnostics.len(), 3, "{report:?}");
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code == ValidationCode::InvalidNewtypeMetadata)
    );
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic.reason.contains("consumes the wrapper"))
    );
}

#[test]
fn mutable_transparent_wrapper_parameters_are_rejected_before_codegen() {
    let mut parameter = ParamDef {
        name: "secret".to_string(),
        ty: TypeRef::String,
        is_ref: true,
        is_mut: true,
        ..ParamDef::default()
    };
    parameter.newtype_wrapper = Some(transparent_string_metadata(vec![]));
    let api = ApiSurface {
        crate_name: "sample-lib".to_string(),
        functions: vec![function_def("mutate_secret", vec![parameter], TypeRef::Unit)],
        ..ApiSurface::default()
    };

    let report = validate_api_surface(&api);

    let diagnostic = report
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == ValidationCode::InvalidNewtypeMetadata)
        .expect("mutable wrapper parameter must be rejected");
    assert_eq!(
        diagnostic.item_path.as_deref(),
        Some("sample_lib::mutate_secret parameter `secret`")
    );
    assert!(diagnostic.reason.contains("mutable transparent-wrapper parameter"));
    assert!(diagnostic.reason.contains("write the mutated wrapper back"));
}

#[test]
fn clone_is_required_only_when_a_generated_field_accessor_clones_the_wrapper() {
    let mut metadata =
        NewtypeWrapperMetadata::transparent_string("sample_lib::SecretString", "from", "into_inner", vec![]);
    let NewtypeConversion::TransparentString { wrapper_is_clone, .. } = &mut metadata.conversion else {
        unreachable!("transparent_string constructor creates transparent metadata");
    };
    *wrapper_is_clone = Some(false);
    let metadata = NewtypeWrapper::encode_explicit(&[metadata]);
    let wrapper_type = TypeDef {
        name: "SecretString".to_string(),
        rust_path: "sample_lib::SecretString".to_string(),
        binding_excluded: true,
        binding_exclusion_reason: Some("alef(transparent_string)".to_string()),
        is_clone: false,
        ..TypeDef::default()
    };
    let container_type = TypeDef {
        name: "Credentials".to_string(),
        rust_path: "sample_lib::Credentials".to_string(),
        fields: vec![FieldDef {
            name: "secret".to_string(),
            ty: TypeRef::String,
            newtype_wrapper: Some(metadata.clone()),
            ..FieldDef::default()
        }],
        ..TypeDef::default()
    };
    let by_value = ParamDef {
        name: "secret".to_string(),
        ty: TypeRef::String,
        newtype_wrapper: Some(metadata),
        ..ParamDef::default()
    };
    let api = ApiSurface {
        crate_name: "sample-lib".to_string(),
        types: vec![wrapper_type, container_type],
        functions: vec![function_def("consume", vec![by_value], TypeRef::Unit)],
        ..ApiSurface::default()
    };

    let report = validate_api_surface(&api);

    let clone_diagnostics: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.reason.contains("requires Clone"))
        .collect();
    assert_eq!(clone_diagnostics.len(), 1, "{report:?}");
    assert_eq!(
        clone_diagnostics[0].item_path.as_deref(),
        Some("sample_lib::Credentials.secret")
    );
}
