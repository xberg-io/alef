use super::methods::gen_param_to_c;
use super::types::{emit_type_doc, go_return_expr};
use crate::backends::go::type_map::{go_optional_type, go_type};
use crate::codegen::naming::{go_param_name, pascal_to_snake, to_go_name};
use crate::core::config::TraitBridgeConfig;
use crate::core::ir::{FunctionDef, MethodDef, ParamDef, TypeRef};
use heck::ToSnakeCase;
use std::collections::HashSet;

/// Returns true if any parameter in the list requires JSON marshaling (non-opaque Named, Vec, or Map).
///
/// Such parameters use `json.Marshal` internally, which is fallible. When the surrounding
/// function has no declared `error_type`, we must still propagate the marshal error rather
/// than panicking — so we synthesize an error return in the generated signature.
pub(super) fn params_require_marshal(params: &[ParamDef], opaque_names: &std::collections::HashSet<&str>) -> bool {
    params.iter().any(|p| match &p.ty {
        TypeRef::Named(name) => !opaque_names.contains(name.as_str()),
        TypeRef::Vec(_) | TypeRef::Map(_, _) => true,
        _ => false,
    })
}

/// Returns true when `param` is a visitor bridge parameter that should be stripped from the
/// generated Go function signature and replaced with a nil argument to the C function.
pub(super) fn is_bridge_param(
    param: &ParamDef,
    bridge_param_names: &HashSet<String>,
    bridge_type_aliases: &HashSet<String>,
) -> bool {
    if bridge_param_names.contains(param.name.as_str()) {
        return true;
    }
    let type_name = match &param.ty {
        TypeRef::Named(n) => Some(n.as_str()),
        TypeRef::Optional(inner) => {
            if let TypeRef::Named(n) = inner.as_ref() {
                Some(n.as_str())
            } else {
                None
            }
        }
        _ => None,
    };
    type_name.is_some_and(|n| bridge_type_aliases.contains(n))
}

/// Returns true when the function returns `Result<Vec<u8>>` — i.e. has both an
/// `error_type` and a `TypeRef::Bytes` return.  These functions use the out-param
/// convention: `(args..., *uint8_t, *uintptr_t, *uintptr_t) -> i32`.
fn is_bytes_result_func(func: &FunctionDef) -> bool {
    func.error_type.is_some() && matches!(func.return_type, TypeRef::Bytes)
}

/// Same check for MethodDef — needed by methods.rs.
pub(super) fn is_bytes_result_method(method: &MethodDef) -> bool {
    method.error_type.is_some() && matches!(method.return_type, TypeRef::Bytes)
}

