use super::functions::{is_bytes_result_method, params_require_marshal};
use super::types::{cgo_type_for_primitive, emit_type_doc, go_return_expr, primitive_max_sentinel};
use crate::backends::go::type_map::{go_optional_type, go_type, go_zero_value};
use crate::codegen::naming::{go_param_name, go_type_name, to_go_name};
use crate::core::ir::{MethodDef, ParamDef, TypeDef, TypeRef};
use heck::ToSnakeCase;

/// Generate a streaming wrapper for a method decorated with the `Streaming` adapter pattern.
///
/// The returned Go method consumes the FFI iterator-handle exports
/// (`<prefix>_<type>_<method>_start`, `_next`, `_free`) and exposes a typed
/// `<-chan <ItemType>` to Go callers. A goroutine drives `_next` until null
/// (clean end-of-stream) or an error is signalled, then frees the handle.
#[allow(clippy::too_many_arguments)]
pub(super) fn gen_streaming_method_wrapper(
    typ: &TypeDef,
    method: &MethodDef,
    ffi_prefix: &str,
    item_type: &str,
    data_enum_names: &std::collections::HashSet<&str>,
    opaque_names: &std::collections::HashSet<&str>,
    _value_only_types: &std::collections::HashSet<String>,
    enum_names: &std::collections::HashSet<String>,
    ffi_param_enum_names: &std::collections::HashSet<String>,
) -> String {
    let mut out = String::with_capacity(2048);

    let method_go_name = to_go_name(&method.name);
    emit_type_doc(&mut out, &method_go_name, &method.doc, "is a streaming method.");

    let receiver_name = if typ.is_opaque { "h" } else { "r" };
    let go_receiver_type = go_type_name(&typ.name);
    let item_go_type = go_type_name(item_type);

    let item_is_sum_type = data_enum_names.contains(item_type);

    let params: Vec<String> = method
        .params
        .iter()
        .map(|p| {
            let param_type: String = if p.optional {
                go_optional_type(&p.ty).into_owned()
            } else if let TypeRef::Named(name) = &p.ty {
                if opaque_names.contains(name.as_str()) {
                    format!("*{}", go_type(&p.ty))
                } else {
                    go_type(&p.ty).into_owned()
                }
            } else {
                go_type(&p.ty).into_owned()
            };
            format!("{} {}", go_param_name(&p.name), param_type)
        })
        .collect();

    out.push_str(&crate::backends::go::template_env::render(
        "streaming_method_signature.jinja",
        minijinja::context! {
            receiver_name => receiver_name,
            receiver_type => &go_receiver_type,
            method_name => &method_go_name,
            params => params.join(", "),
            item_type => &item_go_type,
        },
    ));

    for param in &method.params {
        out.push_str(&gen_param_to_c(
            param,
            "nil, ",
            true,
            ffi_prefix,
            opaque_names,
            enum_names,
            ffi_param_enum_names,
        ));
    }

    let c_params: Vec<String> = method
        .params
        .iter()
        .flat_map(|p| -> Vec<String> {
            let c_name = go_param_name(&format!("c_{}", p.name));
            if matches!(p.ty, TypeRef::Bytes) {
                vec![c_name.clone(), format!("{}Len", c_name)]
            } else {
                vec![c_name]
            }
        })
        .collect();

    let type_snake = typ.name.to_snake_case();
    let method_snake = method.name.to_snake_case();
    let item_snake = item_type.to_snake_case();
    let upper_prefix = ffi_prefix.to_uppercase();

    let c_receiver = format!(
        "(*C.{}{})(unsafe.Pointer({}.ptr))",
        upper_prefix, typ.name, receiver_name
    );
    let start_call = if c_params.is_empty() {
        format!("C.{}_{}_{}_start({})", ffi_prefix, type_snake, method_snake, c_receiver)
    } else {
        format!(
            "C.{}_{}_{}_start({}, {})",
            ffi_prefix,
            type_snake,
            method_snake,
            c_receiver,
            c_params.join(", "),
        )
    };

    out.push_str(&crate::backends::go::template_env::render(
        "streaming_method_body.jinja",
        minijinja::context! {
            start_call => &start_call,
            ffi_prefix => ffi_prefix,
            type_snake => &type_snake,
            method_snake => &method_snake,
            item_snake => &item_snake,
            item_type => &item_go_type,
            item_is_sum_type => item_is_sum_type,
        },
    ));

    out
}

