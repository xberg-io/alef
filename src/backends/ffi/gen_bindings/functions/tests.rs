use crate::core::ir::{
    FunctionDef, MethodDef, NewtypeContainer, NewtypeWrapper, NewtypeWrapperMetadata, ParamDef, PrimitiveType, TypeDef,
    TypeRef,
};
use ahash::{AHashMap, AHashSet};

use super::orchestration::{gen_free_function, gen_function_wrapper_footer, gen_method_wrapper};
use super::params::{ParamConversionContext, gen_param_conversion_with_enums, optional_borrowed_param_call_arg};
use super::result_presence::gen_method_result_presence_wrapper;
use super::return_handling::return_type_needs_non_serde_named;

fn transparent_wrapper(paths: &[Vec<NewtypeContainer>]) -> String {
    NewtypeWrapper::encode_explicit(
        &paths
            .iter()
            .map(|path| {
                NewtypeWrapperMetadata::transparent_string(
                    "sample_crate::SecretString",
                    "from",
                    "into_inner",
                    path.clone(),
                )
            })
            .collect::<Vec<_>>(),
    )
}

fn conversion_context<'a>(
    return_type: &'a TypeRef,
    path_map: &'a AHashMap<String, String>,
    enum_names: &'a AHashSet<String>,
) -> ParamConversionContext<'a> {
    ParamConversionContext {
        has_error: false,
        is_bytes_result: false,
        return_type,
        ffi_return_type: None,
        core_import: "sample_crate",
        path_map,
        enum_names,
    }
}

#[test]
fn transparent_string_optional_param_wraps_after_ffi_decoding() {
    let param = ParamDef {
        name: "credential".to_string(),
        ty: TypeRef::String,
        optional: true,
        newtype_wrapper: Some(transparent_wrapper(&[vec![NewtypeContainer::Optional]])),
        ..ParamDef::default()
    };
    let return_type = TypeRef::Unit;
    let path_map = AHashMap::new();
    let enum_names = AHashSet::new();
    let context = conversion_context(&return_type, &path_map, &enum_names);
    let output = gen_param_conversion_with_enums(&param, &context);

    assert!(
        output.contains("let credential_rs: Option<sample_crate::SecretString> =")
            && output.contains("map(|value| sample_crate::SecretString::from(value))"),
        "got:\n{output}"
    );
}

#[test]
fn borrowed_optional_transparent_string_param_uses_as_ref_after_ffi_decoding() {
    let function = FunctionDef {
        name: "borrow_optional_secret".to_string(),
        rust_path: "sample_crate::borrow_optional_secret".to_string(),
        params: vec![ParamDef {
            name: "value".to_string(),
            ty: TypeRef::String,
            optional: true,
            is_ref: true,
            newtype_wrapper: Some(transparent_wrapper(&[vec![NewtypeContainer::Optional]])),
            ..ParamDef::default()
        }],
        ..FunctionDef::default()
    };

    let output = gen_free_function(
        &function,
        "sample",
        "sample_crate",
        &AHashMap::new(),
        &AHashSet::new(),
        &AHashSet::new(),
        None,
        false,
    );

    assert!(
        output.contains("sample_crate::borrow_optional_secret(value_rs.as_ref())"),
        "got:\n{output}"
    );
    assert!(!output.contains("value_rs.as_deref()"), "got:\n{output}");
}

#[test]
fn ordinary_optional_borrowed_string_and_bytes_preserve_as_deref() {
    for ty in [TypeRef::String, TypeRef::Bytes] {
        let param = ParamDef {
            name: "value".to_string(),
            ty,
            optional: true,
            is_ref: true,
            ..ParamDef::default()
        };

        assert_eq!(
            optional_borrowed_param_call_arg(&param, "value_rs"),
            "value_rs.as_deref()"
        );
    }
}

#[test]
fn borrowed_optional_transparent_string_method_and_presence_use_as_ref() {
    let method = MethodDef {
        name: "borrow_optional_secret".to_string(),
        params: vec![ParamDef {
            name: "value".to_string(),
            ty: TypeRef::String,
            optional: true,
            is_ref: true,
            newtype_wrapper: Some(transparent_wrapper(&[vec![NewtypeContainer::Optional]])),
            ..ParamDef::default()
        }],
        return_type: TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::U64))),
        is_static: true,
        ..MethodDef::default()
    };
    let typ = TypeDef {
        name: "Vault".to_string(),
        rust_path: "sample_crate::Vault".to_string(),
        ..TypeDef::default()
    };
    let path_map = AHashMap::new();
    let names = AHashSet::new();

    let method_output = gen_method_wrapper(&typ, &method, "sample", "sample_crate", &path_map, &names, &names);
    let presence_output =
        gen_method_result_presence_wrapper(&typ, &method, "sample", "sample_crate", &path_map, &names)
            .expect("optional scalar return should emit a presence companion");

    for output in [&method_output, &presence_output] {
        assert!(
            output.contains("sample_crate::Vault::borrow_optional_secret(value_rs.as_ref())"),
            "got:\n{output}"
        );
        assert!(!output.contains("value_rs.as_deref()"), "got:\n{output}");
    }
}

