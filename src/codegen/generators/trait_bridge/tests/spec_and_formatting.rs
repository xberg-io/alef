use super::*;

#[test]
fn test_wrapper_name() {
    let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", vec![]);
    let config = make_trait_bridge_config(None, None);
    let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
    assert_eq!(spec.wrapper_name(), "PyOcrBackendBridge");
}

#[test]
fn test_trait_snake() {
    let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", vec![]);
    let config = make_trait_bridge_config(None, None);
    let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
    assert_eq!(spec.trait_snake(), "ocr_backend");
}

#[test]
fn test_trait_path_replaces_hyphens() {
    let trait_def = make_type_def("OcrBackend", "my-lib::OcrBackend", vec![]);
    let config = make_trait_bridge_config(None, None);
    let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
    assert_eq!(spec.trait_path(), "my_lib::OcrBackend");
}

#[test]
fn test_required_methods_filters_no_default_impl() {
    let methods = vec![
        make_method(
            "process",
            MakeMethodOptions {
                params: vec![],
                return_type: TypeRef::String,
                is_async: false,
                has_default_impl: false,
                trait_source: None,
                error_type: None,
            },
        ),
        make_method(
            "initialize",
            MakeMethodOptions {
                params: vec![],
                return_type: TypeRef::Unit,
                is_async: false,
                has_default_impl: true,
                trait_source: None,
                error_type: None,
            },
        ),
        make_method(
            "detect",
            MakeMethodOptions {
                params: vec![],
                return_type: TypeRef::String,
                is_async: false,
                has_default_impl: false,
                trait_source: None,
                error_type: None,
            },
        ),
    ];
    let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", methods);
    let config = make_trait_bridge_config(None, None);
    let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
    let required = spec.required_methods();
    assert_eq!(required.len(), 2);
    assert!(required.iter().any(|m| m.name == "process"));
    assert!(required.iter().any(|m| m.name == "detect"));
}

#[test]
fn test_optional_methods_filters_has_default_impl() {
    let methods = vec![
        make_method(
            "process",
            MakeMethodOptions {
                params: vec![],
                return_type: TypeRef::String,
                is_async: false,
                has_default_impl: false,
                trait_source: None,
                error_type: None,
            },
        ),
        make_method(
            "initialize",
            MakeMethodOptions {
                params: vec![],
                return_type: TypeRef::Unit,
                is_async: false,
                has_default_impl: true,
                trait_source: None,
                error_type: None,
            },
        ),
        make_method(
            "shutdown",
            MakeMethodOptions {
                params: vec![],
                return_type: TypeRef::Unit,
                is_async: false,
                has_default_impl: true,
                trait_source: None,
                error_type: None,
            },
        ),
    ];
    let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", methods);
    let config = make_trait_bridge_config(None, None);
    let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
    let optional = spec.optional_methods();
    assert_eq!(optional.len(), 2);
    assert!(optional.iter().any(|m| m.name == "initialize"));
    assert!(optional.iter().any(|m| m.name == "shutdown"));
}

#[test]
fn test_error_path() {
    let trait_def = make_type_def("OcrBackend", "mylib::OcrBackend", vec![]);
    let config = make_trait_bridge_config(None, None);
    let spec = make_spec(&trait_def, &config, "Py", HashMap::new());
    assert_eq!(spec.error_path(), "mylib::MyError");
}

#[test]
fn test_format_type_ref_primitives() {
    let paths = HashMap::new();
    let cases: Vec<(TypeRef, &str)> = vec![
        (TypeRef::Primitive(PrimitiveType::Bool), "bool"),
        (TypeRef::Primitive(PrimitiveType::U8), "u8"),
        (TypeRef::Primitive(PrimitiveType::U16), "u16"),
        (TypeRef::Primitive(PrimitiveType::U32), "u32"),
        (TypeRef::Primitive(PrimitiveType::U64), "u64"),
        (TypeRef::Primitive(PrimitiveType::I8), "i8"),
        (TypeRef::Primitive(PrimitiveType::I16), "i16"),
        (TypeRef::Primitive(PrimitiveType::I32), "i32"),
        (TypeRef::Primitive(PrimitiveType::I64), "i64"),
        (TypeRef::Primitive(PrimitiveType::F32), "f32"),
        (TypeRef::Primitive(PrimitiveType::F64), "f64"),
        (TypeRef::Primitive(PrimitiveType::Usize), "usize"),
        (TypeRef::Primitive(PrimitiveType::Isize), "isize"),
    ];
    for (ty, expected) in cases {
        assert_eq!(format_type_ref(&ty, &paths), expected, "mismatch for {expected}");
    }
}

