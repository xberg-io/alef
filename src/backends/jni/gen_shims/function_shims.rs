struct FunctionReturnShape<'a> {
    capsule: Option<(&'a crate::core::config::FfiCapsuleTypeConfig, bool)>,
    opaque: bool,
    optional_opaque: bool,
}

struct FunctionParamProjection {
    signature: String,
    unmarshal: String,
    call_arg: String,
}

/// Emit a shim for a top-level API function.
#[allow(clippy::too_many_arguments)]
fn emit_function_shim(
    out: &mut String,
    symbol: &str,
    function: &crate::core::ir::FunctionDef,
    opaque_type_names: &std::collections::HashSet<&str>,
    capsule_types: &std::collections::HashMap<String, crate::core::config::FfiCapsuleTypeConfig>,
    core_crate_prefix: &str,
) {
    let shape = function_return_shape(&function.return_type, opaque_type_names, capsule_types);
    let return_null = function_return_null(&function.return_type, &shape);
    let (param_sigs, unmarshal, call_args) = project_function_params(&function.params, opaque_type_names, return_null);
    out.push_str(&template_env::render(
        "function_shim_open.rs.jinja",
        crate::alef_context! {
            symbol => symbol,
            param_sigs => param_sigs,
            ret_decl => function_return_declaration(&function.return_type, &shape),
        },
    ));
    out.push_str(&unmarshal);
    let core_function = core_function_path(&function.rust_path, core_crate_prefix);
    let call = if call_args.is_empty() {
        format!("{core_function}()")
    } else {
        format!("{core_function}({call_args})")
    };
    emit_function_return(out, function, &shape, &call, return_null);
}

fn function_return_shape<'a>(
    return_type: &TypeRef,
    opaque_type_names: &std::collections::HashSet<&str>,
    capsule_types: &'a std::collections::HashMap<String, crate::core::config::FfiCapsuleTypeConfig>,
) -> FunctionReturnShape<'a> {
    let opaque = matches!(return_type, TypeRef::Named(name) if opaque_type_names.contains(name.as_str()));
    let optional_opaque = matches!(
        return_type,
        TypeRef::Optional(inner)
            if matches!(inner.as_ref(), TypeRef::Named(name) if opaque_type_names.contains(name.as_str()))
    );
    let capsule = match return_type {
        TypeRef::Named(name) => capsule_types.get(name).map(|config| (config, false)),
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(name) => capsule_types.get(name).map(|config| (config, true)),
            _ => None,
        },
        _ => None,
    };
    FunctionReturnShape {
        capsule,
        opaque,
        optional_opaque,
    }
}

fn function_return_declaration(return_type: &TypeRef, shape: &FunctionReturnShape<'_>) -> String {
    if shape.opaque || shape.optional_opaque || shape.capsule.is_some() {
        " -> jlong".to_string()
    } else {
        method_return_type_decl(return_type)
    }
}

fn function_return_null<'a>(return_type: &'a TypeRef, shape: &FunctionReturnShape<'_>) -> &'a str {
    if shape.opaque || shape.optional_opaque || shape.capsule.is_some() {
        "0"
    } else {
        method_return_null(return_type)
    }
}

fn core_function_path(rust_path: &str, core_crate_prefix: &str) -> String {
    let path = rust_path.replace('-', "_");
    let prefix = format!("{}::", core_crate_prefix.replace('-', "_"));
    if path.starts_with(&prefix) {
        path.replacen(&prefix, "core_crate::", 1)
    } else if let Some((_sibling_crate, item)) = path.split_once("::") {
        format!("core_crate::{item}")
    } else {
        format!("core_crate::{path}")
    }
}