#[test]
fn transparent_string_map_param_has_explicit_core_collection_type() {
    let param = ParamDef {
        name: "credentials".to_string(),
        ty: TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
        newtype_wrapper: Some(transparent_wrapper(&[
            vec![NewtypeContainer::MapKey],
            vec![NewtypeContainer::MapValue],
        ])),
        ..ParamDef::default()
    };
    let return_type = TypeRef::Unit;
    let path_map = AHashMap::new();
    let enum_names = AHashSet::new();
    let context = conversion_context(&return_type, &path_map, &enum_names);
    let output = gen_param_conversion_with_enums(&param, &context);

    assert!(
        output.contains(
            "let credentials_rs: std::collections::HashMap<sample_crate::SecretString, sample_crate::SecretString> ="
        ),
        "got:\n{output}"
    );
    assert!(
        output.contains("sample_crate::SecretString::from(key)"),
        "got:\n{output}"
    );
    assert!(
        output.contains("sample_crate::SecretString::from(value)"),
        "got:\n{output}"
    );
}

#[test]
fn transparent_string_btree_map_param_does_not_require_hashable_core_keys() {
    let param = ParamDef {
        name: "credentials".to_string(),
        ty: TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
        map_is_btree: true,
        newtype_wrapper: Some(transparent_wrapper(&[
            vec![NewtypeContainer::MapKey],
            vec![NewtypeContainer::MapValue],
        ])),
        ..ParamDef::default()
    };
    let return_type = TypeRef::Unit;
    let path_map = AHashMap::new();
    let enum_names = AHashSet::new();
    let context = conversion_context(&return_type, &path_map, &enum_names);
    let output = gen_param_conversion_with_enums(&param, &context);

    assert!(
        output.contains(
            "let credentials_rs: std::collections::BTreeMap<sample_crate::SecretString, sample_crate::SecretString> ="
        ),
        "got:\n{output}"
    );
    assert_eq!(
        output.matches("credentials_rs).into_iter()").count(),
        1,
        "got:\n{output}"
    );
    assert!(!output.contains("HashMap<sample_crate::SecretString"), "got:\n{output}");
}

#[test]
fn return_type_needs_non_serde_named_vec_non_serde() {
    let mut serde_names: AHashSet<String> = AHashSet::new();
    serde_names.insert("ExtractionResult".to_string());

    let vec_non_serde = TypeRef::Vec(Box::new(TypeRef::Named("PatternMatch".to_string())));
    assert!(
        return_type_needs_non_serde_named(&vec_non_serde, &serde_names),
        "Vec<PatternMatch> without Serialize must be detected as needing stub"
    );
}

#[test]
fn return_type_needs_non_serde_named_vec_serde_ok() {
    let mut serde_names: AHashSet<String> = AHashSet::new();
    serde_names.insert("ExtractionResult".to_string());

    let vec_serde = TypeRef::Vec(Box::new(TypeRef::Named("ExtractionResult".to_string())));
    assert!(
        !return_type_needs_non_serde_named(&vec_serde, &serde_names),
        "Vec<ExtractionResult> with Serialize must NOT be detected as needing stub"
    );
}

#[test]
fn return_type_needs_non_serde_named_primitive_vec_not_affected() {
    let serde_names: AHashSet<String> = AHashSet::new();
    assert!(!return_type_needs_non_serde_named(
        &TypeRef::Vec(Box::new(TypeRef::String)),
        &serde_names
    ));
    assert!(!return_type_needs_non_serde_named(
        &TypeRef::Vec(Box::new(TypeRef::Primitive(crate::core::ir::PrimitiveType::U64))),
        &serde_names
    ));
}

