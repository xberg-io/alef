use super::*;

#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_function(
    f: &FunctionDef,
    prefix: &str,
    declared_errors: &[String],
    top_level_names: &std::collections::HashSet<String>,
    struct_names: &std::collections::HashSet<String>,
    opaque_creator_map: &std::collections::HashMap<String, (String, String)>,
    capsule_types: &std::collections::HashMap<String, crate::core::config::HostCapsuleTypeConfig>,
    out: &mut String,
) {
    // `opaque_type_name_inner` matches both bare `Named(name)` and `Optional(Named(name))` —
    // capsule returns share one raw C ABI (`*const T`) in both cases, see
    // `backends::ffi::gen_bindings::capsule::capsule_c_return_type` and `capsule_return_name`.
    // ~keep
    if let Some(name) = opaque_type_name_inner(&f.return_type)
        && let Some(cap) = capsule_types.get(name)
    {
        emit_capsule_function(f, prefix, struct_names, opaque_creator_map, cap, declared_errors, out);
        return;
    }

    emit_cleaned_zig_doc(out, &f.doc, "");

    let renamed_params: Vec<ParamDef> = f
        .params
        .iter()
        .map(|p| {
            let mut p2 = p.clone();
            if top_level_names.contains(&p2.name) {
                p2.name = format!("{}_arg", p2.name);
            }
            p2
        })
        .collect();
    let f_local = FunctionDef {
        params: renamed_params,
        ..f.clone()
    };
    let f = &f_local;

    let params: Vec<String> = f
        .params
        .iter()
        .map(|p| format_param_wrapper(p, struct_names, opaque_creator_map))
        .collect();

    let zig_error_type = f
        .error_type
        .as_ref()
        .map(|e| resolve_zig_error_type(e, declared_errors));

    let return_ty = wrapper_return_type(f, declared_errors, struct_names, opaque_creator_map);
    let emit_start = out.len();

    out.push_str(&crate::backends::zig::template_env::render(
        "function_signature.jinja",
        crate::alef_context! {
            func_name => &f.name,
            params => params.join(", "),
            return_ty => &return_ty,
        },
    ));

    let json_error_return = zig_error_type
        .as_ref()
        .map_or("return error.InvalidJson;".to_string(), |err| {
            format!("return _error_with_message({err});")
        });
    for p in &f.params {
        emit_param_conversion(p, prefix, struct_names, opaque_creator_map, &json_error_return, out);
    }

    let returns_bytes = return_uses_bytes_out_params(&f.return_type);
    let returns_optional_bytes = returns_bytes && matches!(f.return_type, TypeRef::Optional(_));
    if returns_bytes {
        out.push_str("    var _out_ptr: [*c]u8 = undefined;\n");
        out.push_str("    var _out_len: usize = 0;\n");
        out.push_str("    var _out_cap: usize = 0;\n");
    }

    let mut c_args: Vec<String> = f
        .params
        .iter()
        .flat_map(|p| c_arg_names(p, struct_names, opaque_creator_map))
        .collect();
    if returns_bytes {
        c_args.push("&_out_ptr".to_string());
        c_args.push("&_out_len".to_string());
        c_args.push("&_out_cap".to_string());
    }
    let c_call = format!("c.{prefix}_{}({})", f.name, c_args.join(", "));
    let returns_c_char_like = return_uses_len_companion(&f.return_type);
    let c_len_call = if returns_c_char_like {
        Some(format!("c.{prefix}_{}_len({})", f.name, c_args.join(", ")))
    } else {
        None
    };

    if let Some(gate) = free_function_presence_gate(
        f,
        prefix,
        struct_names,
        opaque_creator_map,
        &c_call,
        zig_error_type.as_deref(),
    ) {
        out.push_str(&gate);
    }

    if let Some(error_type) = &zig_error_type {
        let result_is_pointer = !(matches!(f.return_type, TypeRef::Unit) || returns_bytes);
        let result_can_be_null = return_type_can_be_null(&f.return_type, struct_names);
        if !result_is_pointer {
            out.push_str(&crate::backends::zig::template_env::render(
                "function_call_unit.jinja",
                crate::alef_context! {
                    c_call => &c_call,
                },
            ));
        } else {
            out.push_str(&crate::backends::zig::template_env::render(
                "function_call_result.jinja",
                crate::alef_context! {
                    c_call => &c_call,
                },
            ));
        }
        if result_is_pointer {
            out.push_str(&crate::backends::zig::template_env::render(
                "function_error_check.jinja",
                crate::alef_context! {
                    prefix => prefix,
                },
            ));
            out.push_str(&crate::backends::zig::template_env::render(
                "function_error_return.jinja",
                crate::alef_context! {
                    error_type => error_type,
                },
            ));
            out.push_str("    }\n");
            if result_can_be_null {
                out.push_str("    if (_result == null) {\n");
                out.push_str(&crate::backends::zig::template_env::render(
                    "function_error_return.jinja",
                    crate::alef_context! {
                        error_type => error_type,
                    },
                ));
                out.push_str("    }\n");
            }
        } else {
            out.push_str(&crate::backends::zig::template_env::render(
                "function_error_check.jinja",
                crate::alef_context! {
                    prefix => prefix,
                },
            ));
            out.push_str(&crate::backends::zig::template_env::render(
                "function_error_return.jinja",
                crate::alef_context! {
                    error_type => error_type,
                },
            ));
            out.push_str("    }\n");
        }
        if let Some(len_call) = &c_len_call {
            out.push_str(&crate::backends::zig::template_env::render(
                "function_result_len.jinja",
                crate::alef_context! {
                    len_call => len_call,
                },
            ));
        }

        for p in &f.params {
            emit_param_free(p, prefix, struct_names, opaque_creator_map, out);
        }

        if returns_bytes {
            if returns_optional_bytes {
                out.push_str("    if (_out_ptr == null) return null;\n");
            }
            out.push_str("    const _owned = try std.heap.c_allocator.dupe(u8, _out_ptr[0.._out_len]);\n");
            out.push_str(&crate::backends::zig::template_env::render(
                "function_free_bytes.jinja",
                crate::alef_context! {
                    prefix => prefix,
                },
            ));
            out.push_str("    return _owned;\n");
        } else if matches!(f.return_type, TypeRef::Unit) {
            out.push_str("    return;\n");
        } else {
            let ret_expr = unwrap_return_expr(
                "_result",
                &f.return_type,
                prefix,
                struct_names,
                Some(error_type.as_str()),
            );
            out.push_str(&crate::backends::zig::template_env::render(
                "function_return.jinja",
                crate::alef_context! {
                    ret_expr => ret_expr,
                },
            ));
        }
    } else {
        for p in &f.params {
            emit_param_free(p, prefix, struct_names, opaque_creator_map, out);
        }
        if returns_bytes {
            out.push_str(&crate::backends::zig::template_env::render(
                "function_call_unit.jinja",
                crate::alef_context! {
                    c_call => &c_call,
                },
            ));
            if returns_optional_bytes {
                out.push_str("    if (_out_ptr == null) return null;\n");
            }
            out.push_str("    const _owned = try std.heap.c_allocator.dupe(u8, _out_ptr[0.._out_len]);\n");
            out.push_str(&crate::backends::zig::template_env::render(
                "function_free_bytes.jinja",
                crate::alef_context! {
                    prefix => prefix,
                },
            ));
            out.push_str("    return _owned;\n");
        } else if matches!(f.return_type, TypeRef::Unit) {
            out.push_str(&crate::backends::zig::template_env::render(
                "function_call_unit.jinja",
                crate::alef_context! {
                    c_call => &c_call,
                },
            ));
        } else {
            out.push_str(&crate::backends::zig::template_env::render(
                "function_call_result.jinja",
                crate::alef_context! {
                    c_call => &c_call,
                },
            ));
            // The null check runs before the length call: every FFI entry point clears the
            // native error on entry, so a second call would erase the message. ~keep
            if infallible_result_needs_null_check(&f.return_type) {
                out.push_str("    if (_result == null) {\n");
                out.push_str(&crate::backends::zig::template_env::render(
                    "function_error_return.jinja",
                    crate::alef_context! {
                        error_type => UNMATCHED_ERROR_SET,
                    },
                ));
                out.push_str("    }\n");
            }
            if let Some(len_call) = &c_len_call {
                out.push_str(&crate::backends::zig::template_env::render(
                    "function_result_len.jinja",
                    crate::alef_context! {
                        len_call => len_call,
                    },
                ));
            }
            let ret_expr = unwrap_return_expr("_result", &f.return_type, prefix, struct_names, None);
            out.push_str(&crate::backends::zig::template_env::render(
                "function_return.jinja",
                crate::alef_context! {
                    ret_expr => ret_expr,
                },
            ));
        }
    }

    out.push_str("}\n");
    assert_error_set_covers_body(&f.name, &return_ty, &out[emit_start..], declared_errors);
}
