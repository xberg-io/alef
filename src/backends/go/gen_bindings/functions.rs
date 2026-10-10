use super::cancellation;
use super::methods::{GoParamCtx, gen_param_to_c};
use super::result_presence::result_presence_gate;
use super::types::{emit_type_doc, go_return_expr, go_return_values_with_error};
use crate::backends::go::c_symbols;
use crate::backends::go::type_map::{go_optional_type, go_return_type, go_type};
use crate::codegen::mut_writeback;
use crate::codegen::naming::{go_free_function_name, go_param_name};
use crate::core::ir::{FunctionDef, MethodDef, ParamDef, TypeRef};
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

/// Returns true when the function uses the owned byte-buffer return ABI.
///
/// `Bytes` and `Optional<Bytes>` alike use the out-param convention:
/// `(args..., **uint8_t, *uintptr_t, *uintptr_t) -> i32`. Mirrors
/// `backends::ffi::gen_bindings::functions::returns_bytes_out_params`, which carries the
/// full rationale. Matching only bare `Bytes` here would model `Optional<Bytes>` as a
/// direct `*mut u8` return and call the C symbol without its three out-params, against a
/// header that declares them — the binding and the header disagreeing about one symbol.
///
/// Absence rides on `*out_ptr == NULL` with `*out_len == *out_cap == 0`, which maps to
/// Go's `nil` slice. `Some(&[])` is a distinct, non-null value: presence always runs
/// `Box::into_raw`, so `outPtr != nil && outLen == 0` is an empty present value and must
/// not be collapsed into absence. ~keep
fn is_bytes_result_func(func: &FunctionDef) -> bool {
    returns_bytes_out_params(&func.return_type)
}

/// Same check for MethodDef — needed by methods.rs.
pub(super) fn is_bytes_result_method(method: &MethodDef) -> bool {
    returns_bytes_out_params(&method.return_type)
}

fn returns_bytes_out_params(ty: &TypeRef) -> bool {
    match ty {
        TypeRef::Bytes => true,
        TypeRef::Optional(inner) => matches!(inner.as_ref(), TypeRef::Bytes),
        _ => false,
    }
}

/// The parts of a free function's Go signature shared by its plain wrapper and its `WithContext`
/// sibling.
struct FunctionSignature {
    go_name: String,
    params: Vec<String>,
    param_names: Vec<String>,
    return_type: String,
    can_return_error: bool,
    has_bridge_param: bool,
    has_writeback: bool,
}

impl FunctionSignature {
    fn new(
        func: &FunctionDef,
        opaque_names: &std::collections::HashSet<&str>,
        bridge_param_names: &HashSet<String>,
        bridge_type_aliases: &HashSet<String>,
        reserved_type_names: &HashSet<String>,
    ) -> Self {
        let is_bytes_result = is_bytes_result_func(func);
        let non_bridge_params: Vec<_> = func
            .params
            .iter()
            .filter(|p| !is_bridge_param(p, bridge_param_names, bridge_type_aliases))
            .cloned()
            .collect();
        let marshals_params = params_require_marshal(&non_bridge_params, opaque_names);

        // A `&mut T` DTO parameter on a unit-returning function cannot be bound as an owned
        // by-value parameter: the C call mutates a temporary handle built from JSON and the
        // handle is then freed unread, so the caller's value is silently untouched. When this
        // narrow shape applies, the binding returns the updated `T` instead of `error` alone.
        // `reject_unsupported_writeback` (called by the caller loop before this function runs)
        // has already ruled out every `&mut` DTO shape this can't express. ~keep
        let opaque_names_ahash: ahash::AHashSet<String> = opaque_names.iter().map(|s| s.to_string()).collect();
        let writeback = mut_writeback::writeback_param(&func.params, &func.return_type, &opaque_names_ahash);
        let can_return_error = func.error_type.is_some() || marshals_params || writeback.is_some();

        let return_type = if let Some(wb) = writeback {
            format!("({}, error)", go_optional_type(&wb.ty))
        } else if is_bytes_result {
            "([]byte, error)".to_string()
        } else if can_return_error {
            if matches!(func.return_type, TypeRef::Unit) {
                "error".to_string()
            } else if matches!(
                func.return_type,
                TypeRef::Primitive(_) | TypeRef::Duration | TypeRef::String | TypeRef::Char | TypeRef::Path
            ) {
                format!("({}, error)", go_type(&func.return_type))
            } else {
                format!("({}, error)", go_return_type(&func.return_type))
            }
        } else if matches!(func.return_type, TypeRef::Unit) {
            "".to_string()
        } else if matches!(
            func.return_type,
            TypeRef::Primitive(_) | TypeRef::Duration | TypeRef::String | TypeRef::Char | TypeRef::Path
        ) {
            go_type(&func.return_type).into_owned()
        } else {
            go_return_type(&func.return_type).into_owned()
        };

        let mut params: Vec<String> = Vec::new();
        let mut param_names: Vec<String> = Vec::new();
        for p in func.params.iter() {
            if is_bridge_param(p, bridge_param_names, bridge_type_aliases) {
                continue;
            }
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
            params.push(format!("{} {}", go_param_name(&p.name), param_type));
            param_names.push(go_param_name(&p.name));
        }

        Self {
            go_name: go_free_function_name(&func.name, reserved_type_names),
            params,
            param_names,
            return_type,
            can_return_error,
            has_bridge_param: non_bridge_params.len() != func.params.len(),
            has_writeback: writeback.is_some(),
        }
    }