fn project_function_params(
    params: &[ParamDef],
    opaque_type_names: &std::collections::HashSet<&str>,
    return_null: &str,
) -> (String, String, String) {
    let mut signatures = String::new();
    let mut unmarshal = String::new();
    let mut call_args = Vec::new();
    for param in params {
        let projection = project_function_param(param, opaque_type_names, return_null);
        signatures.push_str(&projection.signature);
        unmarshal.push_str(&projection.unmarshal);
        call_args.push(projection.call_arg);
    }
    (signatures, unmarshal, call_args.join(", "))
}

fn project_function_param(
    param: &ParamDef,
    opaque_type_names: &std::collections::HashSet<&str>,
    return_null: &str,
) -> FunctionParamProjection {
    let rust_name = param.name.replace('-', "_");
    let base_type = method_param_base_type(param);
    let projection = match base_type {
        TypeRef::String => project_string_function_param(param, &rust_name, return_null),
        TypeRef::Primitive(primitive) => project_primitive_function_param(param, primitive, &rust_name, return_null),
        TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Primitive(PrimitiveType::U8)) => {
            project_bytes_function_param(param, &rust_name, return_null)
        }
        TypeRef::Bytes => project_bytes_function_param(param, &rust_name, return_null),
        TypeRef::Path => project_path_function_param(param, &rust_name, return_null),
        TypeRef::Named(type_name) if opaque_type_names.contains(type_name.as_str()) => {
            project_opaque_function_param(type_name, &rust_name, return_null)
        }
        _ => project_complex_function_param(param, base_type, &rust_name, return_null),
    };
    project_function_newtype_param(param, base_type, &rust_name, projection)
}

fn project_function_newtype_param(
    param: &ParamDef,
    base_type: &TypeRef,
    rust_name: &str,
    mut projection: FunctionParamProjection,
) -> FunctionParamProjection {
    if param.newtype_wrapper.is_none() {
        return projection;
    }
    let source = if param.optional && matches!(base_type, TypeRef::String) {
        format!("if {rust_name}.is_empty() {{ None }} else {{ Some({rust_name}) }}")
    } else {
        rust_name.to_string()
    };
    let converted = jni_param_newtype_to_core(&source, param).expect("newtype wrapper is present");
    if !param.is_ref {
        projection.call_arg = converted;
        return projection;
    }
    let binding = format!("{rust_name}_newtype");
    projection
        .unmarshal
        .push_str(&format!("    let {binding} = {converted};\n"));
    projection.call_arg = if param.optional {
        format!("{binding}.as_ref()")
    } else {
        format!("&{binding}")
    };
    projection
}

fn project_string_function_param(param: &ParamDef, rust_name: &str, return_null: &str) -> FunctionParamProjection {
    let call_arg = if param.optional {
        let payload = if param.is_ref {
            format!("&{rust_name}")
        } else {
            rust_name.to_string()
        };
        format!("if {rust_name}.is_empty() {{ None }} else {{ Some({payload}) }}")
    } else if param.is_ref {
        format!("&{rust_name}")
    } else {
        rust_name.to_string()
    };
    FunctionParamProjection {
        signature: render_param_decl(rust_name, "JString"),
        unmarshal: render_string_unmarshal(rust_name, return_null),
        call_arg,
    }
}

