//! Free function and module-init code generators for the Magnus (Ruby) backend.

use ahash::AHashSet;
use alef_codegen::generators;
use alef_codegen::shared::function_params;
use alef_codegen::type_mapper::TypeMapper;
use alef_core::config::{Language, ResolvedCrateConfig};
use alef_core::ir::{ApiSurface, FieldDef, FunctionDef, ReceiverKind, TypeRef};

use crate::type_map::MagnusMapper;

/// Check if a field contains a type that cannot be safely passed across thread boundaries.
/// Magnus's #[magnus::wrap] requires Send + Sync bounds. Fields containing types like
/// VisitorHandle (Rc<RefCell<dyn HtmlVisitor>>) are !Send + !Sync and must be excluded.
fn is_thread_unsafe_field(field: &FieldDef) -> bool {
    matches!(&field.ty, TypeRef::Named(name) if name == "VisitorHandle")
        || matches!(field.ty, TypeRef::Optional(ref inner) if matches!(inner.as_ref(), TypeRef::Named(name) if name == "VisitorHandle"))
}

/// Check if the last parameter is a struct type with has_default (typically a config struct).
/// Used to determine if a function should use variadic arity for optional config handling.
fn last_param_is_default_struct(func: &FunctionDef, api: &ApiSurface) -> bool {
    func.params.last().is_some_and(|p| {
        if let TypeRef::Named(name) = &p.ty {
            api.types
                .iter()
                .find(|t| &t.name == name)
                .is_some_and(|t| t.has_default)
        } else {
            false
        }
    })
}

/// Returns true when the function has optional params (or promoted required params that follow
/// optional ones), meaning Magnus needs variadic arity (-1) with scan_args.
pub(super) fn needs_variadic_arity(params: &[alef_core::ir::ParamDef]) -> bool {
    params.iter().any(|p| p.optional) || {
        // Promoted: any required param that follows an optional one
        let mut seen_optional = false;
        params.iter().any(|p| {
            if p.optional {
                seen_optional = true;
                false
            } else {
                seen_optional && !p.optional
            }
        })
    }
}

/// Map a single parameter's type to its Magnus scan_args type string.
/// Optional and promoted params become `Option<T>`, required params become `T`.
/// When treat_as_optional is true, also wrap in Option (used for default-struct config params).
fn param_scan_args_type(
    p: &alef_core::ir::ParamDef,
    promoted: bool,
    mapper: &MagnusMapper,
    opaque_types: &AHashSet<String>,
) -> String {
    let inner = if let TypeRef::Named(name) = &p.ty {
        if !opaque_types.contains(name.as_str()) {
            "magnus::Value".to_string()
        } else {
            mapper.map_type(&p.ty)
        }
    } else {
        mapper.map_type(&p.ty)
    };
    if p.optional || promoted {
        format!("Option<{inner}>")
    } else {
        inner
    }
}

/// Extended version that accepts treat_as_optional for default-struct config params.
/// For optional String types, use Option<magnus::Value> to handle nil properly via scan_args.
fn param_scan_args_type_extended(
    p: &alef_core::ir::ParamDef,
    promoted: bool,
    mapper: &MagnusMapper,
    opaque_types: &AHashSet<String>,
    treat_as_optional: bool,
) -> String {
    let inner = if let TypeRef::Named(name) = &p.ty {
        if !opaque_types.contains(name.as_str()) {
            "magnus::Value".to_string()
        } else {
            mapper.map_type(&p.ty)
        }
    } else if matches!(p.ty, TypeRef::String) && (p.optional || promoted || treat_as_optional) {
        // For optional String, use Option<magnus::Value> and handle nil manually
        "magnus::Value".to_string()
    } else {
        mapper.map_type(&p.ty)
    };
    if p.optional || promoted || treat_as_optional {
        format!("Option<{inner}>")
    } else {
        inner
    }
}

