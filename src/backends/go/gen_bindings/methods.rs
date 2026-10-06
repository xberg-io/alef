use super::cancellation;
use super::functions::{is_bytes_result_method, params_require_marshal};
use super::result_presence::result_presence_gate;
use super::types::{emit_type_doc, go_return_expr, go_return_values_with_error};
use crate::backends::go::c_symbols;
use crate::backends::go::type_map::{go_optional_type, go_return_type, go_type, go_zero_value};
use crate::codegen::naming::{go_param_name, go_type_name, to_go_name};
use crate::core::ir::{MethodDef, TypeDef, TypeRef};

/// The parts of a method's Go signature shared by its plain wrapper and its `WithContext` sibling.
struct MethodSignature {
    go_name: String,
    receiver_name: &'static str,
    receiver_type: String,
    params: Vec<String>,
    param_names: Vec<String>,
    return_type: String,
    can_return_error: bool,
}

impl MethodSignature {
    fn new(typ: &TypeDef, method: &MethodDef, opaque_names: &std::collections::HashSet<&str>) -> Self {
        let receiver_requires_marshal = !method.is_static && !typ.is_opaque;
        let method_marshals = receiver_requires_marshal || params_require_marshal(&method.params, opaque_names);
        let can_return_error = method.error_type.is_some() || method_marshals;

        let return_type = if is_bytes_result_method(method) {
            "([]byte, error)".to_string()
        } else if can_return_error {
            if matches!(method.return_type, TypeRef::Unit) {
                "error".to_string()
            } else {
                let ret_go_type = if matches!(
                    method.return_type,
                    TypeRef::Primitive(_) | TypeRef::Duration | TypeRef::String | TypeRef::Char | TypeRef::Path
                ) {
                    go_type(&method.return_type).into_owned()
                } else {
                    go_return_type(&method.return_type).into_owned()
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
            go_return_type(&method.return_type).into_owned()
        };

        let params = method
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

        Self {
            go_name: to_go_name(&method.name),
            receiver_name: if typ.is_opaque { "h" } else { "r" },
            receiver_type: go_type_name(&typ.name),
            params,
            param_names: method.params.iter().map(|p| go_param_name(&p.name)).collect(),
            return_type,
            can_return_error,
        }
    }

    fn is_cancellable(&self, method: &MethodDef) -> bool {
        cancellation::is_cancellable(
            method.is_async,
            self.can_return_error,
            &method.return_type,
            method.receiver.as_ref(),
            &method.params,
        )
    }

    /// The opening `func ... {` line; `go_name` and `params` are passed in because the
    /// `WithContext` sibling renames the function and prepends a parameter.
    fn render_head(&self, method: &MethodDef, go_name: &str, params: &str) -> String {
        let ret_type_str = if self.return_type.is_empty() {
            String::new()
        } else {
            format!(" {}", self.return_type)
        };
        if method.is_static {
            crate::backends::go::template_env::render(
                "method_signature_static.jinja",
                minijinja::context! {
                    receiver_type => &self.receiver_type,
                    method_name => go_name,
                    params => params,
                    return_type => &ret_type_str,
                },
            )
        } else {
            crate::backends::go::template_env::render(
                "method_signature_instance.jinja",
                minijinja::context! {
                    receiver_name => self.receiver_name,
                    receiver_type => &self.receiver_type,
                    method_name => go_name,
                    params => params,
                    return_type => &ret_type_str,
                },
            )
        }
    }
}

/// Generate a wrapper method for a struct method.
///
/// A method backed by a cancellable FFI export is emitted twice: `<Name>WithContext(ctx, ...)`
/// does the work, and `<Name>(...)` keeps its signature and forwards with `context.Background()`.
pub(super) fn gen_method_wrapper(
    typ: &TypeDef,
    method: &MethodDef,
    ffi_prefix: &str,
    opaque_names: &std::collections::HashSet<&str>,
    value_only_types: &std::collections::HashSet<String>,
    enum_names: &std::collections::HashSet<String>,
    ffi_param_enum_names: &std::collections::HashSet<String>,
) -> String {
    let signature = MethodSignature::new(typ, method, opaque_names);
    if !signature.is_cancellable(method) {
        return gen_method_wrapper_impl(
            typ,
            method,
            ffi_prefix,
            opaque_names,
            value_only_types,
            enum_names,
            ffi_param_enum_names,
            false,
        );
    }

    let mut out = String::with_capacity(4096);
    emit_type_doc(&mut out, &signature.go_name, &method.doc, "is a method.");
    out.push_str(&signature.render_head(method, &signature.go_name, &signature.params.join(", ")));
    let with_context_name = format!("{}WithContext", signature.go_name);
    let target = if method.is_static {
        format!("{}{with_context_name}", signature.receiver_type)
    } else {
        format!("{}.{with_context_name}", signature.receiver_name)
    };
    out.push_str(&format!(
        "\treturn {target}({})\n}}\n\n",
        cancellation::forwarded_args(&signature.param_names)
    ));
    out.push_str(&gen_method_wrapper_impl(
        typ,
        method,
        ffi_prefix,
        opaque_names,
        value_only_types,
        enum_names,
        ffi_param_enum_names,
        true,
    ));
    out
}

#[allow(clippy::too_many_arguments)]
fn gen_method_wrapper_impl(
    typ: &TypeDef,
    method: &MethodDef,
    ffi_prefix: &str,
    opaque_names: &std::collections::HashSet<&str>,
    value_only_types: &std::collections::HashSet<String>,
    _enum_names: &std::collections::HashSet<String>,
    ffi_param_enum_names: &std::collections::HashSet<String>,
    with_context: bool,
) -> String {
    let mut out = String::with_capacity(2048);

    let signature = MethodSignature::new(typ, method, opaque_names);
    let method_go_name = if with_context {
        format!("{}WithContext", signature.go_name)
    } else {
        signature.go_name.clone()
    };
    let last_error_call = cancellation::last_error_call(with_context);

    if with_context {
        let plain_name = &signature.go_name;
        out.push_str(&format!(
            "// {method_go_name} is {plain_name} with a context: when ctx ends, the in-flight native\n\
             // request is aborted and the returned error wraps ctx.Err().\n"
        ));
    } else {
        emit_type_doc(&mut out, &method_go_name, &method.doc, "is a method.");
    }

    let method_can_return_error = signature.can_return_error;

    let is_bytes_result = is_bytes_result_method(method);

    let receiver_name = signature.receiver_name;
    let params_str = if with_context {
        cancellation::with_ctx_param(&signature.params.join(", "))
    } else {
        signature.params.join(", ")
    };
    out.push_str(&signature.render_head(method, &method_go_name, &params_str));
    out.push_str(&crate::backends::go::template_env::render(
        "lock_os_thread.jinja",
        minijinja::Value::default(),
    ));

    {
        let returns_value_and_error = method_can_return_error && !matches!(method.return_type, TypeRef::Unit);
        let param_err_return_prefix: String = if returns_value_and_error {
            format!("{}, ", go_zero_value(&method.return_type))
        } else {
            String::new()
        };
        if with_context {
            out.push_str(&cancellation::prelude(ffi_prefix, &param_err_return_prefix));
        }
        for param in &method.params {
            out.push_str(&gen_param_to_c(
                param,
                &GoParamCtx {
                    err_return_prefix: &param_err_return_prefix,
                    can_return_error: method_can_return_error,
                    ffi_prefix,
                    opaque_names,
                    ffi_param_enum_names,
                },
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

        let type_snake = c_symbols::type_component(&typ.name);
        let primary_c_fn = c_symbols::method_symbol(ffi_prefix, &typ.name, &method.name);
        let c_fn = if with_context {
            cancellation::cancellable_symbol(&primary_c_fn)
        } else {
            primary_c_fn
        };
        let base_c_call = if method.is_static {
            if c_params.is_empty() {
                format!("C.{}()", c_fn)
            } else {
                format!("C.{}({})", c_fn, c_params.join(", "))
            }
        } else if typ.is_opaque {
            let c_receiver = format!("{}.ptr", receiver_name);
            if c_params.is_empty() {
                format!("C.{}({})", c_fn, c_receiver)
            } else {
                format!("C.{}({}, {})", c_fn, c_receiver, c_params.join(", "))
            }
        } else {
            let err_prefix = if returns_value_and_error {
                format!("{}, ", go_zero_value(&method.return_type))
            } else {
                String::new()
            };
            let err_action = format!("return {err_prefix}fmt.Errorf(\"failed to marshal receiver: %w\", err)");
            let from_json_err_action = format!("return {err_prefix}wrapLastError(\"failed to create receiver\")");
            out.push_str(&crate::backends::go::template_env::render(
                "marshal_receiver_to_c.jinja",
                minijinja::context! {
                    receiver_name => receiver_name,
                    err_action => &err_action,
                    from_json_err_action => &from_json_err_action,
                    ffi_prefix => ffi_prefix,
                    type_snake => &type_snake,
                    receiver_is_consumed => matches!(method.receiver, Some(crate::core::ir::ReceiverKind::Owned)),
                },
            ));
            if c_params.is_empty() {
                format!("C.{}(cRecv)", c_fn)
            } else {
                format!("C.{}(cRecv, {})", c_fn, c_params.join(", "))
            }
        };

        // The companion's C signature is the primary export's parameter list, so its argument
        // expressions are built from the same receiver/param pieces `base_c_call` used. ~keep
        let presence_args: Vec<String> = if method.is_static {
            c_params.clone()
        } else {
            let receiver_arg = if typ.is_opaque {
                format!("{receiver_name}.ptr")
            } else {
                "cRecv".to_string()
            };
            std::iter::once(receiver_arg).chain(c_params.iter().cloned()).collect()
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
        let c_call = if with_context {
            cancellation::with_token_arg(&c_call)
        } else {
            c_call
        };

        if is_bytes_result {
            out.push_str(&crate::backends::go::template_env::render(
                "bytes_result_call.jinja",
                minijinja::context! { c_call => &c_call, ffi_prefix => ffi_prefix, last_error_call => last_error_call },
            ));
            out.push_str("}\n");
            return out;
        }

        let is_builder_return = !method.is_static
            && typ.is_opaque
            && matches!(&method.return_type, TypeRef::Named(n) if n.as_str() == typ.name.as_str());

        if let Some(gate) = result_presence_gate(
            &method.return_type,
            method.receiver.as_ref(),
            &c_fn,
            &presence_args,
            method_can_return_error,
        ) {
            out.push_str(&gate);
        }

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
                            free_string_fn => c_symbols::free_string_symbol(ffi_prefix),
                            type_snake => &type_snake,
                            recv => receiver_name,
                        },
                    ));
                }
                if method.error_type.is_some() {
                    out.push_str(&format!("\treturn {last_error_call}\n"));
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
                    out.push_str(&format!("\tif err := {last_error_call}; err != nil {{\n"));
                    if matches!(
                        method.return_type,
                        TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json
                    ) {
                        out.push_str("\t\tif ptr != nil {\n");
                        out.push_str(&crate::backends::go::template_env::render(
                            "free_string_on_error.jinja",
                            minijinja::context! {
                                free_string_fn => c_symbols::free_string_symbol(ffi_prefix),
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
                            free_string_fn => c_symbols::free_string_symbol(ffi_prefix),
                            ptr => "ptr",
                        },
                    ));
                }
                if let TypeRef::Named(name) = &method.return_type
                    && !opaque_names.contains(name.as_str())
                {
                    let type_snake = c_symbols::type_component(name);
                    out.push_str(&crate::backends::go::template_env::render(
                        "free_type.jinja",
                        minijinja::context! {
                            ffi_prefix => ffi_prefix,
                            type_snake => &type_snake,
                            ptr => "ptr",
                        },
                    ));
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
                    let return_values = go_return_values_with_error(
                        &method.return_type,
                        "ptr",
                        ffi_prefix,
                        opaque_names,
                        value_only_types,
                    );
                    out.push_str(&crate::backends::go::template_env::render(
                        "method_return_simple.jinja",
                        minijinja::context! {
                            value => return_values,
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
                        free_string_fn => c_symbols::free_string_symbol(ffi_prefix),
                        ptr => "ptr",
                    },
                ));
            }
            if let TypeRef::Named(name) = &method.return_type
                && !opaque_names.contains(name.as_str())
            {
                let type_snake = c_symbols::type_component(name);
                out.push_str(&crate::backends::go::template_env::render(
                    "free_type.jinja",
                    minijinja::context! {
                        ffi_prefix => ffi_prefix,
                        type_snake => &type_snake,
                        ptr => "ptr",
                    },
                ));
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

mod params;
mod streaming;

pub(super) use params::{GoParamCtx, gen_param_to_c};
pub(super) use streaming::{StreamingWrapperCtx, gen_streaming_method_wrapper};

#[cfg(test)]
mod streaming_error_paths_tests;
#[cfg(test)]
mod tests;
