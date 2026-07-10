/// Emit a shim for an instance method on an opaque client type.
///
/// `receiver_is_mut` controls whether the handle is cast to `*mut T` (`&mut self`)
/// or `*const T` (`&self`).  `opaque_type_names` is used to identify handle-typed
/// params so they can be received as `jlong` rather than a JSON string.
#[allow(clippy::too_many_arguments)]
fn emit_method_shim(
    out: &mut String,
    symbol: &str,
    type_name: &str,
    method_name: &str,
    params: &[ParamDef],
    return_type: &TypeRef,
    is_async: bool,
    has_error: bool,
    receiver_is_mut: bool,
    receiver_owned: bool,
    opaque_type_names: &std::collections::HashSet<&str>,
) {
    let rust_method = method_name.replace('-', "_");
    let has_params = !params.is_empty();

    let is_opaque_return = matches!(return_type, TypeRef::Named(n) if opaque_type_names.contains(n.as_str()));
    let is_optional_opaque_return = matches!(
        return_type,
        TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Named(n) if opaque_type_names.contains(n.as_str()))
    );

    let ret_decl = if is_opaque_return || is_optional_opaque_return {
        " -> jlong".to_string()
    } else {
        method_return_type_decl(return_type)
    };
    let ret_null = if is_opaque_return || is_optional_opaque_return {
        "0"
    } else {
        method_return_null(return_type)
    };

    let request_param = if !has_params {
        String::new()
    } else if params.len() == 1 {
        let p = &params[0];
        let rust_name = p.name.replace('-', "_");
        let base_ty = match &p.ty {
            TypeRef::Optional(inner) => inner.as_ref(),
            other => other,
        };
        match base_ty {
            TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Primitive(PrimitiveType::U8)) => {
                render_param_decl(&rust_name, "jbyteArray")
            }
            TypeRef::Bytes => render_param_decl(&rust_name, "jbyteArray"),
            _ => "    request_json: JString,\n".to_string(),
        }
    } else {
        "    request_json: JString,\n".to_string()
    };

    out.push_str(&template_env::render(
        "method_shim_open.rs.jinja",
        context! {
            symbol => symbol,
            request_param => request_param,
            ret_decl => ret_decl,
        },
    ));

    out.push_str(&template_env::render(
        "method_client_handle.rs.jinja",
        context! {
            receiver_owned => receiver_owned,
            receiver_is_mut => receiver_is_mut,
            type_name => type_name,
        },
    ));

    let call_args: String = if !has_params {
        String::new()
    } else if params.len() == 1 {
        let p = &params[0];
        let rust_name = p.name.replace('-', "_");
        let base_ty = match &p.ty {
            TypeRef::Optional(inner) => inner.as_ref(),
            other => other,
        };
        let unmarshal_produces_option = p.optional
            && (matches!(base_ty, TypeRef::Bytes)
                || matches!(base_ty, TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Primitive(PrimitiveType::U8)))
                || !matches!(base_ty, TypeRef::Vec(_) | TypeRef::Path | TypeRef::String));
        emit_single_param_unmarshal(
            out,
            &rust_name,
            base_ty,
            ret_null,
            unmarshal_produces_option,
            p.map_is_btree,
        );
        if unmarshal_produces_option {
            if p.is_ref && is_byte_slice(base_ty) {
                format!("{rust_name}.as_deref()")
            } else {
                rust_name
            }
        } else if p.optional {
            format!("Some({rust_name})")
        } else if needs_vec_string_refs(p, base_ty) {
            out.push_str(&render_vec_string_refs_binding(&rust_name));
            vec_string_refs_arg(&rust_name)
        } else if p.is_ref {
            format!("&{rust_name}")
        } else {
            rust_name
        }
    } else {
        out.push_str(&template_env::render(
            "request_map_unmarshal.rs.jinja",
            context! {
                ret_null => ret_null,
            },
        ));
        let mut args = Vec::new();
        for p in params {
            let rust_name = p.name.replace('-', "_");
            let base_ty = match &p.ty {
                TypeRef::Optional(inner) => inner.as_ref(),
                other => other,
            };
            let is_path = matches!(base_ty, TypeRef::Path);
            let type_path = if is_byte_slice(base_ty) {
                "Vec<u8>".to_string()
            } else if is_path {
                "String".to_string()
            } else {
                type_ref_to_core_path_with_btree(base_ty, "core_crate", p.map_is_btree)
            };
            out.push_str(&template_env::render(
                "request_map_param_unmarshal.rs.jinja",
                context! {
                    name => rust_name,
                    type_path => type_path,
                    ret_null => ret_null,
                },
            ));
            if is_path {
                out.push_str(&format!(
                    "    let {rust_name} = std::path::PathBuf::from({rust_name});\n"
                ));
            }
            let call_arg = if p.optional {
                if p.is_ref && is_byte_slice(base_ty) {
                    format!("Some(&{rust_name})")
                } else {
                    format!("Some({rust_name})")
                }
            } else if needs_vec_string_refs(p, base_ty) {
                out.push_str(&render_vec_string_refs_binding(&rust_name));
                vec_string_refs_arg(&rust_name)
            } else if p.is_ref {
                format!("&{rust_name}")
            } else {
                rust_name
            };
            args.push(call_arg);
        }
        args.join(", ")
    };

    let call_expr = if call_args.is_empty() {
        format!("client.{rust_method}()")
    } else {
        format!("client.{rust_method}({call_args})")
    };

    if has_error {
        let mut ok_body = String::new();
        if is_opaque_return {
            ok_body.push_str("            Box::into_raw(Box::new(v)) as jlong\n");
        } else if is_optional_opaque_return {
            ok_body.push_str("            match v {\n");
            ok_body.push_str("                None => 0i64,\n");
            ok_body.push_str("                Some(inner) => Box::into_raw(Box::new(inner)) as jlong,\n");
            ok_body.push_str("            }\n");
        } else {
            emit_return_marshal(&mut ok_body, return_type, ret_null);
        }
        render_call_result_body(out, &call_expr, is_async, true, ret_null, &ok_body, "");
    } else {
        let mut value_body = String::new();
        if is_opaque_return {
            value_body.push_str("    Box::into_raw(Box::new(v)) as jlong\n");
        } else if is_optional_opaque_return {
            value_body.push_str("    match v {\n");
            value_body.push_str("        None => 0i64,\n");
            value_body.push_str("        Some(inner) => Box::into_raw(Box::new(inner)) as jlong,\n");
            value_body.push_str("    }\n");
        } else {
            emit_return_marshal_with_indent(&mut value_body, return_type, "    ", ret_null);
        }
        render_call_result_body(out, &call_expr, is_async, false, ret_null, "", &value_body);
    }
}

fn render_call_result_body(
    out: &mut String,
    call_expr: &str,
    is_async: bool,
    has_error: bool,
    ret_null: &str,
    ok_body: &str,
    value_body: &str,
) {
    let async_call_expr = format!("runtime().block_on({call_expr})");
    out.push_str(&template_env::render(
        "call_result_body.rs.jinja",
        context! {
            call_expr => call_expr,
            async_call_expr => async_call_expr,
            is_async => is_async,
            has_error => has_error,
            ret_null => ret_null,
            ok_body => ok_body,
            value_body => value_body,
        },
    ));
}
