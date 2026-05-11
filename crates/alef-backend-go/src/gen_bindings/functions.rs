use super::methods::gen_param_to_c;
use super::types::{emit_type_doc, go_return_expr};
use crate::type_map::{go_optional_type, go_type};
use alef_codegen::naming::{go_param_name, to_go_name};
use alef_core::ir::{FunctionDef, MethodDef, ParamDef, TypeRef};
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
pub(super) fn gen_function_wrapper(
    func: &FunctionDef,
    ffi_prefix: &str,
    opaque_names: &std::collections::HashSet<&str>,
    bridge_param_names: &HashSet<String>,
    bridge_type_aliases: &HashSet<String>,
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
        } else {
            format!("({}, error)", go_optional_type(&func.return_type))
        }
    } else if matches!(func.return_type, TypeRef::Unit) {
        "".to_string()
    } else {
        go_optional_type(&func.return_type).into_owned()
    };

    let func_snake = func.name.to_snake_case();
    let ffi_name = format!("C.{}_{}", ffi_prefix, func_snake);

    // All optional params (wherever they appear) are represented as pointer types in the Go
    // signature so callers can pass nil to omit them.  This is simpler and more correct than
    // the earlier variadic approach which broke when more than one trailing optional existed.
    // Bridge params (visitor handles) are stripped from the public signature and integrated
    // into the function via ConversionOptions.Visitor field instead.
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

    out.push_str(&crate::template_env::render(
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
    for param in func.params.iter() {
        if is_bridge_param(param, bridge_param_names, bridge_type_aliases) {
            continue;
        }
        out.push_str(&gen_param_to_c(
            param,
            returns_value_and_error,
            can_return_error,
            ffi_prefix,
            opaque_names,
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
        out.push_str(&crate::template_env::render(
            "bytes_result_call.jinja",
            minijinja::context! {
                c_call => &c_call,
                ffi_prefix => ffi_prefix,
            },
        ));
        out.push_str(&crate::template_env::render(
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
            out.push_str(&crate::template_env::render(
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
            out.push_str(&crate::template_env::render(
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
                    out.push_str(&crate::template_env::render(
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
                    out.push_str(&crate::template_env::render(
                        "free_type_on_error.jinja",
                        minijinja::context! {
                            ffi_prefix => ffi_prefix,
                            type_snake => &type_snake,
                        },
                    ));
                    out.push_str("\t\t}\n");
                }
                out.push_str("\t\treturn nil, err\n");
                out.push_str("\t}\n");
            }
            // Free the FFI-allocated string after unmarshaling.
            // Bytes pointers are NOT freed — they alias internal storage owned by
            // the parent handle. The unmarshalBytes helper copies the data instead.
            if matches!(
                func.return_type,
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
            // For non-opaque Named types, free the handle after JSON extraction.
            // Opaque types are NOT freed here — the caller owns them via the Go wrapper.
            if let TypeRef::Named(name) = &func.return_type {
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

            // For Named types that require JSON unmarshaling and can return errors,
            // inline the unmarshal logic to properly propagate errors.
            if can_return_error {
                if let TypeRef::Named(name) = &func.return_type {
                    if !opaque_names.contains(name.as_str()) {
                        let type_snake = name.to_snake_case();
                        out.push_str(&crate::template_env::render(
                            "c_json_to_json.jinja",
                            minijinja::context! {
                                ffi_prefix => ffi_prefix,
                                type_snake => &type_snake,
                            },
                        ));
                        out.push_str("\tif jsonPtr == nil {\n");
                        out.push_str("\t\treturn nil, fmt.Errorf(\"failed to convert to JSON\")\n");
                        out.push_str("\t}\n");
                        out.push_str(&crate::template_env::render(
                            "free_string.jinja",
                            minijinja::context! {
                                ffi_prefix => ffi_prefix,
                                ptr => "jsonPtr",
                            },
                        ));
                        out.push_str(&crate::template_env::render(
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
                        let return_expr = go_return_expr(&func.return_type, "ptr", ffi_prefix, opaque_names);
                        out.push_str(&crate::template_env::render(
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
                        out.push_str(&crate::template_env::render(
                            "free_string.jinja",
                            minijinja::context! {
                                ffi_prefix => ffi_prefix,
                                ptr => "ptr",
                            },
                        ));
                        out.push_str(&crate::template_env::render(
                            "var_decl_slice.jinja",
                            minijinja::context! {
                                element_type => &go_elem,
                            },
                        ));
                        out.push_str("\tif err := json.Unmarshal([]byte(C.GoString(ptr)), &result); err != nil {\n");
                        out.push_str("\t\treturn nil, fmt.Errorf(\"failed to unmarshal: %w\", err)\n");
                        out.push_str("\t}\n");
                        out.push_str(&crate::template_env::render(
                            "method_return_simple.jinja",
                            minijinja::context! {
                                value => "result, nil",
                            },
                        ));
                    }
                } else {
                    let return_expr = go_return_expr(&func.return_type, "ptr", ffi_prefix, opaque_names);
                    out.push_str(&crate::template_env::render(
                        "method_return_simple.jinja",
                        minijinja::context! {
                            value => format!("{}, nil", return_expr),
                        },
                    ));
                }
            } else {
                let return_expr = go_return_expr(&func.return_type, "ptr", ffi_prefix, opaque_names);
                out.push_str(&crate::template_env::render(
                    "method_return_simple.jinja",
                    minijinja::context! {
                        value => return_expr,
                    },
                ));
            }
        }
    } else if matches!(func.return_type, TypeRef::Unit) {
        out.push_str(&crate::template_env::render(
            "c_call_simple.jinja",
            minijinja::context! {
                c_call => &c_call,
            },
        ));
    } else {
        out.push_str(&crate::template_env::render(
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
            out.push_str(&crate::template_env::render(
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
        let return_expr = go_return_expr(&func.return_type, "ptr", ffi_prefix, opaque_names);
        out.push_str(&crate::template_env::render(
            "method_return_simple.jinja",
            minijinja::context! {
                value => return_expr,
            },
        ));
    }

    out.push_str(&crate::template_env::render(
        "function_body_end.jinja",
        minijinja::Value::default(),
    ));
    out
}

/// Generate a custom wrapper for the `convert` function that integrates visitor support.
///
/// When options.Visitor is not nil, the wrapper delegates to convertWithVisitorHelper.
/// Otherwise, it calls the base convert via the FFI layer (with nil visitor).
pub(super) fn gen_convert_with_visitor_wrapper(
    func: &FunctionDef,
    ffi_prefix: &str,
    opaque_names: &std::collections::HashSet<&str>,
) -> String {
    let mut out = String::with_capacity(2048);

    let func_go_name = to_go_name(&func.name);
    emit_type_doc(&mut out, &func_go_name, &func.doc, "converts HTML to Markdown.");

    // Find the html and options parameters.
    let options_param = func.params.iter().find(|p| p.name == "options");

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

    // Return type is (*ConversionResult, error) for convert
    out.push_str(&crate::template_env::render(
        "function_signature.jinja",
        minijinja::context! {
            func_name => func_go_name,
            params => &params_str,
            return_type => " (*ConversionResult, error)",
        },
    ));

    // Check if options.Visitor is set and delegate to helper
    if options_param.is_some() {
        out.push_str("\tif options != nil && options.Visitor != nil {\n");
        out.push_str("\t\treturn convertWithVisitorHelper(html, options, options.Visitor)\n");
        out.push_str("\t}\n");
        out.push('\n');
    }

    // Otherwise, call the FFI convert directly (no visitor).
    let func_snake = func.name.to_snake_case();
    let ffi_name = format!("C.{}_{}", ffi_prefix, func_snake);

    out.push_str("\tcHTML := C.CString(html)\n");
    out.push_str("\tdefer C.free(unsafe.Pointer(cHTML))\n");
    out.push('\n');

    // Handle options parameter.
    if options_param.is_some() {
        out.push_str("\tvar cOptions *C.HTMConversionOptions\n");
        out.push_str("\tif options != nil {\n");
        out.push_str("\t\tjsonBytes, err := json.Marshal(options)\n");
        out.push_str("\t\tif err != nil {\n");
        out.push_str("\t\t\treturn nil, fmt.Errorf(\"failed to marshal options: %w\", err)\n");
        out.push_str("\t\t}\n");
        out.push_str("\t\ttmpStr := C.CString(string(jsonBytes))\n");
        out.push_str(&crate::template_env::render(
            "c_options_from_json_with_name.jinja",
            minijinja::context! {
                ffi_prefix => ffi_prefix,
            },
        ));
        out.push_str("\t\tC.free(unsafe.Pointer(tmpStr))\n");
        out.push_str(&crate::template_env::render(
            "c_options_defer_free_with_name.jinja",
            minijinja::context! {
                ffi_prefix => ffi_prefix,
            },
        ));
        out.push_str("\t}\n");
        out.push('\n');

        out.push_str(&crate::template_env::render(
            "c_ptr_assign_func.jinja",
            minijinja::context! {
                ffi_name => &ffi_name,
                options_var => "cOptions",
            },
        ));
    } else {
        out.push_str(&crate::template_env::render(
            "c_ptr_assign_func.jinja",
            minijinja::context! {
                ffi_name => &ffi_name,
                options_var => "nil",
            },
        ));
    }

    out.push_str("\tif ptr == nil {\n");
    out.push_str("\t\tif err := lastError(); err != nil {\n");
    out.push_str("\t\t\treturn nil, err\n");
    out.push_str("\t\t}\n");
    out.push_str("\t\treturn nil, fmt.Errorf(\"conversion returned nil\")\n");
    out.push_str("\t}\n");
    out.push_str(&crate::template_env::render(
        "c_conversion_result_free.jinja",
        minijinja::context! {
            ffi_prefix => ffi_prefix,
        },
    ));
    out.push('\n');

    out.push_str(&crate::template_env::render(
        "c_conversion_result_to_json.jinja",
        minijinja::context! {
            ffi_prefix => ffi_prefix,
        },
    ));
    out.push_str("\tif jsonPtr == nil {\n");
    out.push_str("\t\treturn nil, fmt.Errorf(\"failed to convert result to JSON\")\n");
    out.push_str("\t}\n");
    out.push_str(&crate::template_env::render(
        "c_free_string_defer.jinja",
        minijinja::context! {
            ffi_prefix => ffi_prefix,
        },
    ));
    out.push_str("\tvar result ConversionResult\n");
    out.push_str("\tif err := json.Unmarshal([]byte(C.GoString(jsonPtr)), &result); err != nil {\n");
    out.push_str("\t\treturn nil, fmt.Errorf(\"failed to unmarshal result: %w\", err)\n");
    out.push_str("\t}\n");
    out.push_str("\treturn &result, nil\n");
    out.push_str(&crate::template_env::render(
        "function_body_end.jinja",
        minijinja::Value::default(),
    ));

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use alef_core::ir::{ParamDef, PrimitiveType, TypeRef};

    fn make_param(name: &str, ty: TypeRef) -> ParamDef {
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

    #[test]
    fn test_params_require_marshal_for_named_non_opaque() {
        let params = vec![make_param("options", TypeRef::Named("Config".to_string()))];
        let opaque: std::collections::HashSet<&str> = std::collections::HashSet::new();
        assert!(params_require_marshal(&params, &opaque));
    }

    #[test]
    fn test_params_require_marshal_false_for_opaque() {
        let params = vec![make_param("client", TypeRef::Named("Client".to_string()))];
        let opaque: std::collections::HashSet<&str> = ["Client"].into();
        assert!(!params_require_marshal(&params, &opaque));
    }

    #[test]
    fn test_is_bridge_param_matches_by_name() {
        let param = make_param("visitor", TypeRef::Named("VisitorHandle".to_string()));
        let bridge_names: HashSet<String> = ["visitor".to_string()].into();
        let aliases: HashSet<String> = HashSet::new();
        assert!(is_bridge_param(&param, &bridge_names, &aliases));
    }

    #[test]
    fn test_params_require_marshal_for_vec() {
        let params = vec![make_param(
            "items",
            TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::U32))),
        )];
        let opaque: std::collections::HashSet<&str> = std::collections::HashSet::new();
        assert!(params_require_marshal(&params, &opaque));
    }

    fn make_bytes_result_func(name: &str, with_bytes_param: bool) -> FunctionDef {
        let params = if with_bytes_param {
            vec![ParamDef {
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
            }]
        } else {
            vec![]
        };
        FunctionDef {
            name: name.to_string(),
            rust_path: String::new(),
            original_rust_path: String::new(),
            params,
            return_type: TypeRef::Bytes,
            is_async: false,
            error_type: Some("KreuzbergError".to_string()),
            doc: String::new(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
        }
    }

    fn make_bytes_result_method(name: &str) -> MethodDef {
        MethodDef {
            name: name.to_string(),
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
        }
    }

    #[test]
    fn test_is_bytes_result_func_detects_bytes_with_error() {
        let func = make_bytes_result_func("process_image", true);
        assert!(is_bytes_result_func(&func));
    }

    #[test]
    fn test_is_bytes_result_func_false_for_bytes_without_error() {
        let mut func = make_bytes_result_func("get_data", false);
        func.error_type = None;
        assert!(!is_bytes_result_func(&func));
    }

    #[test]
    fn test_is_bytes_result_func_false_for_string_with_error() {
        let func = FunctionDef {
            name: "get_text".to_string(),
            rust_path: String::new(),
            original_rust_path: String::new(),
            params: vec![],
            return_type: TypeRef::String,
            is_async: false,
            error_type: Some("KreuzbergError".to_string()),
            doc: String::new(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
        };
        assert!(!is_bytes_result_func(&func));
    }

    #[test]
    fn test_is_bytes_result_method_detects_correctly() {
        let method = make_bytes_result_method("render_page");
        assert!(is_bytes_result_method(&method));
    }

    #[test]
    fn test_gen_function_wrapper_bytes_result_emits_out_params() {
        let func = make_bytes_result_func("process_image", true);
        let opaque: std::collections::HashSet<&str> = std::collections::HashSet::new();
        let bridge_names: HashSet<String> = HashSet::new();
        let bridge_aliases: HashSet<String> = HashSet::new();
        let out = gen_function_wrapper(&func, "krz", &opaque, &bridge_names, &bridge_aliases);
        // Return type must be ([]byte, error)
        assert!(out.contains("([]byte, error)"), "missing bytes return type in:\n{out}");
        // Must declare out-param variables (outLen and outCap are declared together)
        assert!(out.contains("var outPtr"), "missing outPtr in:\n{out}");
        assert!(out.contains("outLen"), "missing outLen in:\n{out}");
        assert!(out.contains("outCap"), "missing outCap in:\n{out}");
        // Must pass out-params to C call
        assert!(out.contains("&outPtr"), "missing &outPtr in:\n{out}");
        assert!(out.contains("&outLen"), "missing &outLen in:\n{out}");
        assert!(out.contains("&outCap"), "missing &outCap in:\n{out}");
        // Must copy bytes via C.GoBytes
        assert!(out.contains("C.GoBytes"), "missing C.GoBytes in:\n{out}");
        // Must free via krz_free_bytes
        assert!(out.contains("krz_free_bytes"), "missing krz_free_bytes in:\n{out}");
    }
}