fn project_primitive_function_param(
    param: &ParamDef,
    primitive: &PrimitiveType,
    rust_name: &str,
    return_null: &str,
) -> FunctionParamProjection {
    if matches!(primitive, PrimitiveType::U32) {
        let binding = format!("{rust_name}_checked");
        let unmarshal = if param.optional {
            format!(
                "        let {binding} = if {rust_name} == -1 {{ None }} else {{\n            match u32::try_from({rust_name}) {{\n                Ok(value) => Some(value),\n                Err(_) => {{ throw_jni_error(env, \"parameter '{rust_name}' must be between 0 and u32::MAX\"); return {return_null}; }}\n            }}\n        }};\n"
            )
        } else {
            format!(
                "        let {binding} = match u32::try_from({rust_name}) {{\n            Ok(value) => value,\n            Err(_) => {{ throw_jni_error(env, \"parameter '{rust_name}' must be between 0 and u32::MAX\"); return {return_null}; }}\n        }};\n"
            )
        };
        return FunctionParamProjection {
            signature: render_param_decl(rust_name, jni_primitive_type(primitive)),
            unmarshal,
            call_arg: binding,
        };
    }
    let cast = primitive_cast(primitive);
    let cast_expression = if cast.is_empty() {
        rust_name.to_string()
    } else {
        format!("{rust_name} as {cast}")
    };
    let call_arg = if param.optional {
        primitive_zero_literal(primitive).map_or_else(
            || format!("Some({cast_expression})"),
            |zero| format!("if {rust_name} != {zero} {{ Some({cast_expression}) }} else {{ None }}"),
        )
    } else {
        cast_expression
    };
    FunctionParamProjection {
        signature: render_param_decl(rust_name, jni_primitive_type(primitive)),
        unmarshal: String::new(),
        call_arg,
    }
}

fn project_bytes_function_param(param: &ParamDef, rust_name: &str, return_null: &str) -> FunctionParamProjection {
    FunctionParamProjection {
        signature: render_param_decl(rust_name, "JString"),
        unmarshal: render_base64_bytes_unmarshal(rust_name, return_null, param.optional),
        call_arg: bytes_call_arg(rust_name, param.optional, param.is_ref),
    }
}

fn project_path_function_param(param: &ParamDef, rust_name: &str, return_null: &str) -> FunctionParamProjection {
    let mut unmarshal = render_string_unmarshal(rust_name, return_null);
    unmarshal.push_str(&format!(
        "    let {rust_name} = std::path::PathBuf::from({rust_name});\n"
    ));
    let call_arg = if param.optional {
        format!("if {rust_name}.as_os_str().is_empty() {{ None }} else {{ Some({rust_name}) }}")
    } else if param.is_ref {
        format!("&{rust_name}")
    } else {
        rust_name.to_string()
    };
    FunctionParamProjection {
        signature: render_param_decl(rust_name, "JString"),
        unmarshal,
        call_arg,
    }
}

fn project_opaque_function_param(type_name: &str, rust_name: &str, return_null: &str) -> FunctionParamProjection {
    FunctionParamProjection {
        signature: render_param_decl(rust_name, "jlong"),
        unmarshal: template_env::render(
            "opaque_handle_unmarshal.rs.jinja",
            crate::alef_context! {
                name => rust_name,
                type_path => format!("core_crate::{type_name}"),
                ret_null => return_null,
            },
        ),
        call_arg: rust_name.to_string(),
    }
}

fn project_complex_function_param(
    param: &ParamDef,
    base_type: &TypeRef,
    rust_name: &str,
    return_null: &str,
) -> FunctionParamProjection {
    let type_path = type_ref_to_core_path_with_btree(base_type, "core_crate", param.map_is_btree);
    let mut unmarshal = render_complex_unmarshal(rust_name, &type_path, return_null, param.optional);
    if param.is_ref && param.is_mut && !param.optional {
        unmarshal.push_str(&format!("    let mut {rust_name} = {rust_name};\n"));
    }
    let call_arg = if param.optional {
        rust_name.to_string()
    } else if needs_vec_string_refs(param, base_type) {
        unmarshal.push_str(&render_vec_string_refs_binding(rust_name));
        vec_string_refs_arg(rust_name)
    } else if param.is_ref && param.is_mut {
        format!("&mut {rust_name}")
    } else if param.is_ref {
        format!("&{rust_name}")
    } else {
        rust_name.to_string()
    };
    FunctionParamProjection {
        signature: render_param_decl(rust_name, "JString"),
        unmarshal,
        call_arg,
    }
}