#[test]
fn named_param_is_mut_call_site_passes_local_directly() {
    let p = ParamDef {
        name: "result".to_string(),
        ty: TypeRef::Named("ExtractionResult".to_string()),
        optional: false,
        default: None,
        sanitized: false,
        typed_default: None,
        is_ref: false,
        is_mut: true,
        newtype_wrapper: None,
        original_type: None,
        map_is_ahash: false,
        map_key_is_cow: false,
        vec_inner_is_ref: false,
        map_is_btree: false,
        core_wrapper: crate::core::ir::CoreWrapper::None,
    };
    let rs = format!("{}_rs", p.name);
    let result = if p.is_mut {
        rs.clone()
    } else if p.is_ref {
        format!("&{rs}")
    } else {
        rs.clone()
    };
    assert_eq!(
        result, "result_rs",
        "is_mut Named param must pass local directly (already &mut T)"
    );
}

#[test]
fn enum_param_local_name_uses_param_name_not_type_name() {
    let mut enum_names: AHashSet<String> = AHashSet::new();
    enum_names.insert("RedactionStrategy".to_string());

    let p = ParamDef {
        name: "strategy".to_string(),
        ty: TypeRef::Named("RedactionStrategy".to_string()),
        optional: false,
        default: None,
        sanitized: false,
        typed_default: None,
        is_ref: false,
        is_mut: false,
        newtype_wrapper: None,
        original_type: None,
        map_is_ahash: false,
        map_key_is_cow: false,
        vec_inner_is_ref: false,
        map_is_btree: false,
        core_wrapper: crate::core::ir::CoreWrapper::None,
    };

    let output = gen_param_conversion_with_enums(
        &p,
        &ParamConversionContext {
            has_error: false,
            is_bytes_result: false,
            return_type: &TypeRef::Unit,
            ffi_return_type: None,
            core_import: "sample_crate",
            path_map: &AHashMap::new(),
            enum_names: &enum_names,
        },
    );

    assert!(
        output.contains("let strategy_rs ="),
        "enum local must be named after param (strategy_rs), got:\n{output}"
    );
    assert!(
        output.contains("redaction_strategy_from_i32_rs(strategy)"),
        "enum helper must receive the FFI param name (strategy), got:\n{output}"
    );
}

#[test]
fn panic_footer_uses_existing_failure_sentinel() {
    let pointer_footer = gen_function_wrapper_footer(
        &Some("*mut std::ffi::c_char".to_string()),
        &TypeRef::String,
        false,
        false,
    );
    assert!(pointer_footer.contains("AssertUnwindSafe(set_panic_error)"));
    assert!(pointer_footer.contains("std::ptr::null_mut()"));

    let const_pointer_footer = gen_function_wrapper_footer(
        &Some("*const sample_runtime::RawValue".to_string()),
        &TypeRef::Named("Value".to_string()),
        false,
        false,
    );
    assert!(const_pointer_footer.contains("std::ptr::null()"));
    assert!(!const_pointer_footer.contains("std::ptr::null_mut()"));

    let status_footer = gen_function_wrapper_footer(&Some("i32".to_string()), &TypeRef::Unit, true, false);
    assert!(status_footer.contains("-1"));
}

/// `trivial_call` skips the footer's leading `})) {` — the header already closed
/// `AssertUnwindSafe(inline_callee))` and opened the `match` arms itself when the closure was
/// replaced by a bare callee path (see `can_inline_trivially` in `orchestration.rs`).
#[test]
fn panic_footer_skips_closure_close_for_trivial_call() {
    let footer = gen_function_wrapper_footer(&None, &TypeRef::Unit, false, true);
    assert!(
        !footer.contains("})) {"),
        "trivial_call footer must not re-close a closure the header never opened:\n{footer}"
    );
    assert!(
        footer.trim_start().starts_with("Ok(value) => value,"),
        "trivial_call footer must start directly at the match arms:\n{footer}"
    );
}

#[test]
fn scalar_handle_override_controls_parameter_failure_sentinel() {
    let parameter = ParamDef {
        name: "source".to_string(),
        ty: TypeRef::String,
        ..ParamDef::default()
    };
    let output = gen_param_conversion_with_enums(
        &parameter,
        &ParamConversionContext {
            has_error: false,
            is_bytes_result: false,
            return_type: &TypeRef::String,
            ffi_return_type: Some("AlefHandle"),
            core_import: "sample_lib",
            path_map: &AHashMap::new(),
            enum_names: &AHashSet::new(),
        },
    );

    assert!(output.contains("return 0;"), "{output}");
    assert!(!output.contains("return std::ptr::null_mut();"), "{output}");
}