/// Generate a wrapper method for a struct method.
pub(super) fn gen_method_wrapper(
    typ: &TypeDef,
    method: &MethodDef,
    ffi_prefix: &str,
    opaque_names: &std::collections::HashSet<&str>,
    value_only_types: &std::collections::HashSet<String>,
    enum_names: &std::collections::HashSet<String>,
    ffi_param_enum_names: &std::collections::HashSet<String>,
) -> String {
    let mut out = String::with_capacity(2048);

    let method_go_name = to_go_name(&method.name);

    emit_type_doc(&mut out, &method_go_name, &method.doc, "is a method.");

    let receiver_requires_marshal = !method.is_static && !typ.is_opaque;
    let method_marshals = receiver_requires_marshal || params_require_marshal(&method.params, opaque_names);
    let method_can_return_error = method.error_type.is_some() || method_marshals;

    let is_bytes_result = is_bytes_result_method(method);

    let return_type = if is_bytes_result {
        "([]byte, error)".to_string()
    } else if method_can_return_error {
        if matches!(method.return_type, TypeRef::Unit) {
            "error".to_string()
        } else {
            let ret_go_type = if matches!(
                method.return_type,
                TypeRef::Primitive(_) | TypeRef::Duration | TypeRef::String | TypeRef::Char | TypeRef::Path
            ) {
                go_type(&method.return_type).into_owned()
            } else {
                go_optional_type(&method.return_type).into_owned()
            };
            format!("({}, error)", ret_go_type)
        }
    } else if matches!(method.return_type, TypeRef::Unit) {
        "".to_string()
    } else if matches!(
        method.return_type,
        TypeRef::Primitive(_) | TypeRef::Duration | TypeRef::String | TypeRef::Char | TypeRef::Path
    ) {
        go_type(&method.return_type).into_owned()
    } else {
        go_optional_type(&method.return_type).into_owned()
    };

    let receiver_name = if typ.is_opaque { "h" } else { "r" };
    let go_receiver_type = go_type_name(&typ.name);

    let params: Vec<String> = method
        .params
        .iter()
        .map(|p| {
            let param_type: String = if p.optional {
                go_optional_type(&p.ty).into_owned()
            } else if let TypeRef::Named(name) = &p.ty {
                if opaque_names.contains(name.as_str()) {
                    format!("*{}", go_type(&p.ty))
                } else {
                    go_type(&p.ty).into_owned()
                }
            } else {
                go_type(&p.ty).into_owned()
            };
            format!("{} {}", go_param_name(&p.name), param_type)
        })
        .collect();
    let params_str = params.join(", ");

    let ret_type_str = if return_type.is_empty() {
        String::new()
    } else {
        format!(" {return_type}")
    };

    if method.is_static {
        out.push_str(&crate::backends::go::template_env::render(
            "method_signature_static.jinja",
            minijinja::context! {
                receiver_type => &go_receiver_type,
                method_name => &method_go_name,
                params => &params_str,
                return_type => &ret_type_str,
            },
        ));
    } else {
        out.push_str(&crate::backends::go::template_env::render(
            "method_signature_instance.jinja",
            minijinja::context! {
                receiver_name => receiver_name,
                receiver_type => &go_receiver_type,
                method_name => &method_go_name,
                params => &params_str,
                return_type => &ret_type_str,
            },
        ));
    }

    {
        let returns_value_and_error = method_can_return_error && !matches!(method.return_type, TypeRef::Unit);
        let param_err_return_prefix: String = if returns_value_and_error {
            format!("{}, ", go_zero_value(&method.return_type))
        } else {
            String::new()
        };
        for param in &method.params {
            out.push_str(&gen_param_to_c(
                param,
                &param_err_return_prefix,
                method_can_return_error,
                ffi_prefix,
                opaque_names,
                enum_names,
                ffi_param_enum_names,
            ));
        }

        let c_params: Vec<String> = method
            .params
            .iter()
            .flat_map(|p| -> Vec<String> {
                let c_name = go_param_name(&format!("c_{}", p.name));
                if matches!(p.ty, TypeRef::Bytes) {
                    vec![c_name.clone(), format!("{}Len", c_name)]
                } else {
                    vec![c_name]
                }
            })
            .collect();

        let type_snake = typ.name.to_snake_case();
        let method_snake = method.name.to_snake_case();
        let base_c_call = if method.is_static {
            if c_params.is_empty() {
                format!("C.{}_{}_{}()", ffi_prefix, type_snake, method_snake)
            } else {
                format!(
                    "C.{}_{}_{}({})",
                    ffi_prefix,
                    type_snake,
                    method_snake,
                    c_params.join(", ")
                )
            }
        } else if typ.is_opaque {
            let c_receiver = format!(
                "(*C.{}{})(unsafe.Pointer({}.ptr))",
                ffi_prefix.to_uppercase(),
                typ.name,
                receiver_name
            );
            if c_params.is_empty() {
                format!("C.{}_{}_{}({})", ffi_prefix, type_snake, method_snake, c_receiver)
            } else {
                format!(
                    "C.{}_{}_{}({}, {})",
                    ffi_prefix,
                    type_snake,
                    method_snake,
                    c_receiver,
                    c_params.join(", ")
                )
            }
        } else {
            let err_prefix = if returns_value_and_error {
                format!("{}, ", go_zero_value(&method.return_type))
            } else {
                String::new()
            };
            let err_action = format!("return {err_prefix}fmt.Errorf(\"failed to marshal receiver: %w\", err)");
            let from_json_err_action = format!(
                "return {err_prefix}fmt.Errorf(\"failed to create receiver: %s\", C.GoString(C.{ffi_prefix}_last_error_context()))"
            );
            out.push_str(&crate::backends::go::template_env::render(
                "marshal_receiver_to_c.jinja",
                minijinja::context! {
                    receiver_name => receiver_name,
                    err_action => &err_action,
                    from_json_err_action => &from_json_err_action,
                    ffi_prefix => ffi_prefix,
                    type_snake => &type_snake,
                },
            ));
            if c_params.is_empty() {
                format!("C.{}_{}_{}(cRecv)", ffi_prefix, type_snake, method_snake)
            } else {
                format!(
                    "C.{}_{}_{}(cRecv, {})",
                    ffi_prefix,
                    type_snake,
                    method_snake,
                    c_params.join(", ")
                )
            }
        };

        let c_call = if is_bytes_result {
            let base = base_c_call.trim_end_matches(')');
            if base.ends_with('(') {
                format!("{}&outPtr, &outLen, &outCap)", base)
            } else {
                format!("{}, &outPtr, &outLen, &outCap)", base)
            }
        } else {
            base_c_call
        };

        if is_bytes_result {
            out.push_str(&crate::backends::go::template_env::render(
                "bytes_result_call.jinja",
                minijinja::context! { c_call => &c_call, ffi_prefix => ffi_prefix },
            ));
            out.push_str("}\n");
            return out;
        }

        let is_builder_return = !method.is_static
            && typ.is_opaque
            && matches!(&method.return_type, TypeRef::Named(n) if n.as_str() == typ.name.as_str());

        if method_can_return_error {
            if matches!(method.return_type, TypeRef::Unit) {
                out.push_str(&crate::backends::go::template_env::render(
                    "c_call_unit.jinja",
                    minijinja::context! {
                        c_call => &c_call,
                    },
                ));
                if !method.is_static && !typ.is_opaque {
                    out.push_str(&crate::backends::go::template_env::render(
                        "method_update_from_json.jinja",
                        minijinja::context! {
                            ffi_prefix => ffi_prefix,
                            type_snake => &type_snake,
                            recv => receiver_name,
                        },
                    ));
                }
                if method.error_type.is_some() {
                    out.push_str("\treturn lastError()\n");
                } else {
                    out.push_str("\treturn nil\n");
                }
            } else {
                out.push_str(&crate::backends::go::template_env::render(
                    "c_call_with_ptr_assign.jinja",
                    minijinja::context! {
                        c_call => &c_call,
                    },
                ));
                if method.error_type.is_some() {
                    out.push_str("\tif err := lastError(); err != nil {\n");
                    if matches!(
                        method.return_type,
                        TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json
                    ) {
                        out.push_str("\t\tif ptr != nil {\n");
                        out.push_str(&crate::backends::go::template_env::render(
                            "free_string_on_error.jinja",
                            minijinja::context! {
                                ffi_prefix => ffi_prefix,
                            },
                        ));
                        out.push_str("\t\t}\n");
                    }
                    let zero_value = go_zero_value(&method.return_type);
                    out.push_str(&crate::backends::go::template_env::render(
                        "return_zero_err.jinja",
                        minijinja::context! {
                            zero_value => &zero_value,
                        },
                    ));
                    out.push_str("\t}\n");
                }
                if matches!(
                    method.return_type,
                    TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json
                ) {
                    out.push_str(&crate::backends::go::template_env::render(
                        "free_string.jinja",
                        minijinja::context! {
                            ffi_prefix => ffi_prefix,
                            ptr => "ptr",
                        },
                    ));
                }
                if let TypeRef::Named(name) = &method.return_type {
                    if !opaque_names.contains(name.as_str()) {
                        let type_snake = name.to_snake_case();
                        out.push_str(&crate::backends::go::template_env::render(
                            "free_type.jinja",
                            minijinja::context! {
                                ffi_prefix => ffi_prefix,
                                type_snake => &type_snake,
                                ptr => "ptr",
                            },
                        ));
                    }
                }
                if is_builder_return {
                    out.push_str(&crate::backends::go::template_env::render(
                        "receiver_ptr_assign.jinja",
                        minijinja::context! {
                            receiver_name => receiver_name,
                        },
                    ));
                    out.push_str(&crate::backends::go::template_env::render(
                        "return_value_and_nil.jinja",
                        minijinja::context! {
                            value => receiver_name,
                        },
                    ));
                } else {
                    let return_expr =
                        go_return_expr(&method.return_type, "ptr", ffi_prefix, opaque_names, value_only_types);
                    out.push_str(&crate::backends::go::template_env::render(
                        "method_return_simple.jinja",
                        minijinja::context! {
                            value => format!("{}, nil", return_expr),
                        },
                    ));
                }
            }
        } else if matches!(method.return_type, TypeRef::Unit) {
            out.push_str(&crate::backends::go::template_env::render(
                "c_call_simple.jinja",
                minijinja::context! {
                    c_call => &c_call,
                },
            ));
        } else {
            out.push_str(&crate::backends::go::template_env::render(
                "c_call_with_ptr_assign.jinja",
                minijinja::context! {
                    c_call => &c_call,
                },
            ));
            if matches!(
                method.return_type,
                TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json
            ) {
                out.push_str(&crate::backends::go::template_env::render(
                    "free_string.jinja",
                    minijinja::context! {
                        ffi_prefix => ffi_prefix,
                        ptr => "ptr",
                    },
                ));
            }
            if let TypeRef::Named(name) = &method.return_type {
                if !opaque_names.contains(name.as_str()) {
                    let type_snake = name.to_snake_case();
                    out.push_str(&crate::backends::go::template_env::render(
                        "free_type.jinja",
                        minijinja::context! {
                            ffi_prefix => ffi_prefix,
                            type_snake => &type_snake,
                            ptr => "ptr",
                        },
                    ));
                }
            }
            if is_builder_return {
                out.push_str(&crate::backends::go::template_env::render(
                    "method_receiver_ptr_assign.jinja",
                    minijinja::context! {
                        receiver_name => receiver_name,
                    },
                ));
                out.push_str(&crate::backends::go::template_env::render(
                    "method_return_simple.jinja",
                    minijinja::context! {
                        value => receiver_name,
                    },
                ));
            } else {
                let return_expr =
                    go_return_expr(&method.return_type, "ptr", ffi_prefix, opaque_names, value_only_types);
                out.push_str(&crate::backends::go::template_env::render(
                    "method_return_simple.jinja",
                    minijinja::context! {
                        value => return_expr,
                    },
                ));
            }
        }
    }

    out.push_str("}\n");
    out
}

