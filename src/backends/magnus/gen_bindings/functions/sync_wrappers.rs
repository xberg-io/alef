use super::scan_args_defaults::{
    gen_scan_args_prologue_with_defaults, last_param_is_default_struct, needs_variadic_arity,
};
use super::serde_bindings::{
    magnus_ahash_pre_call_bindings, magnus_call_args_with_ahash, magnus_serde_let_bindings, magnus_serde_recoverable,
};
use crate::backends::magnus::type_map::MagnusMapper;
use crate::codegen::generators;
use crate::codegen::shared::function_params;
use crate::codegen::type_mapper::TypeMapper;
use crate::core::ir::{ApiSurface, FunctionDef, TypeRef};
use ahash::AHashSet;

/// Generate a free function binding.
pub(in crate::backends::magnus::gen_bindings) fn gen_function(
    func: &FunctionDef,
    mapper: &MagnusMapper,
    opaque_types: &AHashSet<String>,
    mutex_types: &AHashSet<String>,
    core_import: &str,
    api: &ApiSurface,
) -> String {
    let is_default_config_func = last_param_is_default_struct(func, api);
    let variadic = needs_variadic_arity(&func.params) || is_default_config_func;

    // For non-opaque Named params, accept magnus::Value so a plain Ruby Hash works directly.
    // The binding calls to_json internally before serde_json deserialization.
    let params = if variadic {
        "args: &[magnus::Value]".to_string()
    } else {
        function_params(&func.params, &|ty| {
            if let TypeRef::Named(name) = ty {
                if !opaque_types.contains(name.as_str()) {
                    return "magnus::Value".to_string();
                }
            }
            mapper.map_type(ty)
        })
    };
    let return_type = mapper.map_type(&func.return_type);
    // Async functions always return Result because Runtime::new() can fail.
    // Variadic functions must return Result because scan_args uses ? operator.
    let has_error = func.error_type.is_some() || func.is_async || variadic;
    let return_annotation = mapper.wrap_return(&return_type, has_error);

    let can_delegate = crate::codegen::shared::can_auto_delegate_function(func, opaque_types);
    let serde_recoverable = !can_delegate && magnus_serde_recoverable(func, opaque_types);

    // Check if any param is a Vec<Named> that will need `{name}_core` rebinding.
    let needs_vec_named_let_binding = func.params.iter().any(|p| match &p.ty {
        TypeRef::Vec(inner) => matches!(inner.as_ref(), TypeRef::Named(name) if !opaque_types.contains(name.as_str())),
        _ => false,
    });

    // Generate serde_magnus deserialization preamble for non-opaque Named params.
    // Two emission modes:
    //   - delegate path: rebind {name} to the binding type so the existing call_args gen works.
    //   - serde-recovery path: emit `{name}_core: core::Type` so gen_call_args_with_let_bindings
    //     can pass `&{name}_core` to the core function.
    let mut deser_lines = Vec::new();
    // When a Vec<Named> param forces the `_core` call-arg path (gen_call_args_with_let_bindings_*),
    // every Named param in the call is referenced as `{name}_core`. The serde let-binding emitter
    // names scalar Named params `{name}_core` too, so use it here as well — otherwise the non-serde
    // preamble would bind a scalar Named param as `{name}` and the `&{name}_core` call site would
    // not resolve.
    if serde_recoverable || needs_vec_named_let_binding {
        deser_lines.extend(magnus_serde_let_bindings(
            &func.params,
            opaque_types,
            core_import,
            mapper,
            is_default_config_func,
        ));
    } else {
        for (idx, p) in func.params.iter().enumerate() {
            let promoted = crate::codegen::shared::is_promoted_optional(&func.params, idx);
            if let TypeRef::Named(name) = &p.ty {
                if !opaque_types.contains(name.as_str()) {
                    let binding_ty = &p.name;
                    if p.optional {
                        deser_lines.push(crate::backends::magnus::template_env::render(
                            "function_named_binding.rs.jinja",
                            minijinja::context! {
                                mode => "optional",
                                binding_name => binding_ty,
                                core_import => core_import,
                                type_name => name,
                                is_mut => p.is_mut,
                            },
                        ));
                    } else if promoted || (idx == func.params.len() - 1 && is_default_config_func) {
                        deser_lines.push(crate::backends::magnus::template_env::render(
                            "function_named_binding.rs.jinja",
                            minijinja::context! {
                                mode => "default",
                                binding_name => binding_ty,
                                core_import => core_import,
                                type_name => name,
                                is_mut => p.is_mut,
                            },
                        ));
                    } else {
                        deser_lines.push(crate::backends::magnus::template_env::render(
                            "function_named_binding.rs.jinja",
                            minijinja::context! {
                                mode => "required",
                                binding_name => binding_ty,
                                core_import => core_import,
                                type_name => name,
                                is_mut => p.is_mut,
                            },
                        ));
                    }
                }
            } else if let TypeRef::Vec(inner) = &p.ty {
                if let TypeRef::Named(name) = inner.as_ref() {
                    if !opaque_types.contains(name.as_str()) {
                        let core_inner_ty = format!("{core_import}::{name}");
                        let vec_ty = format!("Vec<{core_inner_ty}>");
                        deser_lines.push(crate::backends::magnus::template_env::render(
                            "function_named_vec_binding.rs.jinja",
                            minijinja::context! {
                                name => &p.name,
                                vec_ty => &vec_ty,
                                optional => p.optional,
                            },
                        ));
                    }
                }
            }
        }
    }

    // AHashMap<Cow<'static, str>, Value> params: Ruby receives these as
    // HashMap<String, String>. Emit pre-call `let __<name>_ahash` bindings so the
    // call site can borrow a properly-typed AHashMap.
    let ahash_bindings = magnus_ahash_pre_call_bindings(&func.params);
    deser_lines.extend(ahash_bindings);

    // When variadic, prepend scan_args prologue to unpack individual bindings from args slice.
    let scan_args_prologue = if variadic {
        format!(
            "{}\n    ",
            gen_scan_args_prologue_with_defaults(&func.params, mapper, opaque_types, is_default_config_func)
        )
    } else {
        String::new()
    };

    let deser_preamble = if deser_lines.is_empty() {
        String::new()
    } else {
        format!("{}\n    ", deser_lines.join("\n    "))
    };

    let body = if can_delegate || serde_recoverable || needs_vec_named_let_binding {
        let base_call_args = if serde_recoverable || needs_vec_named_let_binding {
            generators::gen_call_args_with_let_bindings_json_str(&func.params, opaque_types)
        } else {
            generators::gen_call_args(&func.params, opaque_types)
        };
        let call_args = magnus_call_args_with_ahash(&func.params, opaque_types, &base_call_args);
        let core_fn_path = {
            let path = func.rust_path.replace('-', "_");
            if path.starts_with(core_import) {
                path
            } else {
                format!("{core_import}::{}", func.name)
            }
        };
        let core_call = format!("{core_fn_path}({call_args})");
        if func.is_async {
            // Async core function: wrap in tokio runtime block_on.
            // Runtime::new() can fail, so always use map_err and return Ok(...).
            let wrap = generators::wrap_return_with_mutex_mapped(
                "result",
                &func.return_type,
                "",
                opaque_types,
                mutex_types,
                false,
                func.returns_ref,
                false,
                mapper,
            );
            if func.error_type.is_some() {
                crate::backends::magnus::template_env::render(
                    "function_async_body.rs.jinja",
                    minijinja::context! {
                        core_call => &core_call,
                        wrap => &wrap,
                        has_error => true,
                    },
                )
            } else {
                crate::backends::magnus::template_env::render(
                    "function_async_body.rs.jinja",
                    minijinja::context! {
                        core_call => &core_call,
                        wrap => &wrap,
                        has_error => false,
                    },
                )
            }
        } else if func.error_type.is_some() {
            let wrap = generators::wrap_return_with_mutex_mapped(
                "result",
                &func.return_type,
                "",
                opaque_types,
                mutex_types,
                false,
                func.returns_ref,
                false,
                mapper,
            );
            crate::backends::magnus::template_env::render(
                "function_result_body.rs.jinja",
                minijinja::context! {
                    core_call => &core_call,
                    wrap => &wrap,
                },
            )
        } else if variadic {
            // Variadic functions must return Result (scan_args uses ?), so wrap plain value in Ok().
            let inner = generators::wrap_return_with_mutex_mapped(
                &core_call,
                &func.return_type,
                "",
                opaque_types,
                mutex_types,
                false,
                func.returns_ref,
                false,
                mapper,
            );
            crate::backends::magnus::template_env::render(
                "function_variadic_ok_body.rs.jinja",
                minijinja::context! {
                    inner => &inner,
                },
            )
        } else {
            generators::wrap_return_with_mutex_mapped(
                &core_call,
                &func.return_type,
                "",
                opaque_types,
                mutex_types,
                false,
                func.returns_ref,
                false,
                mapper,
            )
        }
    } else {
        gen_magnus_unimplemented_body(&func.return_type, &func.name, func.error_type.is_some() || variadic)
    };
    // Add #[allow(unused_variables)] to functions with unimplemented bodies to suppress warnings for unused params
    let allow_attr = if !can_delegate && !serde_recoverable {
        "#[allow(unused_variables)]\n"
    } else {
        ""
    };
    crate::backends::magnus::template_env::render(
        "function_wrapper.rs.jinja",
        minijinja::context! {
            allow_attr => allow_attr,
            name => &func.name,
            params => &params,
            return_annotation => &return_annotation,
            scan_args_prologue => &scan_args_prologue,
            deser_preamble => &deser_preamble,
            body => &body,
        },
    )
}