fn emit_function_return(
    out: &mut String,
    function: &crate::core::ir::FunctionDef,
    shape: &FunctionReturnShape<'_>,
    call: &str,
    return_null: &str,
) {
    let has_error = function.error_type.is_some();
    let indent = if has_error { "            " } else { "    " };
    let body = render_function_return_body(function, shape, indent, return_null);
    let (ok_body, value_body) = if has_error {
        (body.as_str(), "")
    } else {
        ("", body.as_str())
    };
    render_call_result_body(
        out,
        call,
        &CallResultShape {
            is_async: function.is_async,
            has_error,
            ret_null: return_null,
            ok_body,
            value_body,
        },
    );
}

fn render_function_return_body(
    function: &crate::core::ir::FunctionDef,
    shape: &FunctionReturnShape<'_>,
    indent: &str,
    return_null: &str,
) -> String {
    if let Some((capsule, optional)) = shape.capsule {
        return render_method_capsule_return(capsule, optional, function.returns_ref, function.returns_cow, indent);
    }
    if shape.opaque {
        let value = capsule_owned_value("v", function.returns_ref, function.returns_cow);
        return format!("{indent}Box::into_raw(Box::new({value})) as jlong\n");
    }
    if shape.optional_opaque {
        let value = capsule_owned_value("inner", function.returns_ref, function.returns_cow);
        return format!(
            "{indent}match v {{\n{indent}    None => 0i64,\n{indent}    Some(inner) => \
             Box::into_raw(Box::new({value})) as jlong,\n{indent}}}\n"
        );
    }
    let mut body = String::new();
    if let Some(wrapper) = &function.return_newtype_wrapper {
        let converted = jni_return_newtype_from_core("v", wrapper);
        body.push_str(&format!("{indent}let v = {converted};\n"));
    }
    emit_return_marshal_with_indent(&mut body, &function.return_type, indent, return_null);
    body
}

#[cfg(test)]
mod function_shims_tests {
    use super::*;
    use crate::core::ir::ParamDef;

    fn primitive_param(name: &str, primitive: PrimitiveType) -> ParamDef {
        ParamDef {
            name: name.to_string(),
            ty: TypeRef::Primitive(primitive),
            ..Default::default()
        }
    }

    /// Regression test for the liter-llm sighting: generated JNI shim code contained
    /// `record_cost_usd(..., cost_usd as f64)` where `cost_usd` is already `f64`, tripping
    /// `clippy::unnecessary_cast`. This test was red before the fix (`call_arg` contained
    /// `" as f64"`).
    #[test]
    fn f64_param_call_arg_has_no_cast() {
        let param = primitive_param("cost_usd", PrimitiveType::F64);
        let projection = project_primitive_function_param(&param, &PrimitiveType::F64, "cost_usd", "0");
        assert_eq!(projection.call_arg, "cost_usd");
    }

    /// Sibling positive control: a genuinely-needed cast (JNI wire type `jlong` differs from
    /// `u64`) must still be emitted -- the fix must not remove casts wholesale.
    #[test]
    fn u64_param_call_arg_still_casts() {
        let param = primitive_param("count", PrimitiveType::U64);
        let projection = project_primitive_function_param(&param, &PrimitiveType::U64, "count", "0");
        assert_eq!(projection.call_arg, "count as u64");
    }

    #[test]
    fn optional_u32_distinguishes_none_from_zero_and_rejects_other_negatives() {
        let mut param = primitive_param("max", PrimitiveType::U32);
        param.optional = true;
        let projection = project_primitive_function_param(&param, &PrimitiveType::U32, "max", "0");

        assert!(
            projection.unmarshal.contains("max == -1 { None }"),
            "{}",
            projection.unmarshal
        );
        assert!(
            projection.unmarshal.contains("Ok(value) => Some(value)"),
            "{}",
            projection.unmarshal
        );
        assert!(
            projection.unmarshal.contains("u32::try_from(max)"),
            "{}",
            projection.unmarshal
        );
        assert!(
            !projection.unmarshal.contains("max <= 0"),
            "Some(0) must survive: {}",
            projection.unmarshal
        );
    }
}
