struct MethodReturnShape<'a> {
    capsule: Option<(&'a crate::core::config::FfiCapsuleTypeConfig, bool)>,
    opaque: bool,
    optional_opaque: bool,
}

/// The receiver-identifying values `emit_method_shim`/`emit_method_shim_header` need alongside
/// the method itself: which client type owns the handle and how it must be borrowed. Built once
/// by the caller in `client_shims.rs` (which already computes these three values) and threaded
/// through unchanged, keeping both functions' own parameter counts under the lint limit. ~keep
struct MethodShimReceiver<'a> {
    type_name: &'a str,
    receiver_is_mut: bool,
    receiver_owned: bool,
}

/// Emit a shim for an instance method on an opaque client type.
fn emit_method_shim(
    out: &mut String,
    symbol: &str,
    method: &MethodDef,
    receiver: &MethodShimReceiver<'_>,
    opaque_type_names: &std::collections::HashSet<&str>,
    capsule_types: &std::collections::HashMap<String, crate::core::config::FfiCapsuleTypeConfig>,
) {
    let shape = method_return_shape(&method.return_type, opaque_type_names, capsule_types);
    let return_null = method_return_null_value(&method.return_type, &shape);
    emit_method_shim_header(out, symbol, method, receiver, &shape, return_null);
    let call_args = emit_method_call_args(out, &method.params, return_null);
    let rust_method = method.name.replace('-', "_");
    let call_expression = method_call_expression(&rust_method, &call_args);
    emit_method_return(out, method, &shape, &call_expression, return_null);
}

fn method_return_shape<'a>(
    return_type: &TypeRef,
    opaque_type_names: &std::collections::HashSet<&str>,
    capsule_types: &'a std::collections::HashMap<String, crate::core::config::FfiCapsuleTypeConfig>,
) -> MethodReturnShape<'a> {
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
    MethodReturnShape {
        capsule,
        opaque,
        optional_opaque,
    }
}

fn emit_method_shim_header(
    out: &mut String,
    symbol: &str,
    method: &MethodDef,
    receiver: &MethodShimReceiver<'_>,
    shape: &MethodReturnShape<'_>,
    return_null: &str,
) {
    out.push_str(&template_env::render(
        "method_shim_open.rs.jinja",
        crate::alef_context! {
            symbol => symbol,
            request_param => method_request_param(&method.params),
            ret_decl => method_return_declaration(&method.return_type, shape),
        },
    ));
    out.push_str(&template_env::render(
        "method_client_handle.rs.jinja",
        crate::alef_context! {
            receiver_owned => receiver.receiver_owned,
            receiver_is_mut => receiver.receiver_is_mut,
            type_name => receiver.type_name,
            ret_null => return_null,
        },
    ));
}

fn method_return_declaration(return_type: &TypeRef, shape: &MethodReturnShape<'_>) -> String {
    if shape.opaque || shape.optional_opaque || shape.capsule.is_some() {
        " -> jlong".to_string()
    } else {
        method_return_type_decl(return_type)
    }
}

fn method_return_null_value<'a>(return_type: &'a TypeRef, shape: &MethodReturnShape<'_>) -> &'a str {
    if shape.opaque || shape.optional_opaque || shape.capsule.is_some() {
        "0"
    } else {
        method_return_null(return_type)
    }
}

fn method_request_param(params: &[ParamDef]) -> String {
    let [param] = params else {
        return if params.is_empty() {
            String::new()
        } else {
            "    request_json: JString,\n".to_string()
        };
    };
    let rust_name = param.name.replace('-', "_");
    match method_param_base_type(param) {
        TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Primitive(PrimitiveType::U8)) => {
            render_param_decl(&rust_name, "jbyteArray")
        }
        TypeRef::Bytes => render_param_decl(&rust_name, "jbyteArray"),
        _ => "    request_json: JString,\n".to_string(),
    }
}

fn method_param_base_type(param: &ParamDef) -> &TypeRef {
    match &param.ty {
        TypeRef::Optional(inner) => inner.as_ref(),
        other => other,
    }
}