/// Generate parameter conversion code from Go to C.
/// `err_return_prefix` is the leading `"<zero>, "` (or `""` for value-less returns) prepended to
/// every `return ... fmt.Errorf(...)` early exit. Callers compute it from the enclosing function's
/// return type — `"nil, "` for pointer/slice/channel returns, `"0, "` / `"false, "` / `"\"\", "`
/// for plain primitive/string returns, and `""` when the function only returns `error`.
/// `can_return_error` should be true when the enclosing function has `error` in its return type.
/// When false, marshal failures are handled with `panic` since the function signature has no error return.
pub(super) fn gen_param_to_c(
    param: &ParamDef,
    err_return_prefix: &str,
    can_return_error: bool,
    ffi_prefix: &str,
    opaque_names: &std::collections::HashSet<&str>,
    enum_names: &std::collections::HashSet<String>,
    ffi_param_enum_names: &std::collections::HashSet<String>,
) -> String {
    let mut out = String::with_capacity(512);
    let go_param = go_param_name(&param.name);
    let c_name = go_param_name(&format!("c_{}", param.name));

    match &param.ty {
        TypeRef::String | TypeRef::Char => {
            if param.optional {
                out.push_str(&crate::backends::go::template_env::render(
                    "param_string_optional.jinja",
                    minijinja::context! {
                        c_name => &c_name,
                        go_param => &go_param,
                    },
                ));
                out.push('\n');
            } else {
                out.push_str(&crate::backends::go::template_env::render(
                    "param_string_required.jinja",
                    minijinja::context! {
                        c_name => &c_name,
                        go_param => &go_param,
                    },
                ));
                out.push('\n');
            }
        }
        TypeRef::Path => {
            if param.optional {
                out.push_str(&crate::backends::go::template_env::render(
                    "param_string_optional.jinja",
                    minijinja::context! {
                        c_name => &c_name,
                        go_param => &go_param,
                    },
                ));
                out.push('\n');
            } else {
                out.push_str(&crate::backends::go::template_env::render(
                    "param_string_required.jinja",
                    minijinja::context! {
                        c_name => &c_name,
                        go_param => &go_param,
                    },
                ));
                out.push('\n');
            }
        }
        TypeRef::Bytes => {
            out.push_str(&crate::backends::go::template_env::render(
                "bytes_to_c_pointer.jinja",
                minijinja::context! {
                    c_name => &c_name,
                    go_param => &go_param,
                },
            ));
            out.push('\n');
        }
        TypeRef::Named(name) => {
            if opaque_names.contains(name.as_str()) {
                let c_type = format!("{}{}", ffi_prefix.to_uppercase(), name);
                out.push_str(&crate::backends::go::template_env::render(
                    "param_opaque_cast.jinja",
                    minijinja::context! {
                        c_name => &c_name,
                        go_param => &go_param,
                        c_type => &c_type,
                    },
                ));
                out.push('\n');
            } else if ffi_param_enum_names.contains(name) {
                let enum_snake = name.to_snake_case();
                out.push_str(&crate::backends::go::template_env::render(
                    "param_enum_to_i32.jinja",
                    minijinja::context! {
                        c_name => &c_name,
                        go_param => &go_param,
                        ffi_prefix => ffi_prefix,
                        enum_snake => &enum_snake,
                    },
                ));
                out.push('\n');
            } else if enum_names.contains(name) {
                let type_snake = name.to_snake_case();
                let err_action = if can_return_error {
                    format!("return {err_return_prefix}fmt.Errorf(\"failed to marshal: %w\", err)")
                } else {
                    "panic(fmt.Sprintf(\"failed to marshal: %v\", err))".to_string()
                };
                let from_json_err_action = if can_return_error {
                    format!(
                        "return {err_return_prefix}fmt.Errorf(\"failed to create {type_snake}: %s\", C.GoString(C.{ffi_prefix}_last_error_context()))"
                    )
                } else {
                    format!(
                        "panic(\"failed to create {type_snake}: \" + C.GoString(C.{ffi_prefix}_last_error_context()))"
                    )
                };
                out.push_str(&crate::backends::go::template_env::render(
                    "param_named_type.jinja",
                    minijinja::context! {
                        c_name => &c_name,
                        go_param => &go_param,
                        err_action => &err_action,
                        from_json_err_action => &from_json_err_action,
                        ffi_prefix => ffi_prefix,
                        type_snake => &type_snake,
                    },
                ));
                out.push('\n');
            } else {
                let type_snake = name.to_snake_case();
                let err_action = if can_return_error {
                    format!("return {err_return_prefix}fmt.Errorf(\"failed to marshal: %w\", err)")
                } else {
                    "panic(fmt.Sprintf(\"failed to marshal: %v\", err))".to_string()
                };
                let from_json_err_action = if can_return_error {
                    format!(
                        "return {err_return_prefix}fmt.Errorf(\"failed to create {type_snake}: %s\", C.GoString(C.{ffi_prefix}_last_error_context()))"
                    )
                } else {
                    format!(
                        "panic(\"failed to create {type_snake}: \" + C.GoString(C.{ffi_prefix}_last_error_context()))"
                    )
                };
                out.push_str(&crate::backends::go::template_env::render(
                    "param_named_type.jinja",
                    minijinja::context! {
                        c_name => &c_name,
                        go_param => &go_param,
                        err_action => &err_action,
                        from_json_err_action => &from_json_err_action,
                        ffi_prefix => ffi_prefix,
                        type_snake => &type_snake,
                    },
                ));
                out.push('\n');
            }
        }
        TypeRef::Vec(_) | TypeRef::Map(_, _) | TypeRef::Json => {
            let err_action = if can_return_error {
                format!("return {err_return_prefix}fmt.Errorf(\"failed to marshal: %w\", err)")
            } else {
                "panic(fmt.Sprintf(\"failed to marshal: %v\", err))".to_string()
            };
            out.push_str(&crate::backends::go::template_env::render(
                "param_vec_or_map.jinja",
                minijinja::context! {
                    c_name => &c_name,
                    go_param => &go_param,
                    err_action => &err_action,
                },
            ));
            out.push('\n');
        }
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::String | TypeRef::Char | TypeRef::Path => {
                out.push_str(&crate::backends::go::template_env::render(
                    "param_string_optional.jinja",
                    minijinja::context! {
                        c_name => &c_name,
                        go_param => &go_param,
                    },
                ));
                out.push('\n');
            }
            TypeRef::Named(name) if opaque_names.contains(name.as_str()) => {
                let c_type = format!("{}{}", ffi_prefix.to_uppercase(), name);
                out.push_str(&crate::backends::go::template_env::render(
                    "param_optional_opaque.jinja",
                    minijinja::context! {
                        c_name => &c_name,
                        c_type => &c_type,
                        go_param => &go_param,
                    },
                ));
                out.push('\n');
            }
            TypeRef::Named(_) => {
                out.push_str(&crate::backends::go::template_env::render(
                    "param_optional_named_inline.jinja",
                    minijinja::context! {
                        c_name => &c_name,
                        go_param => &go_param,
                    },
                ));
                out.push('\n');
            }
            _ => {
                out.push_str(&crate::backends::go::template_env::render(
                    "param_optional_decl.jinja",
                    minijinja::context! {
                        c_name => &c_name,
                    },
                ));
                out.push('\n');
            }
        },
        TypeRef::Primitive(prim) if !param.optional => {
            let cgo_ty = cgo_type_for_primitive(prim);
            let go_ty = go_type(&TypeRef::Primitive(prim.clone()));
            if matches!(prim, crate::core::ir::PrimitiveType::Bool) {
                out.push_str(&crate::backends::go::template_env::render(
                    "param_primitive_bool.jinja",
                    minijinja::context! {
                        c_name => &c_name,
                        cgo_ty => &cgo_ty,
                        go_param => &go_param,
                    },
                ));
                out.push('\n');
            } else {
                out.push_str(&crate::backends::go::template_env::render(
                    "param_primitive_numeric.jinja",
                    minijinja::context! {
                        c_name => &c_name,
                        cgo_ty => &cgo_ty,
                        go_ty => &go_ty,
                        go_param => &go_param,
                    },
                ));
                out.push('\n');
            }
        }
        TypeRef::Primitive(prim) if param.optional => {
            let cgo_ty = cgo_type_for_primitive(prim);
            let go_ty = go_type(&TypeRef::Primitive(prim.clone()));
            let sentinel = primitive_max_sentinel(prim);

            if matches!(prim, crate::core::ir::PrimitiveType::Bool) {
                out.push_str(&crate::backends::go::template_env::render(
                    "param_optional_primitive_bool.jinja",
                    minijinja::context! {
                        c_name => &c_name,
                        cgo_ty => &cgo_ty,
                        go_param => &go_param,
                    },
                ));
                out.push('\n');
            } else {
                out.push_str(&crate::backends::go::template_env::render(
                    "param_optional_primitive_numeric.jinja",
                    minijinja::context! {
                        c_name => &c_name,
                        cgo_ty => &cgo_ty,
                        go_ty => &go_ty,
                        sentinel => &sentinel,
                        go_param => &go_param,
                    },
                ));
                out.push('\n');
            }
        }
        _ => {}
    }

    if !out.is_empty() {
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests;