    fn is_cancellable(&self, func: &FunctionDef) -> bool {
        !self.has_bridge_param
            && !self.has_writeback
            && cancellation::is_cancellable(
                func.is_async,
                self.can_return_error,
                &func.return_type,
                None,
                &func.params,
            )
    }

    fn render_head(&self, go_name: &str, params: &str) -> String {
        let ret_type_str = if self.return_type.is_empty() {
            String::new()
        } else {
            format!(" {}", self.return_type)
        };
        crate::backends::go::template_env::render(
            "function_signature.jinja",
            crate::alef_context! {
                func_name => go_name,
                params => params,
                return_type => &ret_type_str,
            },
        )
    }
}

/// Generate a wrapper function for a free function.
///
/// A function backed by a cancellable FFI export is emitted twice: `<Name>WithContext(ctx, ...)`
/// does the work, and `<Name>(...)` keeps its signature and forwards with `context.Background()`.
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
    reserved_type_names: &HashSet<String>,
) -> String {
    let signature = FunctionSignature::new(
        func,
        opaque_names,
        bridge_param_names,
        bridge_type_aliases,
        reserved_type_names,
    );
    let generate = |with_context: bool| {
        gen_function_wrapper_impl(
            func,
            ffi_prefix,
            opaque_names,
            bridge_param_names,
            bridge_type_aliases,
            value_only_types,
            enum_names,
            ffi_param_enum_names,
            reserved_type_names,
            with_context,
        )
    };
    if !signature.is_cancellable(func) {
        return generate(false);
    }

    let mut out = String::with_capacity(4096);
    emit_type_doc(&mut out, &signature.go_name, &func.doc, "calls the FFI function.");
    out.push_str(&signature.render_head(&signature.go_name, &signature.params.join(", ")));
    out.push_str(&format!(
        "\treturn {}WithContext({})\n}}\n\n",
        signature.go_name,
        cancellation::forwarded_args(&signature.param_names)
    ));
    out.push_str(&generate(true));
    out
}