fn emit_method_call_args(out: &mut String, params: &[ParamDef], return_null: &str) -> String {
    match params {
        [] => String::new(),
        [param] => emit_single_method_call_arg(out, param, return_null),
        _ => emit_multi_method_call_args(out, params, return_null),
    }
}

fn emit_single_method_call_arg(out: &mut String, param: &ParamDef, return_null: &str) -> String {
    let rust_name = param.name.replace('-', "_");
    let base_type = method_param_base_type(param);
    let produces_option = single_unmarshal_produces_option(param, base_type);
    emit_single_param_unmarshal(
        out,
        &rust_name,
        base_type,
        return_null,
        produces_option,
        param.map_is_btree,
    );
    method_call_arg(out, param, base_type, &rust_name, produces_option)
}

fn single_unmarshal_produces_option(param: &ParamDef, base_type: &TypeRef) -> bool {
    param.optional
        && (matches!(base_type, TypeRef::Bytes)
            || matches!(base_type, TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Primitive(PrimitiveType::U8)))
            || !matches!(base_type, TypeRef::Vec(_) | TypeRef::Path | TypeRef::String))
}

fn method_call_arg(
    out: &mut String,
    param: &ParamDef,
    base_type: &TypeRef,
    rust_name: &str,
    produces_option: bool,
) -> String {
    if param.newtype_wrapper.is_some() {
        return method_newtype_call_arg(out, param, rust_name, produces_option);
    }
    if produces_option {
        return if param.is_ref && is_byte_slice(base_type) {
            format!("{rust_name}.as_deref()")
        } else {
            rust_name.to_string()
        };
    }
    if param.optional {
        return if param.is_ref {
            format!("Some(&{rust_name})")
        } else {
            format!("Some({rust_name})")
        };
    }
    if needs_vec_string_refs(param, base_type) {
        out.push_str(&render_vec_string_refs_binding(rust_name));
        return vec_string_refs_arg(rust_name);
    }
    if param.is_ref {
        format!("&{rust_name}")
    } else {
        rust_name.to_string()
    }
}

fn method_newtype_call_arg(out: &mut String, param: &ParamDef, rust_name: &str, produces_option: bool) -> String {
    let source = if param.optional && !produces_option {
        format!("Some({rust_name})")
    } else {
        rust_name.to_string()
    };
    let converted = jni_param_newtype_to_core(&source, param).expect("newtype wrapper is present");
    if !param.is_ref {
        return converted;
    }
    let binding = format!("{rust_name}_newtype");
    out.push_str(&format!("    let {binding} = {converted};\n"));
    if param.optional {
        format!("{binding}.as_ref()")
    } else {
        format!("&{binding}")
    }
}

