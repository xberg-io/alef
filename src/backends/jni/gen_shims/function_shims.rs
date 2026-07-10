/// Emit a shim for a top-level API function.
///
/// When the return type is an opaque named type the function returns `jlong`
/// (a raw `Box::into_raw` pointer) rather than a JSON-encoded `jstring`.
/// When a parameter is an opaque named type it is received as `jlong` and
/// dereferenced via an unsafe pointer cast — the Kotlin caller holds the
/// handle as a `Long` that was previously obtained from the constructor shim.
#[allow(clippy::too_many_arguments)]
fn emit_function_shim(
    out: &mut String,
    symbol: &str,
    rust_path: &str,
    params: &[ParamDef],
    return_type: &TypeRef,
    is_async: bool,
    has_error: bool,
    opaque_type_names: &std::collections::HashSet<&str>,
    core_crate_prefix: &str,
) {
    let path = rust_path.replace('-', "_");
    let from_prefix = format!("{}::", core_crate_prefix.replace('-', "_"));
    let core_fn = if path.starts_with(&from_prefix) {
        path.replacen(&from_prefix, "core_crate::", 1)
    } else {
        format!("core_crate::{path}")
    };

    let is_opaque_return = matches!(return_type, TypeRef::Named(n) if opaque_type_names.contains(n.as_str()));
    let ret_decl = if is_opaque_return {
        " -> jlong".to_string()
    } else {
        method_return_type_decl(return_type)
    };
    let err_null = if is_opaque_return {
        "0"
    } else {
        method_return_null(return_type)
    };

    let mut param_sigs = String::new();
    let mut unmarshal = String::new();
    let mut call_args = String::new();

    for p in params {
        let rust_name = p.name.replace('-', "_");
        let base_ty = match &p.ty {
            TypeRef::Optional(inner) => inner.as_ref(),
            other => other,
        };
        match base_ty {
            TypeRef::String => {
                param_sigs.push_str(&render_param_decl(&rust_name, "JString"));
                unmarshal.push_str(&render_string_unmarshal(&rust_name, err_null));
                if p.optional {
                    let some_payload = if p.is_ref {
                        format!("&{rust_name}")
                    } else {
                        rust_name.clone()
                    };
                    call_args.push_str("if ");
                    call_args.push_str(&rust_name);
                    call_args.push_str(".is_empty() { None } else { Some(");
                    call_args.push_str(&some_payload);
                    call_args.push_str(") }");
                } else if p.is_ref {
                    call_args.push('&');
                    call_args.push_str(&rust_name);
                } else {
                    call_args.push_str(&rust_name);
                }
            }
            TypeRef::Primitive(prim) => {
                let jni_ty = jni_primitive_type(prim);
                param_sigs.push_str(&render_param_decl(&rust_name, jni_ty));
                let cast = primitive_cast(prim);
                let cast_expr = if cast.is_empty() {
                    rust_name.clone()
                } else {
                    format!("{rust_name} as {cast}")
                };
                if p.optional {
                    let zero_lit = primitive_zero_literal(prim);
                    if let Some(zero) = zero_lit {
                        call_args.push_str("if ");
                        call_args.push_str(&rust_name);
                        call_args.push_str(" != ");
                        call_args.push_str(zero);
                        call_args.push_str(" { Some(");
                        call_args.push_str(&cast_expr);
                        call_args.push_str(") } else { None }");
                    } else {
                        call_args.push_str("Some(");
                        call_args.push_str(&cast_expr);
                        call_args.push(')');
                    }
                } else {
                    call_args.push_str(&cast_expr);
                }
            }
            TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Primitive(PrimitiveType::U8)) => {
                param_sigs.push_str(&render_param_decl(&rust_name, "JString"));
                unmarshal.push_str(&render_base64_bytes_unmarshal(&rust_name, err_null, p.optional));
                call_args.push_str(&bytes_call_arg(&rust_name, p.optional, p.is_ref));
            }
            TypeRef::Bytes => {
                param_sigs.push_str(&render_param_decl(&rust_name, "JString"));
                unmarshal.push_str(&render_base64_bytes_unmarshal(&rust_name, err_null, p.optional));
                call_args.push_str(&bytes_call_arg(&rust_name, p.optional, p.is_ref));
            }
            TypeRef::Path => {
                param_sigs.push_str(&render_param_decl(&rust_name, "JString"));
                unmarshal.push_str(&render_string_unmarshal(&rust_name, err_null));
                unmarshal.push_str(&format!(
                    "    let {rust_name} = std::path::PathBuf::from({rust_name});\n"
                ));
                if p.optional {
                    call_args.push_str("if ");
                    call_args.push_str(&rust_name);
                    call_args.push_str(".as_os_str().is_empty() { None } else { Some(");
                    call_args.push_str(&rust_name);
                    call_args.push_str(") }");
                } else if p.is_ref {
                    call_args.push('&');
                    call_args.push_str(&rust_name);
                } else {
                    call_args.push_str(&rust_name);
                }
            }
            TypeRef::Named(type_name) if opaque_type_names.contains(type_name.as_str()) => {
                // SAFETY: the Kotlin caller holds a Long obtained from the matching
                param_sigs.push_str(&render_param_decl(&rust_name, "jlong"));
                let type_path = format!("core_crate::{type_name}");
                unmarshal.push_str(&template_env::render(
                    "opaque_handle_unmarshal.rs.jinja",
                    context! {
                        name => rust_name,
                        type_path => type_path,
                    },
                ));
                call_args.push_str(&rust_name);
            }
            _ => {
                param_sigs.push_str(&render_param_decl(&rust_name, "JString"));
                let type_path = type_ref_to_core_path_with_btree(base_ty, "core_crate", p.map_is_btree);
                if p.optional {
                    unmarshal.push_str(&render_complex_unmarshal(&rust_name, &type_path, err_null, true));
                    call_args.push_str(&rust_name);
                } else {
                    unmarshal.push_str(&render_complex_unmarshal(&rust_name, &type_path, err_null, false));
                    if p.is_ref && p.is_mut {
                        unmarshal.push_str(&format!("    let mut {rust_name} = {rust_name};\n"));
                    }
                    if needs_vec_string_refs(p, base_ty) {
                        unmarshal.push_str(&render_vec_string_refs_binding(&rust_name));
                        call_args.push_str(&vec_string_refs_arg(&rust_name));
                    } else if p.is_ref {
                        if p.is_mut {
                            call_args.push_str("&mut ");
                        } else {
                            call_args.push('&');
                        }
                        call_args.push_str(&rust_name);
                    } else {
                        call_args.push_str(&rust_name);
                    }
                }
            }
        }
        call_args.push_str(", ");
    }
    if call_args.ends_with(", ") {
        call_args.truncate(call_args.len() - 2);
    }

    out.push_str(&template_env::render(
        "function_shim_open.rs.jinja",
        context! {
            symbol => symbol,
            param_sigs => param_sigs,
            ret_decl => ret_decl,
        },
    ));

    out.push_str(&unmarshal);

    let raw_call = if call_args.is_empty() {
        format!("{core_fn}()")
    } else {
        format!("{core_fn}({call_args})")
    };

    if has_error {
        let mut ok_body = String::new();
        if is_opaque_return {
            ok_body.push_str("            Box::into_raw(Box::new(v)) as jlong\n");
        } else {
            emit_return_marshal_with_indent(&mut ok_body, return_type, "            ", err_null);
        }
        render_call_result_body(out, &raw_call, is_async, true, err_null, &ok_body, "");
    } else {
        let mut value_body = String::new();
        if is_opaque_return {
            value_body.push_str("    Box::into_raw(Box::new(v)) as jlong\n");
        } else {
            emit_return_marshal_with_indent(&mut value_body, return_type, "    ", err_null);
        }
        render_call_result_body(out, &raw_call, is_async, false, err_null, "", &value_body);
    }
}