/// Generate a wrapper function for a free function.
#[allow(clippy::too_many_arguments)]
pub(super) fn gen_function_wrapper(
    func: &FunctionDef,
    ffi_prefix: &str,
    opaque_names: &std::collections::HashSet<&str>,
    bridge_param_names: &HashSet<String>,
    bridge_type_aliases: &HashSet<String>,
    value_only_types: &std::collections::HashSet<String>,
    enum_names: &std::collections::HashSet<String>,
    ffi_param_enum_names: &std::collections::HashSet<String>,
) -> String {
    let mut out = String::with_capacity(2048);

    let func_go_name = to_go_name(&func.name);

    emit_type_doc(&mut out, &func_go_name, &func.doc, "calls the FFI function.");

    // Detect Result<Vec<u8>> — uses out-param convention, always returns ([]byte, error).
    let is_bytes_result = is_bytes_result_func(func);

    // A function that marshals parameters to JSON can fail even without a declared error_type.
    // Synthesize an error return in those cases so we never panic on marshal failure.
    // Exclude bridge params — they are not marshalled (they're passed as nil).
    let non_bridge_params: Vec<_> = func
        .params
        .iter()
        .filter(|p| !is_bridge_param(p, bridge_param_names, bridge_type_aliases))
        .cloned()
        .collect();
    let marshals_params = params_require_marshal(&non_bridge_params, opaque_names);
    let can_return_error = func.error_type.is_some() || marshals_params;

    let return_type = if is_bytes_result {
        // Out-param bytes result always returns ([]byte, error)
        "([]byte, error)".to_string()
    } else if can_return_error {
        if matches!(func.return_type, TypeRef::Unit) {
            "error".to_string()
        } else if matches!(
            func.return_type,
            TypeRef::Primitive(_) | TypeRef::Duration | TypeRef::String | TypeRef::Char | TypeRef::Path
        ) {
            // Scalar value types (primitives and strings) use value-form `(T, error)`.
            // `go_return_expr` emits `ptr != 0` for bool, `uint(ptr)` for numeric primitives, and
            // `C.GoString(ptr)` for strings — all producing bare values, not pointers.
            format!("({}, error)", go_type(&func.return_type))
        } else {
            format!("({}, error)", go_optional_type(&func.return_type))
        }
    } else if matches!(func.return_type, TypeRef::Unit) {
        "".to_string()
    } else if matches!(
        func.return_type,
        TypeRef::Primitive(_) | TypeRef::Duration | TypeRef::String | TypeRef::Char | TypeRef::Path
    ) {
        // Non-error case: scalar value types use bare form because `go_return_expr`
        // produces bare values for all scalar types.
        go_type(&func.return_type).into_owned()
    } else {
        go_optional_type(&func.return_type).into_owned()
    };

    let func_snake = func.name.to_snake_case();
    let ffi_name = format!("C.{}_{}", ffi_prefix, func_snake);

    // All optional params (wherever they appear) are represented as pointer types in the Go
    // signature so callers can pass nil to omit them.  This is simpler and more correct than
    // the earlier variadic approach which broke when more than one trailing optional existed.
    // Bridge params (visitor handles) are stripped from the public signature and integrated
    // into the function via the configured options field instead.
    let mut param_strs: Vec<String> = Vec::new();
    for p in func.params.iter() {
        if is_bridge_param(p, bridge_param_names, bridge_type_aliases) {
            continue;
        }
        let param_type: String = if p.optional {
            go_optional_type(&p.ty).into_owned()
        } else if let TypeRef::Named(name) = &p.ty {
            if opaque_names.contains(name.as_str()) {
                // Opaque types are pointer wrappers — accept as pointer
                format!("*{}", go_type(&p.ty))
            } else {
                go_type(&p.ty).into_owned()
            }
        } else {
            go_type(&p.ty).into_owned()
        };
        param_strs.push(format!("{} {}", go_param_name(&p.name), param_type));
    }
    let params_str = param_strs.join(", ");
    let ret_type_str = if return_type.is_empty() {
        "".to_string()
    } else {
        format!(" {}", return_type)
    };

    out.push_str(&crate::backends::go::template_env::render(
        "function_signature.jinja",
        minijinja::context! {
            func_name => func_go_name,
            params => &params_str,
            return_type => &ret_type_str,
        },
    ));

    // Convert parameters
    // Note: can_return_error is set above (includes synthesized error for marshal-requiring params).
    let returns_value_and_error = can_return_error && !matches!(func.return_type, TypeRef::Unit);
    let param_err_return_prefix: String = if returns_value_and_error {
        format!("{}, ", crate::backends::go::type_map::go_zero_value(&func.return_type))
    } else {
        String::new()
    };
    for param in func.params.iter() {
        if is_bridge_param(param, bridge_param_names, bridge_type_aliases) {
            continue;
        }
        out.push_str(&gen_param_to_c(
            param,
            &param_err_return_prefix,
            can_return_error,
            ffi_prefix,
            opaque_names,
            enum_names,
            ffi_param_enum_names,
        ));
    }

    // Build the C call with converted parameters.
    // Bridge params that are sanitized (unknown type in IR) are omitted from the C call — the
    // FFI backend strips them from the generated C function signature entirely and handles the
    // visitor path via a separate {prefix}_convert_with_visitor function.
    // Non-sanitized bridge params pass nil (no visitor) in the plain Convert().
    // Bytes params expand to two C arguments: the pointer and the length.
    // For bytes-result functions, three trailing out-params (&outPtr, &outLen, &outCap) are appended.
    let c_params: Vec<String> = func
        .params
        .iter()
        .flat_map(|p| -> Vec<String> {
            if is_bridge_param(p, bridge_param_names, bridge_type_aliases) {
                // Sanitized bridge params have been removed from the C function signature;
                // do not emit a nil slot for them.
                if p.sanitized { vec![] } else { vec!["nil".to_string()] }
            } else {
                let c_name = go_param_name(&format!("c_{}", p.name));
                if matches!(p.ty, TypeRef::Bytes) {
                    vec![c_name.clone(), format!("{}Len", c_name)]
                } else {
                    vec![c_name]
                }
            }
        })
        .collect();

    // For bytes-result, append the three out-param addresses.
    let c_call = if is_bytes_result {
        let mut all_params = c_params.clone();
        all_params.push("&outPtr".to_string());
        all_params.push("&outLen".to_string());
        all_params.push("&outCap".to_string());
        format!("{}({})", ffi_name, all_params.join(", "))
    } else {
        format!("{}({})", ffi_name, c_params.join(", "))
    };

    // Handle result and error.
    // Result<Vec<u8>> uses the out-param convention: emit bytes_result_call which
    // declares outPtr/outLen/outCap, calls the FFI with those addresses appended,
    // checks the i32 return code, copies the bytes, and frees via {prefix}_free_bytes.
    if is_bytes_result {
        out.push_str(&crate::backends::go::template_env::render(
            "bytes_result_call.jinja",
            minijinja::context! {
                c_call => &c_call,
                ffi_prefix => ffi_prefix,
            },
        ));
        out.push_str(&crate::backends::go::template_env::render(
            "function_body_end.jinja",
            minijinja::Value::default(),
        ));
        return out;
    }

    // When can_return_error is true (either from declared error_type or synthesized for
    // marshal-requiring params), emit lastError() checks. For synthesized-error functions
    // that have no declared error_type, the FFI call itself never sets a last error, so
    // lastError() will return nil and the return value flows through normally.
    if can_return_error {
        if matches!(func.return_type, TypeRef::Unit) {
            out.push_str(&crate::backends::go::template_env::render(
                "c_call_simple.jinja",
                minijinja::context! {
                    c_call => &c_call,
                },
            ));
            if func.error_type.is_some() {
                out.push_str("\treturn lastError()\n");
            } else {
                out.push_str("\treturn nil\n");
            }
        } else {
            out.push_str(&crate::backends::go::template_env::render(
                "c_ptr_assign.jinja",
                minijinja::context! {
                    c_call => &c_call,
                },
            ));
            if func.error_type.is_some() {
                out.push_str("\tif err := lastError(); err != nil {\n");
                // Free the pointer if non-nil even on error, to avoid leaks.
                // Bytes pointers are NOT freed — they alias internal storage.
                if matches!(
                    func.return_type,
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
                if let TypeRef::Named(name) = &func.return_type {
                    let type_snake = name.to_snake_case();
                    out.push_str("\t\tif ptr != nil {\n");
                    out.push_str(&crate::backends::go::template_env::render(
                        "free_type_on_error.jinja",
                        minijinja::context! {
                            ffi_prefix => ffi_prefix,
                            type_snake => &type_snake,
                        },
                    ));
                    out.push_str("\t\t}\n");
                }
                // Use the type-appropriate zero value: `nil` for pointer/slice/Named returns,
                // `0`/`false`/`""` for scalar Primitive/Duration value-form returns.
                let zero_value = crate::backends::go::type_map::go_zero_value(&func.return_type);
                out.push_str(&crate::backends::go::template_env::render(
                    "return_zero_err.jinja",
                    minijinja::context! {
                        zero_value => &zero_value,
                    },
                ));
                out.push_str("\t}\n");
            }
            // Free the FFI-allocated string after unmarshaling.
            // Bytes pointers are NOT freed — they alias internal storage owned by
            // the parent handle. The unmarshalBytes helper copies the data instead.
            if matches!(
                func.return_type,
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
            // For non-opaque Named types, free the handle after JSON extraction.
            // Opaque types are NOT freed here — the caller owns them via the Go wrapper.
            if let TypeRef::Named(name) = &func.return_type {
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

            // For Named types that require JSON unmarshaling and can return errors,
            // inline the unmarshal logic to properly propagate errors.
            if can_return_error {
                if let TypeRef::Named(name) = &func.return_type {
                    if !opaque_names.contains(name.as_str()) {
                        let type_snake = name.to_snake_case();
                        out.push_str(&crate::backends::go::template_env::render(
                            "c_json_to_json.jinja",
                            minijinja::context! {
                                ffi_prefix => ffi_prefix,
                                type_snake => &type_snake,
                            },
                        ));
                        out.push_str("\tif jsonPtr == nil {\n");
                        out.push_str("\t\treturn nil, fmt.Errorf(\"failed to convert to JSON\")\n");
                        out.push_str("\t}\n");
                        out.push_str(&crate::backends::go::template_env::render(
                            "free_string.jinja",
                            minijinja::context! {
                                ffi_prefix => ffi_prefix,
                                ptr => "jsonPtr",
                            },
                        ));
                        out.push_str(&crate::backends::go::template_env::render(
                            "var_decl_type.jinja",
                            minijinja::context! {
                                type_name => name.as_str(),
                            },
                        ));
                        out.push_str(
                            "\tif err := json.Unmarshal([]byte(C.GoString(jsonPtr)), &result); err != nil {\n",
                        );
                        out.push_str("\t\treturn nil, fmt.Errorf(\"failed to unmarshal: %w\", err)\n");
                        out.push_str("\t}\n");
                        out.push_str("\treturn &result, nil\n");
                    } else {
                        let return_expr =
                            go_return_expr(&func.return_type, "ptr", ffi_prefix, opaque_names, value_only_types);
                        out.push_str(&crate::backends::go::template_env::render(
                            "method_return_simple.jinja",
                            minijinja::context! {
                                value => format!("{}, nil", return_expr),
                            },
                        ));
                    }
                } else if matches!(func.return_type, TypeRef::Vec(_)) {
                    // Handle Vec types with error propagation
                    if let TypeRef::Vec(inner) = &func.return_type {
                        let go_elem = go_type(inner);
                        out.push_str("\tif ptr == nil {\n");
                        out.push_str("\t\treturn nil, fmt.Errorf(\"failed to get result\")\n");
                        out.push_str("\t}\n");
                        out.push_str(&crate::backends::go::template_env::render(
                            "free_string.jinja",
                            minijinja::context! {
                                ffi_prefix => ffi_prefix,
                                ptr => "ptr",
                            },
                        ));
                        out.push_str(&crate::backends::go::template_env::render(
                            "var_decl_slice.jinja",
                            minijinja::context! {
                                element_type => &go_elem,
                            },
                        ));
                        out.push_str("\tif err := json.Unmarshal([]byte(C.GoString(ptr)), &result); err != nil {\n");
                        out.push_str("\t\treturn nil, fmt.Errorf(\"failed to unmarshal: %w\", err)\n");
                        out.push_str("\t}\n");
                        out.push_str(&crate::backends::go::template_env::render(
                            "method_return_simple.jinja",
                            minijinja::context! {
                                value => "result, nil",
                            },
                        ));
                    }
                } else {
                    let return_expr =
                        go_return_expr(&func.return_type, "ptr", ffi_prefix, opaque_names, value_only_types);
                    out.push_str(&crate::backends::go::template_env::render(
                        "method_return_simple.jinja",
                        minijinja::context! {
                            value => format!("{}, nil", return_expr),
                        },
                    ));
                }
            } else {
                let return_expr = go_return_expr(&func.return_type, "ptr", ffi_prefix, opaque_names, value_only_types);
                out.push_str(&crate::backends::go::template_env::render(
                    "method_return_simple.jinja",
                    minijinja::context! {
                        value => return_expr,
                    },
                ));
            }
        }
    } else if matches!(func.return_type, TypeRef::Unit) {
        out.push_str(&crate::backends::go::template_env::render(
            "c_call_simple.jinja",
            minijinja::context! {
                c_call => &c_call,
            },
        ));
    } else {
        out.push_str(&crate::backends::go::template_env::render(
            "c_ptr_assign.jinja",
            minijinja::context! {
                c_call => &c_call,
            },
        ));
        // Add defer free for C string returns.
        // Bytes pointers are NOT freed — they alias internal storage.
        if matches!(
            func.return_type,
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
        // For non-opaque Named types, free the handle after JSON extraction.
        // Opaque types are NOT freed here — the caller owns them via the Go wrapper.
        if let TypeRef::Named(name) = &func.return_type {
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
        let return_expr = go_return_expr(&func.return_type, "ptr", ffi_prefix, opaque_names, value_only_types);
        out.push_str(&crate::backends::go::template_env::render(
            "method_return_simple.jinja",
            minijinja::context! {
                value => return_expr,
            },
        ));
    }

    out.push_str(&crate::backends::go::template_env::render(
        "function_body_end.jinja",
        minijinja::Value::default(),
    ));
    out
}

/// Generate a Go wrapper for a function returning a host-native capsule (Language) type.
///
/// The exported C symbol returns the host runtime's raw grammar pointer
/// (`*C.TSLanguage`). The wrapper converts parameters, calls the C function, and
/// constructs the host `Language` (e.g. `tree_sitter.NewLanguage(unsafe.Pointer(ptr))`)
/// from the raw pointer — never an opaque alef handle.
pub(super) fn gen_capsule_function_wrapper(
    func: &FunctionDef,
    ffi_prefix: &str,
    opaque_names: &std::collections::HashSet<&str>,
    enum_names: &std::collections::HashSet<String>,
    ffi_param_enum_names: &std::collections::HashSet<String>,
    cfg: &crate::core::config::HostCapsuleTypeConfig,
) -> String {
    let mut out = String::with_capacity(1024);

    let func_go_name = to_go_name(&func.name);
    emit_type_doc(&mut out, &func_go_name, &func.doc, "calls the FFI function.");

    // Parameter list (capsule functions take only plain scalar/string params in practice).
    let mut param_strs: Vec<String> = Vec::new();
    for p in func.params.iter() {
        let param_type = if p.optional {
            go_optional_type(&p.ty).into_owned()
        } else {
            go_type(&p.ty).into_owned()
        };
        param_strs.push(format!("{} {}", go_param_name(&p.name), param_type));
    }
    let params_str = param_strs.join(", ");
    // A fallible capsule function (Rust `Result`, e.g. `get_language`) surfaces the
    // failure as a Go `error` — matching python (raises) / node (throws) and the
    // e2e fixtures that assert an error for unknown/empty input. An infallible one
    // returns the bare host type (nil = not found).
    let is_fallible = func.error_type.is_some();
    let ret_type_str = if is_fallible {
        format!(" ({}, error)", cfg.host_type)
    } else {
        format!(" {}", cfg.host_type)
    };

    out.push_str(&crate::backends::go::template_env::render(
        "function_signature.jinja",
        minijinja::context! {
            func_name => func_go_name,
            params => &params_str,
            return_type => &ret_type_str,
        },
    ));

    // Convert parameters to C. The conversion-failure prefix returns the zero value:
    // `nil, ` for the fallible `(T, error)` form, bare `nil` for the infallible form.
    let conv_fail_prefix = if is_fallible { "nil, " } else { "nil" };
    for param in func.params.iter() {
        out.push_str(&gen_param_to_c(
            param,
            conv_fail_prefix,
            false,
            ffi_prefix,
            opaque_names,
            enum_names,
            ffi_param_enum_names,
        ));
    }

    let func_snake = func.name.to_snake_case();
    let ffi_name = format!("C.{}_{}", ffi_prefix, func_snake);
    let c_params: Vec<String> = func
        .params
        .iter()
        .map(|p| go_param_name(&format!("c_{}", p.name)))
        .collect();
    let c_call = format!("{}({})", ffi_name, c_params.join(", "));

    // Call the C function, guard nil, then construct the host Language from the raw pointer.
    // The `{ptr}` placeholder receives the raw cgo pointer; the default wraps it via unsafe.Pointer.
    let construct = cfg.construct("cLang", "tree_sitter.NewLanguage(unsafe.Pointer({ptr}))");
    out.push_str(&format!("\tcLang := {c_call}\n"));
    if is_fallible {
        // The FFI sets last_error and returns null on failure; surface it as a Go error
        // (mirrors the registry's fallible `GetLanguage`, which checks `lastError()`).
        out.push_str("\tif err := lastError(); err != nil {\n\t\treturn nil, err\n\t}\n");
        out.push_str(&format!("\treturn {construct}, nil\n"));
    } else {
        out.push_str("\tif cLang == nil {\n\t\treturn nil\n\t}\n");
        out.push_str(&format!("\treturn {construct}\n"));
    }

    out.push_str(&crate::backends::go::template_env::render(
        "function_body_end.jinja",
        minijinja::Value::default(),
    ));
    out
}

/// Generate a custom wrapper for an options-field visitor bridge function.
///
/// When the configured options field is not nil, the wrapper delegates to the visitor helper.
/// Otherwise, it calls the base FFI function without attaching a visitor bridge.
pub(super) fn gen_convert_with_visitor_wrapper(
    func: &FunctionDef,
    ffi_prefix: &str,
    opaque_names: &std::collections::HashSet<&str>,
    _value_only_types: &std::collections::HashSet<String>,
    bridge_cfg: &TraitBridgeConfig,
) -> String {
    let mut out = String::with_capacity(2048);

    let func_go_name = to_go_name(&func.name);
    emit_type_doc(&mut out, &func_go_name, &func.doc, "runs the generated conversion.");

    let options_type = bridge_cfg
        .options_type
        .as_deref()
        .expect("go options-field bridge requires options_type");
    let options_field = bridge_cfg
        .resolved_options_field()
        .expect("go options-field bridge requires options_field or param_name");
    let options_param = func
        .params
        .iter()
        .find(|p| type_ref_named_type(&p.ty) == Some(options_type));
    let options_go_name = options_param.map(|p| go_param_name(&p.name));
    let options_field_go = to_go_name(options_field);
    let helper_name = format!("{}WithVisitorHelper", func.name.to_snake_case());
    let return_type_name = named_return_type(&func.return_type)
        .expect("go options-field visitor wrapper currently requires a named return type");
    let return_go_type = go_optional_type(&func.return_type).into_owned();
    let return_type_str = format!(" ({return_go_type}, error)");
    let options_c_type = format!("{}{}", ffi_prefix.to_uppercase(), options_type);
    let options_type_snake = pascal_to_snake(options_type);
    let return_type_snake = pascal_to_snake(return_type_name);

    let mut param_strs: Vec<String> = Vec::new();
    for p in &func.params {
        let param_type = if p.optional {
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
        param_strs.push(format!("{} {}", go_param_name(&p.name), param_type));
    }
    let params_str = param_strs.join(", ");

    out.push_str(&crate::backends::go::template_env::render(
        "function_signature.jinja",
        minijinja::context! {
            func_name => func_go_name,
            params => &params_str,
            return_type => &return_type_str,
        },
    ));

    if let Some(options_var) = options_go_name.as_deref() {
        let helper_args = func
            .params
            .iter()
            .map(|p| go_param_name(&p.name))
            .chain(std::iter::once(format!("{options_var}.{options_field_go}")))
            .collect::<Vec<_>>()
            .join(", ");
        out.push_str(&crate::backends::go::template_env::render(
            "visitor_helper_guard.jinja",
            minijinja::context! {
                options_var => options_var,
                options_field_go => &options_field_go,
                helper_name => &helper_name,
                helper_args => &helper_args,
            },
        ));
    }

    // Otherwise, call the FFI function directly without visitor support.
    let func_snake = func.name.to_snake_case();
    let ffi_name = format!("C.{}_{}", ffi_prefix, func_snake);

    let mut c_args = Vec::new();
    for param in &func.params {
        if type_ref_named_type(&param.ty) == Some(options_type) {
            c_args.push("cOptions".to_string());
            continue;
        }
        let go_name = go_param_name(&param.name);
        let c_name = go_param_name(&format!("c_{}", param.name));
        if matches!(param.ty, TypeRef::String | TypeRef::Path) {
            out.push_str(&crate::backends::go::template_env::render(
                "c_string_arg_setup.jinja",
                minijinja::context! {
                    c_name => &c_name,
                    go_name => &go_name,
                },
            ));
            c_args.push(c_name);
        } else {
            c_args.push(go_name);
        }
    }

    if options_param.is_some() {
        let options_var = options_go_name.as_deref().expect("checked above");
        let from_json_fn = format!("{ffi_prefix}_{options_type_snake}_from_json");
        let free_fn = format!("{ffi_prefix}_{options_type_snake}_free");
        let options_description = options_type_snake.replace('_', " ");
        out.push_str(&crate::backends::go::template_env::render(
            "options_json_to_c.jinja",
            minijinja::context! {
                options_c_type => &options_c_type,
                options_var => options_var,
                from_json_fn => &from_json_fn,
                free_fn => &free_fn,
                options_description => &options_description,
            },
        ));

        out.push_str(&crate::backends::go::template_env::render(
            "ffi_ptr_call.jinja",
            minijinja::context! {
                ffi_name => &ffi_name,
                c_args => c_args.join(", "),
            },
        ));
    } else {
        out.push_str(&crate::backends::go::template_env::render(
            "ffi_ptr_call.jinja",
            minijinja::context! {
                ffi_name => &ffi_name,
                c_args => c_args.join(", "),
            },
        ));
    }

    out.push_str("\tif ptr == nil {\n");
    out.push_str("\t\tif err := lastError(); err != nil {\n");
    out.push_str("\t\t\treturn nil, err\n");
    out.push_str("\t\t}\n");
    out.push_str("\t\treturn nil, fmt.Errorf(\"conversion returned nil\")\n");
    out.push_str("\t}\n");
    out.push_str(&crate::backends::go::template_env::render(
        "free_type.jinja",
        minijinja::context! {
            ffi_prefix => ffi_prefix,
            type_snake => &return_type_snake,
            ptr => "ptr",
        },
    ));
    out.push('\n');

    let to_json_fn = format!("{ffi_prefix}_{return_type_snake}_to_json");
    out.push_str(&crate::backends::go::template_env::render(
        "result_json_unmarshal.jinja",
        minijinja::context! {
            to_json_fn => &to_json_fn,
            use_prefix_free_string => true,
            ffi_prefix => ffi_prefix,
            return_type_name => return_type_name,
        },
    ));
    out.push_str(&crate::backends::go::template_env::render(
        "function_body_end.jinja",
        minijinja::Value::default(),
    ));

    out
}

fn type_ref_named_type(ty: &TypeRef) -> Option<&str> {
    match ty {
        TypeRef::Named(name) => Some(name.as_str()),
        TypeRef::Optional(inner) => type_ref_named_type(inner),
        _ => None,
    }
}

fn named_return_type(ty: &TypeRef) -> Option<&str> {
    type_ref_named_type(ty)
}

/// Emit a module-level wrapper function for a streaming adapter.
/// This allows tests/consumers to call pkg.CrawlStream(engine, url) instead of engine.CrawlStream(url).
/// For adapters with a request_type, decompose the first field into primitive parameters for ergonomics.
pub(super) fn gen_adapter_wrapper(
    adapter: &crate::core::config::AdapterConfig,
    _pkg_name: &str,
    types: &[crate::core::ir::TypeDef],
) -> String {
    let adapter_name = &adapter.name;
    let go_func_name = to_go_name(adapter_name);
    let owner_type = adapter.owner_type.as_deref().unwrap_or_else(|| {
        panic!(
            "go adapter `{adapter_name}`: streaming adapter requires `owner_type` in `[[adapters]]` config (the Rust handle type that owns the streaming method)"
        )
    });
    let item_type = adapter.item_type.as_deref().unwrap_or_else(|| {
        panic!(
            "go adapter `{adapter_name}`: streaming adapter requires `item_type` in `[[adapters]]` config (the Rust item type yielded by the stream)"
        )
    });
    let item_type_simple = item_type.rsplit("::").next().unwrap_or(item_type);

    // Extract request type and simplify (remove Rust path prefix)
    let request_type = adapter.request_type.as_deref().unwrap_or_else(|| {
        panic!(
            "go adapter `{adapter_name}`: streaming adapter requires `request_type` in `[[adapters]]` config (the Rust request payload type)"
        )
    });
    let request_type_simple = request_type.rsplit("::").next().unwrap_or(request_type);

    // Decompose request struct into primitives for ergonomic wrapper.
    // E.g. CrawlStreamRequest { url: String } → accept (url string), construct req, call method.
    let (param_parts, request_construction) = if adapter.request_type.is_some() && adapter.params.len() == 1 {
        // Single request param: decompose by inspecting the request type's first field in IR.
        let param = &adapter.params[0];
        let param_ty_name = &param.ty;
        let ir_type = types.iter().find(|t| &t.name == param_ty_name);

        if let Some(ty_def) = ir_type {
            if let Some(first_field) = ty_def.fields.first() {
                let field_name = &first_field.name;
                let field_name_go = to_go_name(field_name);

                // Determine Go parameter type based on field type
                let go_field_type = match &first_field.ty {
                    TypeRef::String => "string".to_string(),
                    TypeRef::Vec(inner) if matches!(**inner, TypeRef::String) => "[]string".to_string(),
                    TypeRef::Vec(_) => "[]interface{}".to_string(),
                    other => {
                        // Fallback to type mapping
                        crate::backends::go::type_map::go_type(other).into_owned()
                    }
                };

                let wrapper_params = vec![
                    format!("engine *{owner_type}"),
                    format!("{field_name_go} {go_field_type}"),
                ];

                // Construct request struct: req := &CrawlStreamRequest{Url: url}
                let struct_field_name = to_go_name(field_name);
                let construction = format!("req := &{request_type_simple}{{{struct_field_name}: {field_name_go}}}\n\t");

                (wrapper_params, Some(construction))
            } else {
                // Type has no fields; fall back to original behavior
                let mut params = vec![format!("engine *{owner_type}")];
                for p in &adapter.params {
                    let go_param_type = match p.ty.as_str() {
                        "String" => "string".to_string(),
                        ty => {
                            // Strip Rust path prefix (e.g., "crate::requests::CrawlStreamRequest" → "CrawlStreamRequest")
                            ty.rsplit("::").next().unwrap_or(ty).to_string()
                        }
                    };
                    let param_name = go_param_name(&p.name);
                    params.push(format!("{param_name} {go_param_type}"));
                }
                (params, None)
            }
        } else {
            // Type not found in IR; fall back to original behavior
            let mut params = vec![format!("engine *{owner_type}")];
            for p in &adapter.params {
                let go_param_type = match p.ty.as_str() {
                    "String" => "string".to_string(),
                    ty => {
                        // Strip Rust path prefix (e.g., "crate::requests::CrawlStreamRequest" → "CrawlStreamRequest")
                        ty.rsplit("::").next().unwrap_or(ty).to_string()
                    }
                };
                let param_name = go_param_name(&p.name);
                params.push(format!("{param_name} {go_param_type}"));
            }
            (params, None)
        }
    } else {
        // Multi-param or no request_type: use original behavior
        let mut params = vec![format!("engine *{owner_type}")];
        for p in &adapter.params {
            let go_param_type = match p.ty.as_str() {
                "String" => "string".to_string(),
                ty => {
                    // Strip Rust path prefix (e.g., "crate::requests::CrawlStreamRequest" → "CrawlStreamRequest")
                    ty.rsplit("::").next().unwrap_or(ty).to_string()
                }
            };
            let param_name = go_param_name(&p.name);
            params.push(format!("{param_name} {go_param_type}"));
        }
        (params, None)
    };

    // Return type: (channel of items, error)
    let return_type = format!("<-chan {item_type_simple}, error");

    // Build method call: engine.CrawlStream(*req) or engine.CrawlStream(...params)
    let method_call_name = to_go_name(adapter_name);
    let method_call = if request_construction.is_some() {
        // If we constructed a request, dereference the pointer for the method call
        format!("engine.{}(*req)", method_call_name)
    } else {
        // Otherwise, pass the original parameters
        let param_args = adapter
            .params
            .iter()
            .map(|p| go_param_name(&p.name))
            .collect::<Vec<_>>()
            .join(", ");
        if param_args.is_empty() {
            format!("engine.{}()", method_call_name)
        } else {
            format!("engine.{}({})", method_call_name, param_args)
        }
    };

    crate::backends::go::template_env::render(
        "adapter_wrapper.jinja",
        minijinja::context! {
            go_func_name => &go_func_name,
            owner_type => owner_type,
            method_call_name => &method_call_name,
            params => param_parts.join(", "),
            return_type => &return_type,
            request_construction => request_construction.as_deref(),
            method_call => &method_call,
        },
    )
}

#[cfg(test)]
mod tests;