#[allow(clippy::too_many_arguments)]
fn gen_function_wrapper_impl(
    func: &FunctionDef,
    ffi_prefix: &str,
    opaque_names: &std::collections::HashSet<&str>,
    bridge_param_names: &HashSet<String>,
    bridge_type_aliases: &HashSet<String>,
    value_only_types: &std::collections::HashSet<String>,
    _enum_names: &std::collections::HashSet<String>,
    ffi_param_enum_names: &std::collections::HashSet<String>,
    reserved_type_names: &HashSet<String>,
    with_context: bool,
) -> String {
    let mut out = String::with_capacity(2048);

    let signature = FunctionSignature::new(
        func,
        opaque_names,
        bridge_param_names,
        bridge_type_aliases,
        reserved_type_names,
    );
    let func_go_name = if with_context {
        format!("{}WithContext", signature.go_name)
    } else {
        signature.go_name.clone()
    };
    let last_error_call = cancellation::last_error_call(with_context);

    if with_context {
        let plain_name = &signature.go_name;
        out.push_str(&format!(
            "// {func_go_name} is {plain_name} with a context: when ctx ends, the in-flight native\n\
             // request is aborted and the returned error wraps ctx.Err().\n"
        ));
    } else {
        emit_type_doc(&mut out, &func_go_name, &func.doc, "calls the FFI function.");
    }

    let is_bytes_result = is_bytes_result_func(func);
    let can_return_error = signature.can_return_error;
    let opaque_names_ahash: ahash::AHashSet<String> = opaque_names.iter().map(|s| s.to_string()).collect();
    let writeback = mut_writeback::writeback_param(&func.params, &func.return_type, &opaque_names_ahash);

    let primary_ffi_symbol = c_symbols::free_function_symbol(ffi_prefix, &func.name);
    let ffi_symbol = if with_context {
        cancellation::cancellable_symbol(&primary_ffi_symbol)
    } else {
        primary_ffi_symbol
    };
    let ffi_name = format!("C.{ffi_symbol}");

    let params_str = if with_context {
        cancellation::with_ctx_param(&signature.params.join(", "))
    } else {
        signature.params.join(", ")
    };
    out.push_str(&signature.render_head(&func_go_name, &params_str));
    out.push_str(&crate::backends::go::template_env::render(
        "lock_os_thread.jinja",
        minijinja::Value::default(),
    ));

    let returns_value_and_error =
        can_return_error && (writeback.is_some() || !matches!(func.return_type, TypeRef::Unit));
    let param_err_return_prefix: String = if returns_value_and_error {
        format!("{}, ", crate::backends::go::type_map::go_zero_value(&func.return_type))
    } else {
        String::new()
    };
    if with_context {
        out.push_str(&cancellation::prelude(ffi_prefix, &param_err_return_prefix));
    }
    for param in func.params.iter() {
        if is_bridge_param(param, bridge_param_names, bridge_type_aliases) {
            continue;
        }
        out.push_str(&gen_param_to_c(
            param,
            &GoParamCtx {
                err_return_prefix: &param_err_return_prefix,
                can_return_error,
                ffi_prefix,
                opaque_names,
                ffi_param_enum_names,
            },
        ));
    }

    let c_params: Vec<String> = func
        .params
        .iter()
        .flat_map(|p| -> Vec<String> {
            if is_bridge_param(p, bridge_param_names, bridge_type_aliases) {
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

    let c_call = if is_bytes_result {
        let mut all_params = c_params.clone();
        all_params.push("&outPtr".to_string());
        all_params.push("&outLen".to_string());
        all_params.push("&outCap".to_string());
        format!("{}({})", ffi_name, all_params.join(", "))
    } else {
        format!("{}({})", ffi_name, c_params.join(", "))
    };
    let c_call = if with_context {
        cancellation::with_token_arg(&c_call)
    } else {
        c_call
    };

    if let Some(wb) = writeback {
        let wb_type_name = mut_writeback::writeback_type_name(wb).unwrap_or_default();
        let wb_type_snake = c_symbols::type_component(wb_type_name);
        let wb_handle = go_param_name(&format!("c_{}", wb.name));
        out.push_str(&crate::backends::go::template_env::render(
            "c_call_simple.jinja",
            crate::alef_context! {
                c_call => &c_call,
            },
        ));
        out.push_str(&crate::backends::go::template_env::render(
            "mut_writeback_return.jinja",
            crate::alef_context! {
                ffi_prefix => ffi_prefix,
                free_string_fn => c_symbols::free_string_symbol(ffi_prefix),
                type_snake => &wb_type_snake,
                type_name => wb_type_name,
                handle => &wb_handle,
            },
        ));
        out.push_str(&crate::backends::go::template_env::render(
            "function_body_end.jinja",
            minijinja::Value::default(),
        ));
        return out;
    }

    if is_bytes_result {
        out.push_str(&crate::backends::go::template_env::render(
            "bytes_result_call.jinja",
            crate::alef_context! {
                c_call => &c_call,
                ffi_prefix => ffi_prefix,
                last_error_call => last_error_call,
            },
        ));
        out.push_str(&crate::backends::go::template_env::render(
            "function_body_end.jinja",
            minijinja::Value::default(),
        ));
        return out;
    }

    if let Some(gate) = result_presence_gate(&func.return_type, None, &ffi_symbol, &c_params, can_return_error) {
        out.push_str(&gate);
    }

    if can_return_error {
        if matches!(func.return_type, TypeRef::Unit) {
            out.push_str(&crate::backends::go::template_env::render(
                "c_call_simple.jinja",
                crate::alef_context! {
                    c_call => &c_call,
                },
            ));
            if func.error_type.is_some() {
                out.push_str(&format!("\treturn {last_error_call}\n"));
            } else {
                out.push_str("\treturn nil\n");
            }
        } else {
            out.push_str(&crate::backends::go::template_env::render(
                "c_ptr_assign.jinja",
                crate::alef_context! {
                    c_call => &c_call,
                },
            ));
            if func.error_type.is_some() {
                out.push_str(&format!("\tif err := {last_error_call}; err != nil {{\n"));
                if matches!(
                    func.return_type,
                    TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json
                ) {
                    out.push_str("\t\tif ptr != nil {\n");
                    out.push_str(&crate::backends::go::template_env::render(
                        "free_string_on_error.jinja",
                        crate::alef_context! {
                            free_string_fn => c_symbols::free_string_symbol(ffi_prefix),
                        },
                    ));
                    out.push_str("\t\t}\n");
                }
                if let TypeRef::Named(name) = &func.return_type {
                    let type_snake = c_symbols::type_component(name);
                    out.push_str("\t\tif ptr != 0 {\n");
                    out.push_str(&crate::backends::go::template_env::render(
                        "free_type_on_error.jinja",
                        crate::alef_context! {
                            ffi_prefix => ffi_prefix,
                            type_snake => &type_snake,
                        },
                    ));
                    out.push_str("\t\t}\n");
                }
                let zero_value = crate::backends::go::type_map::go_zero_value(&func.return_type);
                out.push_str(&crate::backends::go::template_env::render(
                    "return_zero_err.jinja",
                    crate::alef_context! {
                        zero_value => &zero_value,
                    },
                ));
                out.push_str("\t}\n");
            }
            if matches!(
                func.return_type,
                TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json
            ) {
                out.push_str(&crate::backends::go::template_env::render(
                    "free_string.jinja",
                    crate::alef_context! {
                        free_string_fn => c_symbols::free_string_symbol(ffi_prefix),
                        ptr => "ptr",
                    },
                ));
            }
            if let TypeRef::Named(name) = &func.return_type
                && !opaque_names.contains(name.as_str())
            {
                let type_snake = c_symbols::type_component(name);
                out.push_str(&crate::backends::go::template_env::render(
                    "free_type.jinja",
                    crate::alef_context! {
                        ffi_prefix => ffi_prefix,
                        type_snake => &type_snake,
                        ptr => "ptr",
                    },
                ));
            }

            if can_return_error {
                if let TypeRef::Named(name) = &func.return_type {
                    if !opaque_names.contains(name.as_str()) {
                        let type_snake = c_symbols::type_component(name);
                        out.push_str(&crate::backends::go::template_env::render(
                            "c_json_to_json.jinja",
                            crate::alef_context! {
                                ffi_prefix => ffi_prefix,
                                type_snake => &type_snake,
                            },
                        ));
                        out.push_str("\tif jsonPtr == nil {\n");
                        out.push_str("\t\treturn nil, fmt.Errorf(\"failed to convert to JSON\")\n");
                        out.push_str("\t}\n");
                        out.push_str(&crate::backends::go::template_env::render(
                            "free_string.jinja",
                            crate::alef_context! {
                                free_string_fn => c_symbols::free_string_symbol(ffi_prefix),
                                ptr => "jsonPtr",
                            },
                        ));
                        out.push_str(&crate::backends::go::template_env::render(
                            "var_decl_type.jinja",
                            crate::alef_context! {
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
                        let return_values = go_return_values_with_error(
                            &func.return_type,
                            "ptr",
                            ffi_prefix,
                            opaque_names,
                            value_only_types,
                        );
                        out.push_str(&crate::backends::go::template_env::render(
                            "method_return_simple.jinja",
                            crate::alef_context! {
                                value => return_values,
                            },
                        ));
                    }
                } else if matches!(func.return_type, TypeRef::Vec(_)) {
                    if let TypeRef::Vec(inner) = &func.return_type {
                        let go_elem = go_type(inner);
                        out.push_str("\tif ptr == nil {\n");
                        out.push_str("\t\treturn nil, fmt.Errorf(\"failed to get result\")\n");
                        out.push_str("\t}\n");
                        out.push_str(&crate::backends::go::template_env::render(
                            "free_string.jinja",
                            crate::alef_context! {
                                free_string_fn => c_symbols::free_string_symbol(ffi_prefix),
                                ptr => "ptr",
                            },
                        ));
                        out.push_str(&crate::backends::go::template_env::render(
                            "var_decl_slice.jinja",
                            crate::alef_context! {
                                element_type => &go_elem,
                            },
                        ));
                        out.push_str("\tif err := json.Unmarshal([]byte(C.GoString(ptr)), &result); err != nil {\n");
                        out.push_str("\t\treturn nil, fmt.Errorf(\"failed to unmarshal: %w\", err)\n");
                        out.push_str("\t}\n");
                        out.push_str(&crate::backends::go::template_env::render(
                            "method_return_simple.jinja",
                            crate::alef_context! {
                                value => "result, nil",
                            },
                        ));
                    }
                } else {
                    let return_values = go_return_values_with_error(
                        &func.return_type,
                        "ptr",
                        ffi_prefix,
                        opaque_names,
                        value_only_types,
                    );
                    out.push_str(&crate::backends::go::template_env::render(
                        "method_return_simple.jinja",
                        crate::alef_context! {
                            value => return_values,
                        },
                    ));
                }
            } else {
                let return_expr = go_return_expr(&func.return_type, "ptr", ffi_prefix, opaque_names, value_only_types);
                out.push_str(&crate::backends::go::template_env::render(
                    "method_return_simple.jinja",
                    crate::alef_context! {
                        value => return_expr,
                    },
                ));
            }
        }
    } else if matches!(func.return_type, TypeRef::Unit) {
        out.push_str(&crate::backends::go::template_env::render(
            "c_call_simple.jinja",
            crate::alef_context! {
                c_call => &c_call,
            },
        ));
    } else {
        out.push_str(&crate::backends::go::template_env::render(
            "c_ptr_assign.jinja",
            crate::alef_context! {
                c_call => &c_call,
            },
        ));
        if matches!(
            func.return_type,
            TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json
        ) {
            out.push_str(&crate::backends::go::template_env::render(
                "free_string.jinja",
                crate::alef_context! {
                    free_string_fn => c_symbols::free_string_symbol(ffi_prefix),
                    ptr => "ptr",
                },
            ));
        }
        if let TypeRef::Named(name) = &func.return_type
            && !opaque_names.contains(name.as_str())
        {
            let type_snake = c_symbols::type_component(name);
            out.push_str(&crate::backends::go::template_env::render(
                "free_type.jinja",
                crate::alef_context! {
                    ffi_prefix => ffi_prefix,
                    type_snake => &type_snake,
                    ptr => "ptr",
                },
            ));
        }
        let return_expr = go_return_expr(&func.return_type, "ptr", ffi_prefix, opaque_names, value_only_types);
        out.push_str(&crate::backends::go::template_env::render(
            "method_return_simple.jinja",
            crate::alef_context! {
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

mod adapter;
mod visitor;

pub(crate) use adapter::adapter_flattened_field;
pub(super) use adapter::gen_adapter_wrapper;
pub(super) use visitor::{gen_capsule_function_wrapper, gen_convert_with_visitor_wrapper};

#[cfg(test)]
mod adapter_wrapper_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod visitor_capsule_tests;