/// Generate a type-appropriate unsupported body for Magnus.
pub(in crate::backends::magnus::gen_bindings) fn gen_magnus_unimplemented_body(
    return_type: &crate::core::ir::TypeRef,
    fn_name: &str,
    has_error: bool,
) -> String {
    use crate::core::ir::TypeRef;
    let err_msg = format!("Not implemented: {fn_name}");
    if has_error {
        crate::backends::magnus::template_env::render(
            "function_unimplemented_error.rs.jinja",
            minijinja::context! {
                message => &err_msg,
            },
        )
    } else {
        match return_type {
            TypeRef::Unit => "()".to_string(),
            TypeRef::String | TypeRef::Char | TypeRef::Path => crate::backends::magnus::template_env::render(
                "function_unimplemented_string.rs.jinja",
                minijinja::context! {
                    name => fn_name,
                },
            ),
            TypeRef::Bytes => "Vec::new()".to_string(),
            TypeRef::Primitive(p) => match p {
                crate::core::ir::PrimitiveType::Bool => "false".to_string(),
                _ => "0".to_string(),
            },
            TypeRef::Optional(_) => "None".to_string(),
            TypeRef::Vec(_) => "Vec::new()".to_string(),
            TypeRef::Map(_, _) => "Default::default()".to_string(),
            TypeRef::Duration => "0u64".to_string(),
            TypeRef::Named(_) | TypeRef::Json => crate::backends::magnus::template_env::render(
                "function_unimplemented_panic.rs.jinja",
                minijinja::context! {
                    name => fn_name,
                },
            ),
        }
    }
}