#[test]
fn test_format_type_ref_string() {
    assert_eq!(format_type_ref(&TypeRef::String, &HashMap::new()), "String");
}

#[test]
fn test_format_type_ref_char() {
    assert_eq!(format_type_ref(&TypeRef::Char, &HashMap::new()), "char");
}

#[test]
fn test_format_type_ref_bytes() {
    assert_eq!(format_type_ref(&TypeRef::Bytes, &HashMap::new()), "Vec<u8>");
}

#[test]
fn test_format_type_ref_path() {
    assert_eq!(format_type_ref(&TypeRef::Path, &HashMap::new()), "std::path::PathBuf");
}

#[test]
fn test_format_type_ref_unit() {
    assert_eq!(format_type_ref(&TypeRef::Unit, &HashMap::new()), "()");
}

#[test]
fn test_format_type_ref_json() {
    assert_eq!(format_type_ref(&TypeRef::Json, &HashMap::new()), "serde_json::Value");
}

#[test]
fn test_format_type_ref_duration() {
    assert_eq!(
        format_type_ref(&TypeRef::Duration, &HashMap::new()),
        "std::time::Duration"
    );
}

#[test]
fn test_format_type_ref_optional() {
    let ty = TypeRef::Optional(Box::new(TypeRef::String));
    assert_eq!(format_type_ref(&ty, &HashMap::new()), "Option<String>");
}

#[test]
fn test_format_type_ref_optional_nested() {
    let ty = TypeRef::Optional(Box::new(TypeRef::Optional(Box::new(TypeRef::Primitive(
        PrimitiveType::U32,
    )))));
    assert_eq!(format_type_ref(&ty, &HashMap::new()), "Option<Option<u32>>");
}

#[test]
fn test_format_type_ref_vec() {
    let ty = TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::U8)));
    assert_eq!(format_type_ref(&ty, &HashMap::new()), "Vec<u8>");
}

#[test]
fn test_format_type_ref_vec_nested() {
    let ty = TypeRef::Vec(Box::new(TypeRef::Vec(Box::new(TypeRef::String))));
    assert_eq!(format_type_ref(&ty, &HashMap::new()), "Vec<Vec<String>>");
}

#[test]
fn test_format_type_ref_map() {
    let ty = TypeRef::Map(
        Box::new(TypeRef::String),
        Box::new(TypeRef::Primitive(PrimitiveType::I64)),
    );
    assert_eq!(
        format_type_ref(&ty, &HashMap::new()),
        "std::collections::HashMap<String, i64>"
    );
}

#[test]
fn test_format_type_ref_map_nested_value() {
    let ty = TypeRef::Map(
        Box::new(TypeRef::String),
        Box::new(TypeRef::Vec(Box::new(TypeRef::String))),
    );
    assert_eq!(
        format_type_ref(&ty, &HashMap::new()),
        "std::collections::HashMap<String, Vec<String>>"
    );
}

#[test]
fn test_format_type_ref_named_without_type_paths() {
    let ty = TypeRef::Named("Config".to_string());
    assert_eq!(format_type_ref(&ty, &HashMap::new()), "Config");
}

#[test]
fn test_format_type_ref_named_with_type_paths() {
    let ty = TypeRef::Named("Config".to_string());
    let mut paths = HashMap::new();
    paths.insert("Config".to_string(), "mylib::Config".to_string());
    assert_eq!(format_type_ref(&ty, &paths), "mylib::Config");
}

