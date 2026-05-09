use super::functions::{is_bytes_result_method, params_require_marshal};
use super::types::{cgo_type_for_primitive, emit_type_doc, go_return_expr, primitive_max_sentinel};
use crate::type_map::{go_optional_type, go_type};
use alef_codegen::naming::{go_param_name, go_type_name, to_go_name};
use alef_core::ir::{MethodDef, ParamDef, TypeDef, TypeRef};
use heck::ToSnakeCase;
use std::fmt::Write;

/// Generate a streaming wrapper for a method decorated with the `Streaming` adapter pattern.
///
/// The returned Go method consumes the FFI iterator-handle exports
/// (`<prefix>_<type>_<method>_start`, `_next`, `_free`) and exposes a typed
/// `<-chan <ItemType>` to Go callers. A goroutine drives `_next` until null
/// (clean end-of-stream) or an error is signalled, then frees the handle.
pub(super) fn gen_streaming_method_wrapper(
    typ: &TypeDef,
    method: &MethodDef,
    ffi_prefix: &str,
    item_type: &str,
    opaque_names: &std::collections::HashSet<&str>,
) -> String {
    let mut out = String::with_capacity(2048);

    let method_go_name = to_go_name(&method.name);
    emit_type_doc(&mut out, &method_go_name, &method.doc, "is a streaming method.");

    // Receiver name follows the pattern used by gen_method_wrapper: opaque -> "h", non-opaque -> "r".
    let receiver_name = if typ.is_opaque { "h" } else { "r" };
    let go_receiver_type = go_type_name(&typ.name);
    let item_go_type = go_type_name(item_type);

    // Build the parameter list mirroring gen_method_wrapper. We do not honour
    // bridge stripping here because streaming adapters never use trait bridges.
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

    // Function signature: receiver, name, params, return type.
    writeln!(
        out,
        "func ({} *{}) {}({}) (<-chan {}, error) {{",
        receiver_name,
        go_receiver_type,
        method_go_name,
        params.join(", "),
        item_go_type,
    )
    .ok();

    // Marshal each parameter exactly like gen_method_wrapper does for the
    // synchronous case. The start function's signature accepts the same C
    // request handle as the regular `chat` method, so reuse the existing
    // gen_param_to_c emitter (returning `(value, error)`-shape).
    for param in &method.params {
        write!(
            out,
            "{}",
            gen_param_to_c(
                param,
                /* returns_value_and_error = */ true,
                /* can_return_error = */ true,
                ffi_prefix,
                opaque_names,
            )
        )
        .ok();
    }

    // Build the C parameter list (e.g. `cReq`) — same as gen_method_wrapper.
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

    // Start the stream. Non-static streaming methods always have a non-null
    // opaque receiver — cast `h.ptr` like other opaque-receiver methods do.
    let c_receiver = format!("(*C.{}{})(unsafe.Pointer({}.ptr))", upper_prefix, typ.name, receiver_name);
    let start_call = if c_params.is_empty() {
        format!(
            "C.{}_{}_{}_start({})",
            ffi_prefix, type_snake, method_snake, c_receiver
        )
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

    // Body: open the handle, spawn a goroutine, deliver chunks on the channel.
    write!(
        out,
        "\thandle := {start_call}\n\
         \tif handle == nil {{\n\
         \t\tif err := lastError(); err != nil {{\n\
         \t\t\treturn nil, err\n\
         \t\t}}\n\
         \t\treturn nil, fmt.Errorf(\"failed to start {method_snake} stream\")\n\
         \t}}\n\
         \tch := make(chan {item_go_type})\n\
         \tgo func() {{\n\
         \t\tdefer close(ch)\n\
         \t\tdefer C.{ffi_prefix}_{type_snake}_{method_snake}_free(handle)\n\
         \t\tfor {{\n\
         \t\t\tchunkPtr := C.{ffi_prefix}_{type_snake}_{method_snake}_next(handle)\n\
         \t\t\tif chunkPtr == nil {{\n\
         \t\t\t\t// Null = clean end-of-stream (errno 0) or stream error (errno != 0).\n\
         \t\t\t\t// In either case there are no more chunks; close the channel.\n\
         \t\t\t\treturn\n\
         \t\t\t}}\n\
         \t\t\tjsonPtr := C.{ffi_prefix}_{item_snake}_to_json(chunkPtr)\n\
         \t\t\tif jsonPtr == nil {{\n\
         \t\t\t\tC.{ffi_prefix}_{item_snake}_free(chunkPtr)\n\
         \t\t\t\treturn\n\
         \t\t\t}}\n\
         \t\t\tvar chunk {item_go_type}\n\
         \t\t\tunmarshalErr := json.Unmarshal([]byte(C.GoString(jsonPtr)), &chunk)\n\
         \t\t\tC.{ffi_prefix}_free_string(jsonPtr)\n\
         \t\t\tC.{ffi_prefix}_{item_snake}_free(chunkPtr)\n\
         \t\t\tif unmarshalErr != nil {{\n\
         \t\t\t\treturn\n\
         \t\t\t}}\n\
         \t\t\tch <- chunk\n\
         \t\t}}\n\
         \t}}()\n\
         \treturn ch, nil\n\
         }}\n",
        start_call = start_call,
        ffi_prefix = ffi_prefix,
        type_snake = type_snake,
        method_snake = method_snake,
        item_snake = item_snake,
        item_go_type = item_go_type,
    )
    .ok();

    out
}

/// Generate a wrapper method for a struct method.
pub(super) fn gen_method_wrapper(
    typ: &TypeDef,
    method: &MethodDef,
    ffi_prefix: &str,
    opaque_names: &std::collections::HashSet<&str>,
) -> String {
    let mut out = String::with_capacity(2048);

    let method_go_name = to_go_name(&method.name);

    emit_type_doc(&mut out, &method_go_name, &method.doc, "is a method.");

    // A non-opaque, non-static method marshals its receiver to JSON — that is fallible.
    // Also include params that require marshaling.
    let receiver_requires_marshal = !method.is_static && !typ.is_opaque;
    let method_marshals = receiver_requires_marshal || params_require_marshal(&method.params, opaque_names);
    let method_can_return_error = method.error_type.is_some() || method_marshals;

    // Detect Result<Vec<u8>> — uses out-param convention, always returns ([]byte, error).
    let is_bytes_result = is_bytes_result_method(method);

    let return_type = if is_bytes_result {
        // Out-param bytes result always returns ([]byte, error)
        "([]byte, error)".to_string()
    } else if method_can_return_error {
        if matches!(method.return_type, TypeRef::Unit) {
            "error".to_string()
        } else {
            format!("({}, error)", go_optional_type(&method.return_type))
        }
    } else if matches!(method.return_type, TypeRef::Unit) {
        "".to_string()
    } else {
        go_optional_type(&method.return_type).into_owned()
    };

    // Opaque types use "h" (for "handle") to match the receiver name in Free().
    // Non-opaque types use "r" (for "receiver").
    let receiver_name = if typ.is_opaque { "h" } else { "r" };
    let go_receiver_type = go_type_name(&typ.name);

    // Static methods become package-level functions (no receiver in Go)
    if method.is_static {
        out.push_str(&crate::template_env::render(
            "method_receiver_static.jinja",
            minijinja::context! {
                receiver_type => &go_receiver_type,
                method_name => &method_go_name,
            },
        ));
    } else {
        out.push_str(&crate::template_env::render(
            "method_receiver_instance.jinja",
            minijinja::context! {
                receiver_name => receiver_name,
                receiver_type => &go_receiver_type,
                method_name => &method_go_name,
            },
        ));
    }

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
    out.push_str(&params.join(", "));

    if return_type.is_empty() {
        out.push_str(&crate::template_env::render(
            "method_empty_return.jinja",
            minijinja::Value::default(),
        ));
    } else {
        out.push_str(&crate::template_env::render(
            "method_return.jinja",
            minijinja::context! {
                return_type => &return_type,
            },
        ));
    }

    {
        // Synchronous method - just convert params and call FFI
        // Note: method_can_return_error is set above (includes synthesized error for marshal-requiring methods).
        let returns_value_and_error = method_can_return_error && !matches!(method.return_type, TypeRef::Unit);
        for param in &method.params {
            write!(
                out,
                "{}",
                gen_param_to_c(
                    param,
                    returns_value_and_error,
                    method_can_return_error,
                    ffi_prefix,
                    opaque_names
                )
            )
            .ok();
        }

        // Bytes params expand to two C arguments: the pointer and the length.
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
            // Static methods don't pass a receiver
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
            // Opaque types have a ptr field — cast it directly.
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
            // Non-opaque structs: marshal to JSON, create a temporary handle, use it, and free it.
            let err_prefix = if returns_value_and_error { "nil, " } else { "" };
            // method_can_return_error is always true here (receiver_requires_marshal is true for
            // non-opaque non-static methods), so we always emit fmt.Errorf, never panic.
            let err_action = format!("return {err_prefix}fmt.Errorf(\"failed to marshal receiver: %w\", err)");
            let from_json_err_action = format!(
                "return {err_prefix}fmt.Errorf(\"failed to create receiver: %s\", C.GoString(C.{ffi_prefix}_last_error_context()))"
            );
            out.push_str(&crate::template_env::render(
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

        // For Result<Vec<u8>> (bytes_result), append the three out-param references to the call.
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

        // Result<Vec<u8>> uses the out-param convention — emit specialized body and return early.
        if is_bytes_result {
            out.push_str(&crate::template_env::render(
                "bytes_result_call.jinja",
                minijinja::context! { c_call => &c_call, ffi_prefix => ffi_prefix },
            ));
            out.push_str("}\n");
            return out;
        }

        // Detect builder pattern: opaque type method that returns the same opaque type.
        // The C function consumes (Box::from_raw) the input pointer and returns a new pointer.
        // Instead of creating a new Go struct, update r.ptr so the caller's handle stays valid.
        let is_builder_return =
            typ.is_opaque && matches!(&method.return_type, TypeRef::Named(n) if n.as_str() == typ.name.as_str());

        if method_can_return_error {
            if matches!(method.return_type, TypeRef::Unit) {
                out.push_str(&crate::template_env::render(
                    "c_call_unit.jinja",
                    minijinja::context! {
                        c_call => &c_call,
                    },
                ));
                // For non-opaque, non-static methods with Unit return, the C function may have
                // mutated cRecv in place (e.g. apply_update).  Write the updated state back to
                // the Go receiver so the mutation is visible to the caller.
                if !method.is_static && !typ.is_opaque {
                    out.push_str(&crate::template_env::render(
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
                out.push_str(&crate::template_env::render(
                    "c_call_with_ptr_assign.jinja",
                    minijinja::context! {
                        c_call => &c_call,
                    },
                ));
                if method.error_type.is_some() {
                    out.push_str("\tif err := lastError(); err != nil {\n");
                    // Free the pointer if non-nil even on error, to avoid leaks.
                    // Bytes pointers are NOT freed — they alias internal storage.
                    if matches!(
                        method.return_type,
                        TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json
                    ) {
                        out.push_str("\t\tif ptr != nil {\n");
                        out.push_str(&crate::template_env::render(
                            "free_string_on_error.jinja",
                            minijinja::context! {
                                ffi_prefix => ffi_prefix,
                            },
                        ));
                        out.push_str("\t\t}\n");
                    }
                    out.push_str("\t\treturn nil, err\n");
                    out.push_str("\t}\n");
                }
                // Free the FFI-allocated string after unmarshaling.
                // Bytes pointers are NOT freed — they alias internal storage.
                if matches!(
                    method.return_type,
                    TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json
                ) {
                    out.push_str(&crate::template_env::render(
                        "free_string.jinja",
                        minijinja::context! {
                            ffi_prefix => ffi_prefix,
                            ptr => "ptr",
                        },
                    ));
                }
                // For non-opaque Named return types, free the handle after JSON extraction.
                // Opaque types are NOT freed here — the caller owns them via the Go wrapper.
                if let TypeRef::Named(name) = &method.return_type {
                    if !opaque_names.contains(name.as_str()) {
                        let type_snake = name.to_snake_case();
                        out.push_str(&crate::template_env::render(
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
                    // Builder pattern: C consumed the old pointer and returned a new one.
                    // Update r.ptr in-place so the caller's handle remains valid.
                    out.push_str(&crate::template_env::render(
                        "receiver_ptr_assign.jinja",
                        minijinja::context! {
                            receiver_name => receiver_name,
                        },
                    ));
                    out.push_str(&crate::template_env::render(
                        "return_value_and_nil.jinja",
                        minijinja::context! {
                            value => receiver_name,
                        },
                    ));
                } else {
                    let return_expr = go_return_expr(&method.return_type, "ptr", ffi_prefix, opaque_names);
                    out.push_str(&crate::template_env::render(
                        "method_return_simple.jinja",
                        minijinja::context! {
                            value => format!("{}, nil", return_expr),
                        },
                    ));
                }
            }
        } else if matches!(method.return_type, TypeRef::Unit) {
            out.push_str(&crate::template_env::render(
                "c_call_simple.jinja",
                minijinja::context! {
                    c_call => &c_call,
                },
            ));
        } else {
            out.push_str(&crate::template_env::render(
                "c_call_with_ptr_assign.jinja",
                minijinja::context! {
                    c_call => &c_call,
                },
            ));
            // Add defer free for C string returns.
            // Bytes pointers are NOT freed — they alias internal storage.
            if matches!(
                method.return_type,
                TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json
            ) {
                out.push_str(&crate::template_env::render(
                    "free_string.jinja",
                    minijinja::context! {
                        ffi_prefix => ffi_prefix,
                        ptr => "ptr",
                    },
                ));
            }
            // For non-opaque Named return types, free the handle after JSON extraction.
            // Opaque types are NOT freed here — the caller owns them via the Go wrapper.
            if let TypeRef::Named(name) = &method.return_type {
                if !opaque_names.contains(name.as_str()) {
                    let type_snake = name.to_snake_case();
                    out.push_str(&crate::template_env::render(
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
                // Builder pattern: C consumed the old pointer and returned a new one.
                // Update r.ptr in-place so the caller's handle remains valid.
                out.push_str(&crate::template_env::render(
                    "method_receiver_ptr_assign.jinja",
                    minijinja::context! {
                        receiver_name => receiver_name,
                    },
                ));
                out.push_str(&crate::template_env::render(
                    "method_return_simple.jinja",
                    minijinja::context! {
                        value => receiver_name,
                    },
                ));
            } else {
                let return_expr = go_return_expr(&method.return_type, "ptr", ffi_prefix, opaque_names);
                out.push_str(&crate::template_env::render(
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
/// `returns_value_and_error` should be true when the enclosing function returns `(*T, error)`,
/// so that error paths emit `return nil, fmt.Errorf(...)` instead of `return fmt.Errorf(...)`.
/// `can_return_error` should be true when the enclosing function has `error` in its return type.
/// When false, marshal failures are handled with `panic` since the function signature has no error return.
pub(super) fn gen_param_to_c(
    param: &ParamDef,
    returns_value_and_error: bool,
    can_return_error: bool,
    ffi_prefix: &str,
    opaque_names: &std::collections::HashSet<&str>,
) -> String {
    let mut out = String::with_capacity(512);
    // Go param names must be lowerCamelCase (no underscores), and internal C-side
    // temporaries use the same stem with acronym uppercasing applied.
    let go_param = go_param_name(&param.name);
    let c_name = go_param_name(&format!("c_{}", param.name));
    let err_return_prefix = if returns_value_and_error { "nil, " } else { "" };

    match &param.ty {
        TypeRef::String | TypeRef::Char => {
            if param.optional {
                // Optional string param (ty=String, optional=true): the Go variable holds *string.
                out.push_str(&crate::template_env::render(
                    "param_string_optional.jinja",
                    minijinja::context! {
                        c_name => &c_name,
                        go_param => &go_param,
                    },
                ));
                out.push('\n');
            } else {
                out.push_str(&crate::template_env::render(
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
                out.push_str(&crate::template_env::render(
                    "param_string_optional.jinja",
                    minijinja::context! {
                        c_name => &c_name,
                        go_param => &go_param,
                    },
                ));
                out.push('\n');
            } else {
                out.push_str(&crate::template_env::render(
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
            // Empty slices have no first element — `&slice[0]` panics. Pass a nil
            // pointer in that case; the FFI side reads zero bytes either way.
            out.push_str(&crate::template_env::render(
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
                // Opaque types are pointer wrappers — cast the raw pointer to the C type.
                let c_type = format!("{}{}", ffi_prefix.to_uppercase(), name);
                out.push_str(&crate::template_env::render(
                    "param_opaque_cast.jinja",
                    minijinja::context! {
                        c_name => &c_name,
                        go_param => &go_param,
                        c_type => &c_type,
                    },
                ));
                out.push('\n');
            } else {
                // Non-opaque Named types: marshal to JSON, create a handle via _from_json,
                // and pass that to the C function.
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
                out.push_str(&crate::template_env::render(
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
        TypeRef::Vec(_) | TypeRef::Map(_, _) => {
            // Vec and Map types are serialized as JSON strings across the FFI boundary.
            let err_action = if can_return_error {
                format!("return {err_return_prefix}fmt.Errorf(\"failed to marshal: %w\", err)")
            } else {
                "panic(fmt.Sprintf(\"failed to marshal: %v\", err))".to_string()
            };
            out.push_str(&crate::template_env::render(
                "param_vec_or_map.jinja",
                minijinja::context! {
                    c_name => &c_name,
                    go_param => &go_param,
                    err_action => &err_action,
                },
            ));
            out.push('\n');
        }
        TypeRef::Optional(inner) => {
            match inner.as_ref() {
                TypeRef::String | TypeRef::Char | TypeRef::Path => {
                    out.push_str(&crate::template_env::render(
                        "param_string_optional.jinja",
                        minijinja::context! {
                            c_name => &c_name,
                            go_param => &go_param,
                        },
                    ));
                    out.push('\n');
                }
                TypeRef::Named(name) if opaque_names.contains(name.as_str()) => {
                    // Optional opaque type: cast the raw pointer to the C type or pass nil.
                    let c_type = format!("{}{}", ffi_prefix.to_uppercase(), name);
                    out.push_str(&crate::template_env::render(
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
                    out.push_str(&crate::template_env::render(
                        "param_optional_named_inline.jinja",
                        minijinja::context! {
                            c_name => &c_name,
                            go_param => &go_param,
                        },
                    ));
                    out.push('\n');
                }
                _ => {
                    // For other optional types, just pass nil or default
                    out.push_str(&crate::template_env::render(
                        "param_optional_decl.jinja",
                        minijinja::context! {
                            c_name => &c_name,
                        },
                    ));
                    out.push('\n');
                }
            }
        }
        TypeRef::Primitive(prim) if !param.optional => {
            // Non-optional primitive: cast to the CGo type so the value can be passed directly
            // to C functions that expect C types (e.g., uintptr_t, uint32_t).
            let cgo_ty = cgo_type_for_primitive(prim);
            let go_ty = go_type(&TypeRef::Primitive(prim.clone()));
            // Special case for bool: Go bool cannot be directly cast to C.uchar.
            // Convert via conditional: if true, 1; else 0.
            if matches!(prim, alef_core::ir::PrimitiveType::Bool) {
                out.push_str(&crate::template_env::render(
                    "param_primitive_bool.jinja",
                    minijinja::context! {
                        c_name => &c_name,
                        cgo_ty => &cgo_ty,
                        go_param => &go_param,
                    },
                ));
                out.push('\n');
            } else {
                out.push_str(&crate::template_env::render(
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
            // Optional primitive: the Go param is a pointer (*T). Dereference it if non-nil,
            // otherwise pass the max-value sentinel (e.g. u64::MAX) so the FFI layer knows
            // the parameter was omitted.
            //
            // Declare the variable using the CGo type (e.g. C.uint64_t) so that CGo does
            // not reject the value when it is passed directly to the C function. Go's native
            // numeric types (uint64, uint32, …) are distinct from CGo types and cannot be
            // passed without an explicit cast — using the CGo type at declaration avoids a
            // second cast at every call-site.
            let cgo_ty = cgo_type_for_primitive(prim);
            let go_ty = go_type(&TypeRef::Primitive(prim.clone()));
            let sentinel = primitive_max_sentinel(prim);

            // Special case for bool: Go bool cannot be directly cast to C.uchar.
            if matches!(prim, alef_core::ir::PrimitiveType::Bool) {
                out.push_str(&crate::template_env::render(
                    "param_optional_primitive_bool.jinja",
                    minijinja::context! {
                        c_name => &c_name,
                        cgo_ty => &cgo_ty,
                        go_param => &go_param,
                    },
                ));
                out.push('\n');
            } else {
                out.push_str(&crate::template_env::render(
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
        _ => {
            // Primitives and other types pass through directly
        }
    }

    if !out.is_empty() {
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use alef_core::ir::{CoreWrapper, FieldDef, MethodDef, ParamDef, PrimitiveType, TypeDef, TypeRef};

    fn opaque_type(name: &str) -> TypeDef {
        TypeDef {
            name: name.to_string(),
            rust_path: String::new(),
            original_rust_path: String::new(),
            doc: String::new(),
            cfg: None,
            fields: vec![],
            is_opaque: true,
            is_clone: false,
            is_copy: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            methods: vec![],
        }
    }

    fn simple_method(name: &str, return_type: TypeRef, is_static: bool) -> MethodDef {
        MethodDef {
            name: name.to_string(),
            doc: String::new(),
            params: vec![],
            return_type,
            is_static,
            is_async: false,
            error_type: None,
            receiver: None,
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
        }
    }

    fn simple_param(name: &str, ty: TypeRef) -> ParamDef {
        ParamDef {
            name: name.to_string(),
            ty,
            optional: false,
            default: None,
            sanitized: false,
            typed_default: None,
            is_ref: false,
            is_mut: false,
            newtype_wrapper: None,
            original_type: None,
        }
    }

    fn simple_field(name: &str, ty: TypeRef) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            ty,
            optional: false,
            default: None,
            doc: String::new(),
            sanitized: false,
            is_boxed: false,
            type_rust_path: None,
            cfg: None,
            typed_default: None,
            core_wrapper: CoreWrapper::None,
            vec_inner_core_wrapper: CoreWrapper::None,
            newtype_wrapper: None,
            serde_rename: None,
        }
    }

    #[test]
    fn test_gen_method_wrapper_opaque_free_method_emits_ptr_cast() {
        let typ = opaque_type("Client");
        let method = simple_method("close", TypeRef::Unit, false);
        let opaque: std::collections::HashSet<&str> = ["Client"].into();
        let out = gen_method_wrapper(&typ, &method, "krz", &opaque);
        // The function signature may span multiple lines (method_receiver_instance + params + method_return).
        // Check for the receiver and name components rather than the full single-line form.
        assert!(
            out.contains("func (h *Client) Close("),
            "expected receiver+method in: {out}"
        );
        assert!(out.contains("unsafe.Pointer(h.ptr)"));
    }

    #[test]
    fn test_gen_param_to_c_string_param_emits_cstring() {
        let param = simple_param("name", TypeRef::String);
        let opaque: std::collections::HashSet<&str> = std::collections::HashSet::new();
        let out = gen_param_to_c(&param, false, false, "krz", &opaque);
        assert!(out.contains("C.CString("));
        assert!(out.contains("defer C.free("));
    }

    #[test]
    fn test_gen_param_to_c_primitive_u64_emits_cgo_cast() {
        let param = simple_param("count", TypeRef::Primitive(PrimitiveType::U64));
        let opaque: std::collections::HashSet<&str> = std::collections::HashSet::new();
        let out = gen_param_to_c(&param, false, false, "krz", &opaque);
        assert!(out.contains("C.uint64_t("));
    }

    #[test]
    fn test_gen_method_wrapper_non_opaque_static_emits_package_func() {
        let mut typ = opaque_type("Config");
        typ.is_opaque = false;
        typ.fields = vec![simple_field("value", TypeRef::String)];
        let method = simple_method("default_value", TypeRef::String, true);
        let opaque: std::collections::HashSet<&str> = std::collections::HashSet::new();
        let out = gen_method_wrapper(&typ, &method, "krz", &opaque);
        // Static methods become package-level functions (no receiver)
        assert!(out.contains("func Config"));
    }

    #[test]
    fn test_gen_method_wrapper_bytes_result_emits_out_params() {
        let typ = opaque_type("Renderer");
        let method = MethodDef {
            name: "render_page".to_string(),
            doc: String::new(),
            params: vec![ParamDef {
                name: "data".to_string(),
                ty: TypeRef::Bytes,
                optional: false,
                default: None,
                sanitized: false,
                typed_default: None,
                is_ref: false,
                is_mut: false,
                newtype_wrapper: None,
                original_type: None,
            }],
            return_type: TypeRef::Bytes,
            is_static: false,
            is_async: false,
            error_type: Some("KreuzbergError".to_string()),
            receiver: None,
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
        };
        let opaque: std::collections::HashSet<&str> = ["Renderer"].into();
        let out = gen_method_wrapper(&typ, &method, "krz", &opaque);
        // Return type must be ([]byte, error).
        assert!(out.contains("([]byte, error)"), "missing bytes return type in:\n{out}");
        // Must declare out-param variables.
        assert!(out.contains("var outPtr"), "missing outPtr in:\n{out}");
        assert!(out.contains("outLen"), "missing outLen in:\n{out}");
        assert!(out.contains("outCap"), "missing outCap in:\n{out}");
        // Must pass out-params to C call.
        assert!(out.contains("&outPtr"), "missing &outPtr in:\n{out}");
        // Must copy bytes via C.GoBytes.
        assert!(out.contains("C.GoBytes"), "missing C.GoBytes in:\n{out}");
        // Must free via krz_free_bytes.
        assert!(out.contains("krz_free_bytes"), "missing krz_free_bytes in:\n{out}");
    }
}
