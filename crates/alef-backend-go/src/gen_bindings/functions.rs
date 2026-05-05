use super::methods::gen_param_to_c;
use super::types::{emit_type_doc, go_return_expr};
use crate::type_map::{go_optional_type, go_type};
use alef_codegen::naming::{go_param_name, to_go_name};
use alef_core::ir::{FunctionDef, ParamDef, TypeRef};
use heck::ToSnakeCase;
use std::collections::HashSet;
use std::fmt::Write;

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

    let return_type = if can_return_error {
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

    write!(out, "func {}(", func_go_name).ok();

    // All optional params (wherever they appear) are represented as pointer types in the Go
    // signature so callers can pass nil to omit them.  This is simpler and more correct than
    // the earlier variadic approach which broke when more than one trailing optional existed.
    // Bridge params (visitor handles) are stripped from the public signature — ConvertWithVisitor
    // provides the visitor-accepting variant separately.
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
    write!(out, "{}", param_strs.join(", ")).ok();

    if return_type.is_empty() {
        writeln!(out, ") {{").ok();
    } else {
        writeln!(out, ") {} {{", return_type).ok();
    }

    // Convert parameters
    // Note: can_return_error is set above (includes synthesized error for marshal-requiring params).
    let returns_value_and_error = can_return_error && !matches!(func.return_type, TypeRef::Unit);
    for param in func.params.iter() {
        if is_bridge_param(param, bridge_param_names, bridge_type_aliases) {
            continue;
        }
        write!(
            out,
            "{}",
            gen_param_to_c(
                param,
                returns_value_and_error,
                can_return_error,
                ffi_prefix,
                opaque_names
            )
        )
        .ok();
    }

    // Build the C call with converted parameters.
    // Bridge params that are sanitized (unknown type in IR) are omitted from the C call — the
    // FFI backend strips them from the generated C function signature entirely and handles the
    // visitor path via a separate {prefix}_convert_with_visitor function.
    // Non-sanitized bridge params pass nil (no visitor) in the plain Convert().
    // Bytes params expand to two C arguments: the pointer and the length.
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

    let c_call = format!("{}({})", ffi_name, c_params.join(", "));

    // Handle result and error.
    // When can_return_error is true (either from declared error_type or synthesized for
    // marshal-requiring params), emit lastError() checks. For synthesized-error functions
    // that have no declared error_type, the FFI call itself never sets a last error, so
    // lastError() will return nil and the return value flows through normally.
    if can_return_error {
        if matches!(func.return_type, TypeRef::Unit) {
            writeln!(out, "\t{}", c_call).ok();
            if func.error_type.is_some() {
                writeln!(out, "\treturn lastError()").ok();
            } else {
                writeln!(out, "\treturn nil").ok();
            }
        } else {
            writeln!(out, "\tptr := {}", c_call).ok();
            if func.error_type.is_some() {
                writeln!(out, "\tif err := lastError(); err != nil {{").ok();
                // Free the pointer if non-nil even on error, to avoid leaks.
                // Bytes pointers are NOT freed — they alias internal storage.
                if matches!(
                    func.return_type,
                    TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json
                ) {
                    writeln!(out, "\t\tif ptr != nil {{").ok();
                    writeln!(out, "\t\t\tC.{}_free_string(ptr)", ffi_prefix).ok();
                    writeln!(out, "\t\t}}").ok();
                }
                if let TypeRef::Named(name) = &func.return_type {
                    let type_snake = name.to_snake_case();
                    writeln!(out, "\t\tif ptr != nil {{").ok();
                    writeln!(out, "\t\t\tC.{}_{}_free(ptr)", ffi_prefix, type_snake).ok();
                    writeln!(out, "\t\t}}").ok();
                }
                writeln!(out, "\t\treturn nil, err").ok();
                writeln!(out, "\t}}").ok();
            }
            // Free the FFI-allocated string after unmarshaling.
            // Bytes pointers are NOT freed — they alias internal storage owned by
            // the parent handle. The unmarshalBytes helper copies the data instead.
            if matches!(
                func.return_type,
                TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json
            ) {
                writeln!(out, "\tdefer C.{}_free_string(ptr)", ffi_prefix).ok();
            }
            // For non-opaque Named types, free the handle after JSON extraction.
            // Opaque types are NOT freed here — the caller owns them via the Go wrapper.
            if let TypeRef::Named(name) = &func.return_type {
                if !opaque_names.contains(name.as_str()) {
                    let type_snake = name.to_snake_case();
                    writeln!(out, "\tdefer C.{}_{}_free(ptr)", ffi_prefix, type_snake).ok();
                }
            }

            // For Named types that require JSON unmarshaling and can return errors,
            // inline the unmarshal logic to properly propagate errors.
            if can_return_error {
                if let TypeRef::Named(name) = &func.return_type {
                    if !opaque_names.contains(name.as_str()) {
                        let type_snake = name.to_snake_case();
                        writeln!(out, "\tjsonPtr := C.{}_{}_to_json(ptr)", ffi_prefix, type_snake).ok();
                        writeln!(out, "\tif jsonPtr == nil {{").ok();
                        writeln!(out, "\t\treturn nil, fmt.Errorf(\"failed to convert to JSON\")").ok();
                        writeln!(out, "\t}}").ok();
                        writeln!(out, "\tdefer C.{}_free_string(jsonPtr)", ffi_prefix).ok();
                        writeln!(out, "\tvar result {}", name).ok();
                        writeln!(
                            out,
                            "\tif err := json.Unmarshal([]byte(C.GoString(jsonPtr)), &result); err != nil {{"
                        )
                        .ok();
                        writeln!(out, "\t\treturn nil, fmt.Errorf(\"failed to unmarshal: %w\", err)").ok();
                        writeln!(out, "\t}}").ok();
                        writeln!(out, "\treturn &result, nil").ok();
                    } else {
                        writeln!(
                            out,
                            "\treturn {}, nil",
                            go_return_expr(&func.return_type, "ptr", ffi_prefix, opaque_names)
                        )
                        .ok();
                    }
                } else if matches!(func.return_type, TypeRef::Vec(_)) {
                    // Handle Vec types with error propagation
                    if let TypeRef::Vec(inner) = &func.return_type {
                        let go_elem = go_type(inner);
                        writeln!(out, "\tif ptr == nil {{").ok();
                        writeln!(out, "\t\treturn nil, fmt.Errorf(\"failed to get result\")").ok();
                        writeln!(out, "\t}}").ok();
                        writeln!(out, "\tdefer C.{}_free_string(ptr)", ffi_prefix).ok();
                        writeln!(out, "\tvar result []{}", go_elem).ok();
                        writeln!(
                            out,
                            "\tif err := json.Unmarshal([]byte(C.GoString(ptr)), &result); err != nil {{"
                        )
                        .ok();
                        writeln!(out, "\t\treturn nil, fmt.Errorf(\"failed to unmarshal: %w\", err)").ok();
                        writeln!(out, "\t}}").ok();
                        writeln!(out, "\treturn result, nil").ok();
                    }
                } else {
                    writeln!(
                        out,
                        "\treturn {}, nil",
                        go_return_expr(&func.return_type, "ptr", ffi_prefix, opaque_names)
                    )
                    .ok();
                }
            } else {
                writeln!(
                    out,
                    "\treturn {}",
                    go_return_expr(&func.return_type, "ptr", ffi_prefix, opaque_names)
                )
                .ok();
            }
        }
    } else if matches!(func.return_type, TypeRef::Unit) {
        writeln!(out, "\t{}", c_call).ok();
    } else {
        writeln!(out, "\tptr := {}", c_call).ok();
        // Add defer free for C string returns.
        // Bytes pointers are NOT freed — they alias internal storage.
        if matches!(
            func.return_type,
            TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json
        ) {
            writeln!(out, "\tdefer C.{}_free_string(ptr)", ffi_prefix).ok();
        }
        // For non-opaque Named types, free the handle after JSON extraction.
        // Opaque types are NOT freed here — the caller owns them via the Go wrapper.
        if let TypeRef::Named(name) = &func.return_type {
            if !opaque_names.contains(name.as_str()) {
                let type_snake = name.to_snake_case();
                writeln!(out, "\tdefer C.{}_{}_free(ptr)", ffi_prefix, type_snake).ok();
            }
        }
        writeln!(
            out,
            "\treturn {}",
            go_return_expr(&func.return_type, "ptr", ffi_prefix, opaque_names)
        )
        .ok();
    }

    writeln!(out, "}}").ok();
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
}