fn emit_multi_method_call_args(out: &mut String, params: &[ParamDef], return_null: &str) -> String {
    out.push_str(&template_env::render(
        "request_map_unmarshal.rs.jinja",
        crate::alef_context! { ret_null => return_null },
    ));
    params
        .iter()
        .map(|param| {
            let rust_name = emit_map_method_param_unmarshal(out, param, return_null);
            method_call_arg(out, param, method_param_base_type(param), &rust_name, false)
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn emit_map_method_param_unmarshal(out: &mut String, param: &ParamDef, return_null: &str) -> String {
    let rust_name = param.name.replace('-', "_");
    let base_type = method_param_base_type(param);
    let is_path = matches!(base_type, TypeRef::Path);
    let type_path = if is_byte_slice(base_type) {
        "Vec<u8>".to_string()
    } else if is_path {
        "String".to_string()
    } else {
        type_ref_to_core_path_with_btree(base_type, "core_crate", param.map_is_btree)
    };
    out.push_str(&template_env::render(
        "request_map_param_unmarshal.rs.jinja",
        crate::alef_context! {
            name => rust_name,
            type_path => type_path,
            ret_null => return_null,
        },
    ));
    if is_path {
        out.push_str(&format!(
            "    let {rust_name} = std::path::PathBuf::from({rust_name});\n"
        ));
    }
    rust_name
}

fn method_call_expression(rust_method: &str, call_args: &str) -> String {
    if call_args.is_empty() {
        format!("client.{rust_method}()")
    } else {
        format!("client.{rust_method}({call_args})")
    }
}

fn emit_method_return(
    out: &mut String,
    method: &MethodDef,
    shape: &MethodReturnShape<'_>,
    call_expression: &str,
    return_null: &str,
) {
    let has_error = method.error_type.is_some();
    let indent = if has_error { "            " } else { "    " };
    let body = render_method_return_body(method, shape, indent, return_null);
    let (ok_body, value_body) = if has_error {
        (body.as_str(), "")
    } else {
        ("", body.as_str())
    };
    render_call_result_body(
        out,
        call_expression,
        &CallResultShape {
            is_async: method.is_async,
            has_error,
            ret_null: return_null,
            ok_body,
            value_body,
        },
    );
}

fn render_method_return_body(
    method: &MethodDef,
    shape: &MethodReturnShape<'_>,
    indent: &str,
    return_null: &str,
) -> String {
    if let Some((capsule, optional)) = shape.capsule {
        return render_method_capsule_return(capsule, optional, method.returns_ref, method.returns_cow, indent);
    }
    if shape.opaque {
        return format!("{indent}Box::into_raw(Box::new(v)) as jlong\n");
    }
    if shape.optional_opaque {
        return format!(
            "{indent}match v {{\n{indent}    None => 0i64,\n{indent}    Some(inner) => \
             Box::into_raw(Box::new(inner)) as jlong,\n{indent}}}\n"
        );
    }
    let mut body = String::new();
    if let Some(wrapper) = &method.return_newtype_wrapper {
        let converted = jni_return_newtype_from_core("v", wrapper);
        body.push_str(&format!("{indent}let v = {converted};\n"));
    }
    emit_return_marshal_with_indent(&mut body, &method.return_type, indent, return_null);
    body
}

fn render_method_capsule_return(
    capsule: &crate::core::config::FfiCapsuleTypeConfig,
    optional: bool,
    returns_ref: bool,
    returns_cow: bool,
    indent: &str,
) -> String {
    let direct_value = capsule_owned_value("v", returns_ref, returns_cow);
    let inner_value = capsule_owned_value("inner", returns_ref, returns_cow);
    template_env::render(
        "method_capsule_return.rs.jinja",
        crate::alef_context! {
            indent => indent,
            into_raw_type => capsule.into_raw_type,
            optional => optional,
            direct_value => direct_value,
            inner_value => inner_value,
        },
    )
}

fn capsule_owned_value(binding: &str, returns_ref: bool, returns_cow: bool) -> String {
    if returns_cow {
        format!("{binding}.into_owned()")
    } else if returns_ref {
        format!("{binding}.clone()")
    } else {
        binding.to_string()
    }
}

/// The result-shaping values `render_call_result_body` needs alongside the call expression
/// itself. Local to this (merged, `include!`-based) `gen_shims` module: shared by its three call
/// sites here, in `function_shims.rs`, and in `value_method_shims.rs`, each of which already
/// computes these values before rendering. ~keep
struct CallResultShape<'a> {
    is_async: bool,
    has_error: bool,
    ret_null: &'a str,
    ok_body: &'a str,
    value_body: &'a str,
}

fn render_call_result_body(out: &mut String, call_expr: &str, shape: &CallResultShape<'_>) {
    let async_call_expr = format!("runtime().block_on({call_expr})");
    out.push_str(&template_env::render(
        "call_result_body.rs.jinja",
        crate::alef_context! {
            call_expr => call_expr,
            async_call_expr => async_call_expr,
            is_async => shape.is_async,
            has_error => shape.has_error,
            ret_null => shape.ret_null,
            ok_body => shape.ok_body,
            value_body => shape.value_body,
        },
    ));
}
