#[test]
fn should_route_php_free_function_failures_through_the_typed_converter() {
    use super::{
        PhpFunctionTarget, PhpParamTypeSets, gen_async_function_as_static_method, gen_function_as_static_method,
    };
    use crate::backends::php::type_map::PhpMapper;
    use crate::core::ir::{FunctionDef, TypeRef};
    use ahash::AHashSet;

    let empty = AHashSet::new();
    let mapper = PhpMapper {
        enum_names: empty.clone(),
        data_enum_names: empty.clone(),
        untagged_data_enum_names: empty.clone(),
        json_string_enum_names: empty.clone(),
    };
    let mut function = FunctionDef {
        name: "request".into(),
        rust_path: "sample::request".into(),
        return_type: TypeRef::String,
        error_type: Some("ConversionError".into()),
        ..Default::default()
    };
    let sets = || PhpParamTypeSets {
        opaque: &empty,
        default: &empty,
        enums: &empty,
    };
    let sync = gen_function_as_static_method(
        &function,
        &mapper,
        sets(),
        PhpFunctionTarget {
            core_import: "sample",
            error_converter: Some("conversion_error_to_php_err"),
        },
        &[],
        false,
        &empty,
    );
    function.is_async = true;
    let asynchronous = gen_async_function_as_static_method(
        &function,
        &mapper,
        sets(),
        PhpFunctionTarget {
            core_import: "sample",
            error_converter: Some("conversion_error_to_php_err"),
        },
        &[],
        &empty,
    );
    for output in [sync, asynchronous] {
        assert!(output.contains(".map_err(conversion_error_to_php_err)?"), "{output}");
    }
}

/// A JSON-typed parameter that does not parse is refused with an error that names the parameter.
/// It never reaches the core call as a default value.
#[test]
fn should_refuse_invalid_json_in_php_free_function_params() {
    use super::{PhpFunctionTarget, PhpParamTypeSets, gen_function_as_static_method};
    use crate::backends::php::type_map::PhpMapper;
    use crate::core::ir::{FunctionDef, ParamDef, TypeRef};
    use ahash::AHashSet;

    let empty = AHashSet::new();
    let mapper = PhpMapper {
        enum_names: empty.clone(),
        data_enum_names: empty.clone(),
        untagged_data_enum_names: empty.clone(),
        json_string_enum_names: empty.clone(),
    };
    for optional in [false, true] {
        let function = FunctionDef {
            name: "send".into(),
            rust_path: "sample::send".into(),
            return_type: TypeRef::String,
            params: vec![ParamDef {
                name: "payload".into(),
                ty: TypeRef::Json,
                optional,
                ..Default::default()
            }],
            ..Default::default()
        };
        let output = gen_function_as_static_method(
            &function,
            &mapper,
            PhpParamTypeSets {
                opaque: &empty,
                default: &empty,
                enums: &empty,
            },
            PhpFunctionTarget {
                core_import: "sample",
                error_converter: None,
            },
            &[],
            false,
            &empty,
        );
        assert!(output.contains("from_str"), "positive control, optional={optional}: {output}");
        assert!(
            !output.contains("unwrap_or_default()") && !output.contains(".ok()"),
            "optional={optional}: {output}"
        );
        assert!(output.contains("invalid JSON in `payload`"), "optional={optional}: {output}");
    }
}

/// Same rule for a static method: the wrapper returns a result so the refusal can reach PHP.
#[test]
fn should_refuse_invalid_json_in_php_static_method_params() {
    use super::gen_static_method;
    use crate::backends::php::type_map::PhpMapper;
    use crate::core::ir::{MethodDef, ParamDef, TypeDef, TypeRef};
    use ahash::AHashSet;

    let empty = AHashSet::new();
    let mapper = PhpMapper {
        enum_names: empty.clone(),
        data_enum_names: empty.clone(),
        untagged_data_enum_names: empty.clone(),
        json_string_enum_names: empty.clone(),
    };
    let typ = TypeDef {
        name: "Sample".into(),
        rust_path: "sample::Sample".into(),
        ..Default::default()
    };
    let method = MethodDef {
        name: "from_value".into(),
        return_type: TypeRef::String,
        params: vec![ParamDef {
            name: "payload".into(),
            ty: TypeRef::Json,
            ..Default::default()
        }],
        ..Default::default()
    };
    let output = gen_static_method(&method, &mapper, &empty, &typ, &empty);
    assert!(output.contains("from_str"), "positive control: {output}");
    assert!(
        !output.contains("unwrap_or_default()") && !output.contains(".ok()"),
        "{output}"
    );
    assert!(output.contains("invalid JSON in `payload`"), "{output}");
    assert!(output.contains("PhpResult<String>"), "{output}");
}