#[test]
fn test_format_type_ref_named_not_in_type_paths_falls_back_to_name() {
    let ty = TypeRef::Named("Unknown".to_string());
    let mut paths = HashMap::new();
    paths.insert("Other".to_string(), "mylib::Other".to_string());
    assert_eq!(format_type_ref(&ty, &paths), "Unknown");
}

#[test]
fn test_format_param_type_string_ref() {
    let param = make_param("input", TypeRef::String, true);
    assert_eq!(format_param_type(&param, &HashMap::new()), "&str");
}

#[test]
fn test_format_param_type_string_owned() {
    let param = make_param("input", TypeRef::String, false);
    assert_eq!(format_param_type(&param, &HashMap::new()), "String");
}

#[test]
fn test_format_param_type_bytes_ref() {
    let param = make_param("data", TypeRef::Bytes, true);
    assert_eq!(format_param_type(&param, &HashMap::new()), "&[u8]");
}

#[test]
fn test_format_param_type_bytes_owned() {
    let param = make_param("data", TypeRef::Bytes, false);
    assert_eq!(format_param_type(&param, &HashMap::new()), "Vec<u8>");
}

#[test]
fn test_format_param_type_path_ref() {
    let param = make_param("path", TypeRef::Path, true);
    assert_eq!(format_param_type(&param, &HashMap::new()), "&std::path::Path");
}

#[test]
fn test_format_param_type_path_owned() {
    let param = make_param("path", TypeRef::Path, false);
    assert_eq!(format_param_type(&param, &HashMap::new()), "std::path::PathBuf");
}

#[test]
fn test_format_param_type_vec_ref() {
    let param = make_param("items", TypeRef::Vec(Box::new(TypeRef::String)), true);
    assert_eq!(format_param_type(&param, &HashMap::new()), "&[String]");
}

#[test]
fn test_format_param_type_vec_owned() {
    let param = make_param("items", TypeRef::Vec(Box::new(TypeRef::String)), false);
    assert_eq!(format_param_type(&param, &HashMap::new()), "Vec<String>");
}

#[test]
fn test_format_param_type_named_ref_with_type_paths() {
    let mut paths = HashMap::new();
    paths.insert("Config".to_string(), "mylib::Config".to_string());
    let param = make_param("cfg", TypeRef::Named("Config".to_string()), true);
    assert_eq!(format_param_type(&param, &paths), "&mylib::Config");
}

#[test]
fn test_format_param_type_named_ref_without_type_paths() {
    let param = make_param("cfg", TypeRef::Named("Config".to_string()), true);
    assert_eq!(format_param_type(&param, &HashMap::new()), "&Config");
}

#[test]
fn test_format_param_type_primitive_ref_passes_by_value() {
    let param = make_param("count", TypeRef::Primitive(PrimitiveType::U32), true);
    assert_eq!(format_param_type(&param, &HashMap::new()), "u32");
}

#[test]
fn test_format_param_type_unit_ref_passes_by_value() {
    let param = make_param("nothing", TypeRef::Unit, true);
    assert_eq!(format_param_type(&param, &HashMap::new()), "()");
}

#[test]
fn test_format_return_type_without_error() {
    let result = format_return_type(&TypeRef::String, None, &HashMap::new(), false);
    assert_eq!(result, "String");
}

#[test]
fn test_format_return_type_with_error() {
    let result = format_return_type(&TypeRef::String, Some("MyError"), &HashMap::new(), false);
    assert_eq!(result, "std::result::Result<String, MyError>");
}

#[test]
fn test_format_return_type_unit_with_error() {
    let result = format_return_type(
        &TypeRef::Unit,
        Some("Box<dyn std::error::Error>"),
        &HashMap::new(),
        false,
    );
    assert_eq!(result, "std::result::Result<(), Box<dyn std::error::Error>>");
}

#[test]
fn test_format_return_type_named_with_type_paths_and_error() {
    let mut paths = HashMap::new();
    paths.insert("Output".to_string(), "mylib::Output".to_string());
    let result = format_return_type(
        &TypeRef::Named("Output".to_string()),
        Some("mylib::MyError"),
        &paths,
        false,
    );
    assert_eq!(result, "std::result::Result<mylib::Output, mylib::MyError>");
}