/// Generate the scan_args call + destructuring for variadic Magnus functions.
///
/// Returns a string of Rust code that:
/// 1. Calls `scan_args` with appropriate required/optional type params.
/// 2. Destructures `.required` and `.optional` to bind individual param names.
/// 3. If last_is_default_config is true, treats the last param as optional (for config defaults).
fn gen_scan_args_prologue_with_defaults(
    params: &[alef_core::ir::ParamDef],
    mapper: &MagnusMapper,
    opaque_types: &AHashSet<String>,
    last_is_default_config: bool,
) -> String {
    let mut seen_optional = false;
    let mut req_types: Vec<String> = Vec::new();
    let mut opt_types: Vec<String> = Vec::new();
    let mut req_names: Vec<String> = Vec::new();
    let mut opt_names: Vec<String> = Vec::new();

    for (idx, p) in params.iter().enumerate() {
        let promoted = alef_codegen::shared::is_promoted_optional(params, idx);
        let is_last = idx == params.len() - 1;
        let treat_as_optional = (p.optional || promoted) || (is_last && last_is_default_config);

        if treat_as_optional {
            seen_optional = true;
            opt_types.push(param_scan_args_type_extended(
                p,
                promoted,
                mapper,
                opaque_types,
                is_last && last_is_default_config,
            ));
            opt_names.push(p.name.clone());
        } else {
            let _ = seen_optional;
            req_types.push(param_scan_args_type(p, false, mapper, opaque_types));
            req_names.push(p.name.clone());
        }
    }

    // Build the scan_args! call
    let req_type_str = req_types.join(", ");
    let opt_type_str = opt_types.join(", ");
    let _type_params = match (req_types.is_empty(), opt_types.is_empty()) {
        (true, true) => "()".to_string(),
        (false, true) => format!("({req_type_str},)"),
        (true, false) => format!("((), ({opt_type_str},))"),
        (false, false) => format!("(({req_type_str},), ({opt_type_str},))"),
    };

    // scan_args requires all 6 generic parameters: Req, Opt, Splat, Trail, Kw, Block
    // The req_type_str and opt_type_str already have proper formatting
    let scan_args_line = crate::template_env::render(
        "function_scan_args_call.rs.jinja",
        minijinja::context! {
            has_required => !req_types.is_empty(),
            has_optional => !opt_types.is_empty(),
            required_types => &req_type_str,
            optional_types => &opt_type_str,
        },
    );

    let mut lines = vec![scan_args_line];

    // Destructure required
    if !req_names.is_empty() {
        // If there's only one param, destructure the tuple directly (e.g., (html,) = ...)
        // If there are multiple, use the tuple pattern as-is
        let pat = if req_names.len() == 1 {
            format!("({},)", req_names[0])
        } else {
            format!(
                "({})",
                req_names.iter().map(|n| n.as_str()).collect::<Vec<_>>().join(", ")
            )
        };
        lines.push(crate::template_env::render(
            "function_scan_args_destructure.rs.jinja",
            minijinja::context! {
                pattern => &pat,
                source => "required",
            },
        ));
    }

    // Destructure optional
    if !opt_names.is_empty() {
        // If there's only one param, destructure the tuple directly (e.g., (options,) = ...)
        // If there are multiple, use the tuple pattern as-is
        let pat = if opt_names.len() == 1 {
            format!("({},)", opt_names[0])
        } else {
            format!(
                "({})",
                opt_names.iter().map(|n| n.as_str()).collect::<Vec<_>>().join(", ")
            )
        };
        lines.push(crate::template_env::render(
            "function_scan_args_destructure.rs.jinja",
            minijinja::context! {
                pattern => &pat,
                source => "optional",
            },
        ));
    }

    // After destructuring, convert Option<magnus::Value> back to Option<String> for optional strings
    for (idx, p) in params.iter().enumerate() {
        let promoted = alef_codegen::shared::is_promoted_optional(params, idx);
        let is_last = idx == params.len() - 1;
        let treat_as_optional = (p.optional || promoted) || (is_last && last_is_default_config);

        if treat_as_optional && matches!(p.ty, TypeRef::String) {
            lines.push(crate::template_env::render(
                "function_optional_string_scan_arg.rs.jinja",
                minijinja::context! {
                    name => &p.name,
                },
            ));
        }
    }

    lines.join("\n    ")
}

