// Shims for instance methods on *value* types — the non-opaque structs the
// Kotlin/Android binding materialises as `data class`es.
//
// Unlike client shims there is no handle to cast: the Kotlin object already
// holds every field, so it serialises `this` and the shim rebuilds the core
// receiver with serde before delegating to the real core method. That keeps
// semantics the Kotlin side cannot reproduce — value clamping in the
// `PaddleOcrConfig::with_*` builders, `validate()`'s error text — in exactly
// one place.
//
// Parameters always arrive as a JSON object keyed by parameter name. The
// single-parameter shorthands used by [`emit_method_shim`] are deliberately not
// reused here so the Kotlin caller has one encoding to implement.

/// Emit a value-method shim for every bridgeable instance method on `ty`.
fn emit_value_type_shims(
    out: &mut String,
    ty: &TypeDef,
    package: &str,
    bridge: &str,
    serde_type_names: &std::collections::HashSet<&str>,
) {
    for method in bridgeable_value_methods(ty, serde_type_names) {
        let symbol = jni_symbol(package, bridge, &bridge_method_name(&ty.name, &method.name));
        emit_value_method_shim(out, &symbol, &ty.name, method);
    }
}

fn emit_value_method_shim(out: &mut String, symbol: &str, type_name: &str, method: &MethodDef) {
    let rust_method = method.name.replace('-', "_");
    let returns_receiver = is_functional_ref_mut_value_method(method);
    let return_type = value_method_return_type(type_name, method);
    let ret_decl = method_return_type_decl(&return_type);
    let ret_null = method_return_null(&return_type);

    let request_param = if method.params.is_empty() {
        String::new()
    } else {
        "    request_json: JString,\n".to_string()
    };

    out.push_str(&template_env::render(
        "value_method_shim_open.rs.jinja",
        crate::alef_context! {
            symbol => symbol,
            request_param => request_param,
            ret_decl => ret_decl,
        },
    ));

    out.push_str(&template_env::render(
        "value_method_receiver.rs.jinja",
        crate::alef_context! {
            type_name => type_name,
            ret_null => ret_null,
        },
    ));

    let call_args = emit_value_param_unmarshal(out, &method.params, ret_null);

    let call_expr = if returns_receiver {
        format!("{{ client.{rust_method}({call_args}); client }}")
    } else {
        format!("client.{rust_method}({call_args})")
    };

    if method.error_type.is_some() {
        let mut ok_body = String::new();
        emit_value_method_newtype_return(&mut ok_body, method, "            ");
        emit_return_marshal(&mut ok_body, &return_type, ret_null);
        render_call_result_body(
            out,
            &call_expr,
            &CallResultShape {
                is_async: false,
                has_error: true,
                ret_null,
                ok_body: &ok_body,
                value_body: "",
            },
        );
    } else {
        let mut value_body = String::new();
        emit_value_method_newtype_return(&mut value_body, method, "    ");
        emit_return_marshal_with_indent(&mut value_body, &return_type, "    ", ret_null);
        render_call_result_body(
            out,
            &call_expr,
            &CallResultShape {
                is_async: false,
                has_error: false,
                ret_null,
                ok_body: "",
                value_body: &value_body,
            },
        );
    }
}

fn emit_value_method_newtype_return(out: &mut String, method: &MethodDef, indent: &str) {
    let Some(wrapper) = &method.return_newtype_wrapper else {
        return;
    };
    let converted = jni_return_newtype_from_core("v", wrapper);
    out.push_str(&format!("{indent}let v = {converted};\n"));
}

/// Unmarshal every parameter out of the `request_json` object and return the
/// comma-joined call-site argument list.
fn emit_value_param_unmarshal(out: &mut String, params: &[ParamDef], ret_null: &str) -> String {
    if params.is_empty() {
        return String::new();
    }
    out.push_str(&template_env::render(
        "request_map_unmarshal.rs.jinja",
        crate::alef_context! {
            ret_null => ret_null,
        },
    ));

    let mut args = Vec::with_capacity(params.len());
    for param in params {
        let rust_name = param.name.replace('-', "_");
        let is_path = matches!(&param.ty, TypeRef::Path);
        let type_path = if is_path {
            "String".to_string()
        } else {
            type_ref_to_core_path_with_btree(&param.ty, "core_crate", param.map_is_btree)
        };
        let type_path = if param.optional {
            format!("Option<{type_path}>")
        } else {
            type_path
        };
        out.push_str(&template_env::render(
            "request_map_param_unmarshal.rs.jinja",
            crate::alef_context! {
                name => rust_name,
                type_path => type_path,
                ret_null => ret_null,
            },
        ));
        if is_path && param.optional {
            out.push_str(&format!(
                "    let {rust_name} = {rust_name}.map(std::path::PathBuf::from);\n"
            ));
        } else if is_path {
            out.push_str(&format!(
                "    let {rust_name} = std::path::PathBuf::from({rust_name});\n"
            ));
        }
        if param.newtype_wrapper.is_some() {
            args.push(method_newtype_call_arg(out, param, &rust_name, param.optional));
        } else if needs_vec_string_refs(param, &param.ty) {
            out.push_str(&render_vec_string_refs_binding(&rust_name));
            args.push(vec_string_refs_arg(&rust_name));
        } else if param.is_ref {
            args.push(format!("&{rust_name}"));
        } else {
            args.push(rust_name);
        }
    }
    args.join(", ")
}