#[test]
fn test_format_return_type_vec_string_with_returns_ref() {
    let result = format_return_type(&TypeRef::Vec(Box::new(TypeRef::String)), None, &HashMap::new(), true);
    assert_eq!(result, "&[&str]", "Vec<String> + returns_ref must yield &[&str]");
}

#[test]
fn test_format_return_type_vec_no_returns_ref_unchanged() {
    let result = format_return_type(&TypeRef::Vec(Box::new(TypeRef::String)), None, &HashMap::new(), false);
    assert_eq!(
        result, "Vec<String>",
        "Vec<String> without returns_ref must stay Vec<String>"
    );
}

#[test]
fn vtable_slot_names_follow_rust_declaration_order() {
    let trait_def = make_type_def(
        "OcrBackend",
        "mylib::OcrBackend",
        vec![
            make_method(
                "process_image",
                MakeMethodOptions {
                    params: vec![],
                    return_type: TypeRef::String,
                    is_async: false,
                    has_default_impl: false,
                    trait_source: None,
                    error_type: Some("Error"),
                },
            ),
            make_method(
                "backend_type",
                MakeMethodOptions {
                    params: vec![],
                    return_type: TypeRef::Named("OcrBackendType".into()),
                    is_async: false,
                    has_default_impl: false,
                    trait_source: None,
                    error_type: None,
                },
            ),
            make_method(
                "supported_languages",
                MakeMethodOptions {
                    params: vec![],
                    return_type: TypeRef::Vec(Box::new(TypeRef::String)),
                    is_async: false,
                    has_default_impl: false,
                    trait_source: None,
                    error_type: None,
                },
            ),
        ],
    );

    assert_eq!(
        vtable_slot_names(&trait_def, true, &[]),
        vec![
            "name_fn",
            "version_fn",
            "initialize_fn",
            "shutdown_fn",
            "process_image",
            "backend_type",
            "supported_languages",
            "free_string",
            "free_user_data",
        ],
        "slots must be the four plugin slots, then own methods in declaration order, then destructors"
    );
}

#[test]
fn vtable_slot_names_omit_only_super_trait_and_skipped_methods() {
    let trait_def = make_type_def(
        "OcrBackend",
        "mylib::OcrBackend",
        vec![
            make_method(
                "name",
                MakeMethodOptions {
                    params: vec![],
                    return_type: TypeRef::String,
                    is_async: false,
                    has_default_impl: false,
                    trait_source: Some("Plugin"),
                    error_type: None,
                },
            ),
            make_method(
                "probe",
                MakeMethodOptions {
                    params: vec![],
                    return_type: TypeRef::String,
                    is_async: false,
                    has_default_impl: false,
                    trait_source: None,
                    error_type: None,
                },
            ),
            make_method(
                "backend_type",
                MakeMethodOptions {
                    params: vec![],
                    return_type: TypeRef::Named("OcrBackendType".into()),
                    is_async: false,
                    has_default_impl: false,
                    trait_source: None,
                    error_type: None,
                },
            ),
        ],
    );

    assert_eq!(
        vtable_slot_names(&trait_def, false, &["probe".to_string()]),
        vec!["backend_type", "free_string", "free_user_data"],
        "a super-trait method and an ffi_skip_methods entry are the only own methods without a slot"
    );
}

#[test]
fn own_vtable_methods_keeps_a_method_returning_any_named_type() {
    let trait_def = make_type_def(
        "PostProcessor",
        "mylib::PostProcessor",
        vec![make_method(
            "processing_stage",
            MakeMethodOptions {
                params: vec![],
                return_type: TypeRef::Named("ProcessingStage".into()),
                is_async: false,
                has_default_impl: false,
                trait_source: None,
                error_type: None,
            },
        )],
    );

    let names: Vec<&str> = own_vtable_methods(&trait_def, &[])
        .iter()
        .map(|method| method.name.as_str())
        .collect();
    assert_eq!(
        names,
        vec!["processing_stage"],
        "slot membership must not depend on whether the return type is visible to a binding"
    );
}