/// Generate a free function binding.
pub(super) fn gen_function(
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

    let can_delegate = alef_codegen::shared::can_auto_delegate_function(func, opaque_types);
    let serde_recoverable = !can_delegate && magnus_serde_recoverable(func, opaque_types);

    // Generate serde_magnus deserialization preamble for non-opaque Named params.
    // Two emission modes:
    //   - delegate path: rebind {name} to the binding type so the existing call_args gen works.
    //   - serde-recovery path: emit `{name}_core: core::Type` so gen_call_args_with_let_bindings
    //     can pass `&{name}_core` to the core function.
    let mut deser_lines = Vec::new();
    if serde_recoverable {
        deser_lines.extend(magnus_serde_let_bindings(
            &func.params,
            opaque_types,
            core_import,
            mapper,
            is_default_config_func,
        ));
    } else {
        for (idx, p) in func.params.iter().enumerate() {
            let promoted = alef_codegen::shared::is_promoted_optional(&func.params, idx);
            if let TypeRef::Named(name) = &p.ty {
                if !opaque_types.contains(name.as_str()) {
                    let binding_ty = &p.name;
                    if p.optional {
                        deser_lines.push(crate::template_env::render(
                            "function_named_binding.rs.jinja",
                            minijinja::context! {
                                mode => "optional",
                                binding_name => binding_ty,
                                core_import => core_import,
                                type_name => name,
                            },
                        ));
                    } else if promoted || (idx == func.params.len() - 1 && is_default_config_func) {
                        deser_lines.push(crate::template_env::render(
                            "function_named_binding.rs.jinja",
                            minijinja::context! {
                                mode => "default",
                                binding_name => binding_ty,
                                core_import => core_import,
                                type_name => name,
                            },
                        ));
                    } else {
                        deser_lines.push(crate::template_env::render(
                            "function_named_binding.rs.jinja",
                            minijinja::context! {
                                mode => "required",
                                binding_name => binding_ty,
                                core_import => core_import,
                                type_name => name,
                            },
                        ));
                    }
                }
            }
        }
    }
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

    let body = if can_delegate || serde_recoverable {
        let call_args = if serde_recoverable {
            generators::gen_call_args_with_let_bindings(&func.params, opaque_types)
        } else {
            generators::gen_call_args(&func.params, opaque_types)
        };
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
                crate::template_env::render(
                    "function_async_body.rs.jinja",
                    minijinja::context! {
                        core_call => &core_call,
                        wrap => &wrap,
                        has_error => true,
                    },
                )
            } else {
                crate::template_env::render(
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
            crate::template_env::render(
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
            crate::template_env::render(
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
    crate::template_env::render(
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

/// Returns true if a non-delegatable Magnus function/method can be recovered via serde
/// JSON-roundtrip on its params: every Named non-opaque param can be deserialized from a
/// String, and every sanitized Vec<String> param has `original_type` set.  Requires the
/// function to return Result (or be async, which wraps in Result via Runtime::new()) so the
/// generated `?` operator works.
fn magnus_serde_recoverable(func: &FunctionDef, opaque_types: &AHashSet<String>) -> bool {
    if func.error_type.is_none() && !func.is_async {
        return false;
    }
    if !alef_codegen::shared::is_delegatable_return(&func.return_type) {
        return false;
    }
    func.params.iter().all(|p| {
        // Sanitized Vec<String> originally Vec<tuple>: recoverable via JSON-decode-each.
        if p.sanitized {
            return p.original_type.is_some()
                && matches!(&p.ty, TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::String));
        }
        match &p.ty {
            // Named non-opaque: serde JSON-roundtrip handles both ref and non-ref cases.
            TypeRef::Named(n) if !opaque_types.contains(n.as_str()) => true,
            // Otherwise must be plain delegatable (no Named ref blockers since they're handled
            // above).
            _ => alef_codegen::shared::is_delegatable_param(&p.ty, opaque_types),
        }
    })
}

/// Generate Magnus serde let-bindings that produce `{name}_core: core::Type` so the shared
/// `gen_call_args_with_let_bindings` can emit `&{name}_core` for is_ref Named params.
///
/// Handles four cases for Named non-opaque params:
/// 1. `optional=true`: parameter is `Option<magnus::Value>` — bind as `Option<CoreType>`.
/// 2. `optional=false` but promoted (follows an optional param): parameter is also
///    `Option<magnus::Value>` due to `function_params` promotion — bind as `CoreType`,
///    falling back to `Default::default()` when the caller passes `nil`/omits the arg.
/// 3. `optional=false` and not promoted, but last param is default config: parameter is
///    `Option<magnus::Value>` from scan_args — bind as `CoreType`, falling back to
///    `Default::default()` when the caller omits the arg.
/// 4. `optional=false` and not promoted: parameter is `magnus::Value` — bind as `CoreType`.
fn magnus_serde_let_bindings(
    params: &[alef_core::ir::ParamDef],
    opaque_types: &AHashSet<String>,
    core_import: &str,
    _mapper: &MagnusMapper,
    is_default_config_func: bool,
) -> Vec<String> {
    let err = "magnus::Error::new(unsafe { Ruby::get_unchecked() }.exception_runtime_error(), e.to_string())";
    let mut out = Vec::new();
    for (idx, p) in params.iter().enumerate() {
        let promoted = alef_codegen::shared::is_promoted_optional(params, idx);
        let is_last = idx == params.len() - 1;
        let is_last_config = is_last && is_default_config_func;
        match &p.ty {
            TypeRef::Named(name) if !opaque_types.contains(name.as_str()) => {
                if p.optional {
                    out.push(crate::template_env::render(
                        "function_serde_named_binding.rs.jinja",
                        minijinja::context! {
                            mode => "optional",
                            name => &p.name,
                            core_import => core_import,
                            type_name => name,
                            error_expr => err,
                        },
                    ));
                } else if promoted || is_last_config {
                    out.push(crate::template_env::render(
                        "function_serde_named_binding.rs.jinja",
                        minijinja::context! {
                            mode => "default",
                            name => &p.name,
                            core_import => core_import,
                            type_name => name,
                            error_expr => err,
                        },
                    ));
                } else {
                    out.push(crate::template_env::render(
                        "function_serde_named_binding.rs.jinja",
                        minijinja::context! {
                            mode => "required",
                            name => &p.name,
                            core_import => core_import,
                            type_name => name,
                            error_expr => err,
                        },
                    ));
                }
            }
            TypeRef::Vec(inner)
                if matches!(inner.as_ref(), TypeRef::String | TypeRef::Char) && p.is_ref && !p.sanitized =>
            {
                // Non-sanitized Vec<String> passed by ref: core expects &[&str], so create refs vec.
                if p.optional {
                    out.push(crate::template_env::render(
                        "function_vec_refs_binding.rs.jinja",
                        minijinja::context! {
                            name => &p.name,
                            optional => true,
                        },
                    ));
                } else {
                    out.push(crate::template_env::render(
                        "function_vec_refs_binding.rs.jinja",
                        minijinja::context! {
                            name => &p.name,
                            optional => false,
                        },
                    ));
                }
            }
            TypeRef::Vec(inner)
                if matches!(inner.as_ref(), TypeRef::String) && p.sanitized && p.original_type.is_some() =>
            {
                if p.optional {
                    out.push(crate::template_env::render(
                        "function_sanitized_vec_binding.rs.jinja",
                        minijinja::context! {
                            name => &p.name,
                            optional => true,
                            error_expr => err,
                        },
                    ));
                } else {
                    out.push(crate::template_env::render(
                        "function_sanitized_vec_binding.rs.jinja",
                        minijinja::context! {
                            name => &p.name,
                            optional => false,
                            error_expr => err,
                        },
                    ));
                }
            }
            TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::Named(_)) => {
                // Generic Vec<T> where T is a struct type (e.g., Vec<BatchFileItem>):
                // The parameter is already a typed Vec<wrapper>; convert each wrapper
                // element into the core type via the generated `From<wrapper> for core` impl.
                if let TypeRef::Named(name) = inner.as_ref() {
                    let core_inner_ty = format!("{core_import}::{name}");
                    let vec_ty = format!("Vec<{core_inner_ty}>");
                    if p.optional {
                        out.push(crate::template_env::render(
                            "function_named_vec_binding.rs.jinja",
                            minijinja::context! {
                                name => &p.name,
                                vec_ty => &vec_ty,
                                optional => true,
                            },
                        ));
                    } else {
                        out.push(crate::template_env::render(
                            "function_named_vec_binding.rs.jinja",
                            minijinja::context! {
                                name => &p.name,
                                vec_ty => &vec_ty,
                                optional => false,
                            },
                        ));
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// Generate an async free function binding for Magnus (block on runtime).
pub(super) fn gen_async_function(
    func: &FunctionDef,
    mapper: &MagnusMapper,
    opaque_types: &AHashSet<String>,
    mutex_types: &AHashSet<String>,
    core_import: &str,
    api: &ApiSurface,
) -> String {
    let is_default_config_func = last_param_is_default_struct(func, api);
    let variadic = needs_variadic_arity(&func.params) || is_default_config_func;

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
    // Async functions always return Result because Runtime::new() can fail, even when the core
    // function itself has no error type.
    let return_annotation = mapper.wrap_return(&return_type, true);

    let can_delegate = alef_codegen::shared::can_auto_delegate_function(func, opaque_types);
    let serde_recoverable = !can_delegate && magnus_serde_recoverable(func, opaque_types);

    let mut deser_lines = Vec::new();
    if serde_recoverable {
        deser_lines.extend(magnus_serde_let_bindings(
            &func.params,
            opaque_types,
            core_import,
            mapper,
            is_default_config_func,
        ));
    } else {
        for (idx, p) in func.params.iter().enumerate() {
            let promoted = alef_codegen::shared::is_promoted_optional(&func.params, idx);
            if let TypeRef::Named(name) = &p.ty {
                if !opaque_types.contains(name.as_str()) {
                    let binding_ty = &p.name;
                    if p.optional {
                        deser_lines.push(crate::template_env::render(
                            "function_named_binding.rs.jinja",
                            minijinja::context! {
                                mode => "optional",
                                binding_name => binding_ty,
                                core_import => core_import,
                                type_name => name,
                            },
                        ));
                    } else if promoted || (idx == func.params.len() - 1 && is_default_config_func) {
                        deser_lines.push(crate::template_env::render(
                            "function_named_binding.rs.jinja",
                            minijinja::context! {
                                mode => "default",
                                binding_name => binding_ty,
                                core_import => core_import,
                                type_name => name,
                            },
                        ));
                    } else {
                        deser_lines.push(crate::template_env::render(
                            "function_named_binding.rs.jinja",
                            minijinja::context! {
                                mode => "required",
                                binding_name => binding_ty,
                                core_import => core_import,
                                type_name => name,
                            },
                        ));
                    }
                }
            }
        }
    }
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

    let body = if can_delegate || serde_recoverable {
        let call_args = if serde_recoverable {
            generators::gen_call_args_with_let_bindings(&func.params, opaque_types)
        } else {
            generators::gen_call_args(&func.params, opaque_types)
        };
        let core_fn_path = {
            let path = func.rust_path.replace('-', "_");
            if path.starts_with(core_import) {
                path
            } else {
                format!("{core_import}::{}", func.name)
            }
        };
        let core_call = format!("{core_fn_path}({call_args})");
        let result_wrap = generators::wrap_return_with_mutex_mapped(
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
            crate::template_env::render(
                "function_async_body.rs.jinja",
                minijinja::context! {
                    core_call => &core_call,
                    wrap => &result_wrap,
                    has_error => true,
                },
            )
        } else {
            crate::template_env::render(
                "function_async_body.rs.jinja",
                minijinja::context! {
                    core_call => &core_call,
                    wrap => &result_wrap,
                    has_error => false,
                },
            )
        }
    } else {
        gen_magnus_unimplemented_body(
            &func.return_type,
            &format!("{}_async", func.name),
            func.error_type.is_some(),
        )
    };
    // Add #[allow(unused_variables)] to functions with unimplemented bodies to suppress warnings for unused params
    let allow_attr = if !can_delegate && !serde_recoverable {
        "#[allow(unused_variables)]\n"
    } else {
        ""
    };
    let name = format!("{}_async", func.name);
    crate::template_env::render(
        "function_wrapper.rs.jinja",
        minijinja::context! {
            allow_attr => allow_attr,
            name => &name,
            params => &params,
            return_annotation => &return_annotation,
            scan_args_prologue => &scan_args_prologue,
            deser_preamble => &deser_preamble,
            body => &body,
        },
    )
}

/// Generate a type-appropriate unimplemented body for Magnus (no todo!()).
pub(super) fn gen_magnus_unimplemented_body(
    return_type: &alef_core::ir::TypeRef,
    fn_name: &str,
    has_error: bool,
) -> String {
    use alef_core::ir::TypeRef;
    let err_msg = format!("Not implemented: {fn_name}");
    if has_error {
        crate::template_env::render(
            "function_unimplemented_error.rs.jinja",
            minijinja::context! {
                message => &err_msg,
            },
        )
    } else {
        match return_type {
            TypeRef::Unit => "()".to_string(),
            TypeRef::String | TypeRef::Char | TypeRef::Path => crate::template_env::render(
                "function_unimplemented_string.rs.jinja",
                minijinja::context! {
                    name => fn_name,
                },
            ),
            TypeRef::Bytes => "Vec::new()".to_string(),
            TypeRef::Primitive(p) => match p {
                alef_core::ir::PrimitiveType::Bool => "false".to_string(),
                _ => "0".to_string(),
            },
            TypeRef::Optional(_) => "None".to_string(),
            TypeRef::Vec(_) => "Vec::new()".to_string(),
            TypeRef::Map(_, _) => "Default::default()".to_string(),
            TypeRef::Duration => "0u64".to_string(),
            TypeRef::Named(_) | TypeRef::Json => crate::template_env::render(
                "function_unimplemented_panic.rs.jinja",
                minijinja::context! {
                    name => fn_name,
                },
            ),
        }
    }
}

/// Generate the module initialization function.
#[allow(clippy::too_many_arguments)]
pub(super) fn gen_module_init(
    module_name: &str,
    api: &ApiSurface,
    config: &ResolvedCrateConfig,
    exclude_functions: &std::collections::HashSet<&str>,
    exclude_types: &std::collections::HashSet<&str>,
    streaming_methods_by_owner: &std::collections::HashMap<String, Vec<String>>,
    streaming_iterator_registrations: &[String],
    streaming_method_registrations: &std::collections::HashMap<String, Vec<String>>,
) -> String {
    let mut lines = vec![
        "#[magnus::init]".to_string(),
        "fn ruby_init(ruby: &Ruby) -> Result<(), Error> {".to_string(),
        crate::template_env::render(
            "module_define.rs.jinja",
            minijinja::context! {
                module_name => module_name,
            },
        ),
        "".to_string(),
        "    // Ensure JSON library is loaded for Hash#to_json".to_string(),
        "    let _ = ruby.eval::<magnus::Value>(\"require \\\"json\\\"\");".to_string(),
        "".to_string(),
    ];

    // Custom registrations (before generated ones)
    if let Some(reg) = config.custom_registrations.for_language(Language::Ruby) {
        for class in &reg.classes {
            lines.push(crate::template_env::render(
                "module_class_define.rs.jinja",
                minijinja::context! {
                    binding => "_class",
                    class_name => class,
                },
            ));
        }
        for func in &reg.functions {
            lines.push(crate::template_env::render(
                "module_function_register.rs.jinja",
                minijinja::context! {
                    ruby_name => func,
                    function_name => func,
                    arity => 0,
                },
            ));
        }
        lines.push("".to_string());
    }

    for typ in api.types.iter().filter(|typ| !typ.is_trait) {
        if exclude_types.contains(typ.name.as_str()) {
            continue;
        }
        let class_used = (!typ.is_opaque && !typ.fields.is_empty()) || typ.methods.iter().any(|m| !m.is_static);
        let binding = if class_used { "class" } else { "_class" };
        lines.push(crate::template_env::render(
            "module_class_define.rs.jinja",
            minijinja::context! {
                binding => binding,
                class_name => &typ.name,
            },
        ));

        if !typ.is_opaque && !typ.fields.is_empty() {
            // Always register the constructor as variadic (-1) since the impl now uses a
            // hash-based kwargs constructor regardless of field count. This keeps Ruby
            // callers consistent: every `Type.new(field: ...)` works whether the type has
            // 3 fields or 30.
            lines.push(crate::template_env::render(
                "module_class_singleton_method_register.rs.jinja",
                minijinja::context! {
                    ruby_name => "new",
                    type_name => &typ.name,
                    function_name => "new",
                    arity => -1,
                },
            ));
        }

        if !typ.is_opaque {
            for field in &typ.fields {
                // Skip thread-unsafe fields (e.g., VisitorHandle) that cannot be used in Magnus methods
                if is_thread_unsafe_field(field) {
                    continue;
                }
                lines.push(crate::template_env::render(
                    "module_class_method_register.rs.jinja",
                    minijinja::context! {
                        ruby_name => &field.name,
                        type_name => &typ.name,
                        function_name => &field.name,
                        arity => 0,
                    },
                ));
            }
            // Register to_s for structs that have a `content: String` or `content: Option<String>` field.
            if super::classes::has_content_string_field(typ) {
                lines.push(crate::template_env::render(
                    "module_class_method_register.rs.jinja",
                    minijinja::context! {
                        ruby_name => "to_s",
                        type_name => &typ.name,
                        function_name => "to_s",
                        arity => 0,
                    },
                ));
            }
        }

        let streaming_owner_methods = streaming_methods_by_owner
            .get(typ.name.as_str())
            .map(|v| v.as_slice())
            .unwrap_or(&[]);

        for method in &typ.methods {
            if !method.is_static {
                // Skip apply_update methods: they mutate self without returning a value,
                // which is incompatible with Magnus's method! macro which requires RubyMethod traits.
                // Callers can use from_update instead.
                if method.name == "apply_update" {
                    continue;
                }

                // Skip &mut self methods: Magnus's method! macro doesn't support mutable receivers.
                // These methods mutate the wrapper in place, which isn't compatible with Ruby's
                // object model. Callers should use builder patterns or from_* constructors instead.
                if matches!(method.receiver, Some(ReceiverKind::RefMut)) {
                    continue;
                }

                // Streaming methods register via streaming_method_registrations below.
                if streaming_owner_methods.contains(&method.name) {
                    continue;
                }

                let method_name = if method.is_async {
                    format!("{}_async", method.name)
                } else {
                    method.name.clone()
                };
                let param_count = method.params.len();
                lines.push(crate::template_env::render(
                    "module_class_method_register.rs.jinja",
                    minijinja::context! {
                        ruby_name => &method_name,
                        type_name => &typ.name,
                        function_name => &method_name,
                        arity => param_count,
                    },
                ));
            }
        }

        // Append streaming method registrations (e.g. chat_stream → DefaultClient::chat_stream)
        // for this owner type. These are emitted by the streaming module.
        if let Some(regs) = streaming_method_registrations.get(typ.name.as_str()) {
            for reg in regs {
                lines.push(reg.clone());
            }
        }

        lines.push("".to_string());
    }

    // Register iterator classes (e.g. ChatStreamIterator) at module scope.
    if !streaming_iterator_registrations.is_empty() {
        lines.extend(streaming_iterator_registrations.iter().cloned());
        lines.push("".to_string());
    }

    for func in &api.functions {
        if super::is_reserved_fn(&func.name) || exclude_functions.contains(func.name.as_str()) {
            continue;
        }
        // Functions with a trait_bridge param use fixed-arity signatures, while
        // options_field bindings use variadic arity. For bridge_param, register fixed arity
        // since those functions don't use scan_args. For options_field, register variadic
        // (-1) since the generated body uses scan_args to unpack arguments.
        let has_bridge_param = crate::trait_bridge::find_bridge_param(func, &config.trait_bridges).is_some();
        let has_options_field_binding =
            crate::trait_bridge::find_options_field_binding(func, &config.trait_bridges).is_some();

        let is_default_config_func = last_param_is_default_struct(func, api);

        let param_count: i32 = if has_options_field_binding {
            // options_field binding functions use variadic arity with scan_args
            -1
        } else if has_bridge_param {
            // bridge_param functions use fixed arity
            func.params.len() as i32
        } else if needs_variadic_arity(&func.params) || is_default_config_func {
            // Functions with optional params OR default-config last param use variadic arity
            -1
        } else {
            // Functions with only required params use fixed arity
            func.params.len() as i32
        };
        if func.is_async {
            // Register both sync (blocking) and async variants
            lines.push(crate::template_env::render(
                "module_function_register.rs.jinja",
                minijinja::context! {
                    ruby_name => &func.name,
                    function_name => &func.name,
                    arity => param_count,
                },
            ));
            let async_name = format!("{}_async", func.name);
            lines.push(crate::template_env::render(
                "module_function_register.rs.jinja",
                minijinja::context! {
                    ruby_name => &async_name,
                    function_name => &async_name,
                    arity => param_count,
                },
            ));
        } else {
            lines.push(crate::template_env::render(
                "module_function_register.rs.jinja",
                minijinja::context! {
                    ruby_name => &func.name,
                    function_name => &func.name,
                    arity => param_count,
                },
            ));
        }
    }

    // Register trait bridge entry points: pub fn register_xxx(rb_obj, name) -> Result<...>
    // is emitted by the trait_bridge generator; surface it on the Ruby module here.
    for bridge_cfg in &config.trait_bridges {
        if bridge_cfg.exclude_languages.iter().any(|s| s == "ruby") {
            continue;
        }
        if let Some(register_fn) = bridge_cfg.register_fn.as_deref() {
            lines.push(crate::template_env::render(
                "module_function_register.rs.jinja",
                minijinja::context! {
                    ruby_name => register_fn,
                    function_name => register_fn,
                    arity => 2,
                },
            ));
        }
    }

    lines.push("".to_string());
    lines.push("    Ok(())".to_string());
    lines.push("}".to_string());

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use alef_core::config::new_config::NewAlefConfig;
    use alef_core::ir::{FunctionDef, ParamDef, PrimitiveType, TypeRef};

    fn resolved_one(toml: &str) -> ResolvedCrateConfig {
        let cfg: NewAlefConfig = toml::from_str(toml).unwrap();
        cfg.resolve().unwrap().remove(0)
    }

    fn make_config() -> ResolvedCrateConfig {
        resolved_one(
            r#"
[workspace]
languages = ["ruby"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.ruby]
gem_name = "test_lib"
"#,
        )
    }

    fn simple_func(name: &str, error: bool) -> FunctionDef {
        FunctionDef {
            name: name.to_string(),
            rust_path: format!("test_lib::{name}"),
            original_rust_path: String::new(),
            params: vec![ParamDef {
                name: "input".to_string(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                sanitized: false,
                typed_default: None,
                is_ref: false,
                is_mut: false,
                newtype_wrapper: None,
                original_type: None,
            }],
            return_type: TypeRef::String,
            is_async: false,
            error_type: if error { Some("Error".to_string()) } else { None },
            doc: String::new(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }
    }

    #[test]
    fn gen_function_emits_fn_name() {
        let func = simple_func("process", false);
        let mapper = crate::type_map::MagnusMapper;
        let api = alef_core::ir::ApiSurface {
            crate_name: "test_lib".to_string(),
            version: "0.1.0".to_string(),
            types: vec![],
            functions: vec![],
            enums: vec![],
            errors: vec![],
            excluded_type_paths: ::std::collections::HashMap::new(),
        };
        let code = gen_function(
            &func,
            &mapper,
            &Default::default(),
            &Default::default(),
            "test_lib",
            &api,
        );
        assert!(code.contains("fn process("), "must emit function name");
        assert!(code.contains("input: String"), "must include typed param");
    }

    #[test]
    fn gen_function_with_error_wraps_result() {
        let func = simple_func("process", true);
        let mapper = crate::type_map::MagnusMapper;
        let api = alef_core::ir::ApiSurface {
            crate_name: "test_lib".to_string(),
            version: "0.1.0".to_string(),
            types: vec![],
            functions: vec![],
            enums: vec![],
            errors: vec![],
            excluded_type_paths: ::std::collections::HashMap::new(),
        };
        let code = gen_function(
            &func,
            &mapper,
            &Default::default(),
            &Default::default(),
            "test_lib",
            &api,
        );
        assert!(code.contains("Result<"), "error function must return Result");
    }

    #[test]
    fn gen_module_init_emits_magnus_init_attr() {
        let config = make_config();
        let api = alef_core::ir::ApiSurface {
            crate_name: "test_lib".to_string(),
            version: "0.1.0".to_string(),
            types: vec![],
            functions: vec![],
            enums: vec![],
            errors: vec![],
            excluded_type_paths: ::std::collections::HashMap::new(),
        };
        let code = gen_module_init(
            "TestLib",
            &api,
            &config,
            &Default::default(),
            &Default::default(),
            &Default::default(),
            &[],
            &Default::default(),
        );
        assert!(code.contains("#[magnus::init]"), "must emit #[magnus::init]");
        assert!(code.contains("fn ruby_init(ruby: &Ruby)"), "must emit init fn");
        assert!(code.contains("define_module(\"TestLib\")"), "must define the module");
    }

    #[test]
    fn needs_variadic_arity_detects_optional_params() {
        let required = ParamDef {
            name: "x".to_string(),
            ty: TypeRef::Primitive(PrimitiveType::U32),
            optional: false,
            default: None,
            sanitized: false,
            typed_default: None,
            is_ref: false,
            is_mut: false,
            newtype_wrapper: None,
            original_type: None,
        };
        let optional = ParamDef {
            optional: true,
            ..required.clone()
        };
        assert!(
            !needs_variadic_arity(std::slice::from_ref(&required)),
            "required-only: no variadic"
        );
        assert!(needs_variadic_arity(&[optional]), "optional param: needs variadic");
    }
}
