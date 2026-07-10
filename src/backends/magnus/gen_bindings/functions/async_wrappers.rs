use super::scan_args_defaults::{
    gen_scan_args_prologue_with_defaults, last_param_is_default_struct, needs_variadic_arity,
};
use super::serde_bindings::{
    magnus_ahash_pre_call_bindings, magnus_call_args_with_ahash, magnus_serde_let_bindings, magnus_serde_recoverable,
};
use super::sync_wrappers::gen_magnus_unimplemented_body;
use crate::backends::magnus::type_map::MagnusMapper;
use crate::codegen::generators;
use crate::codegen::shared::function_params;
use crate::codegen::type_mapper::TypeMapper;
use crate::core::ir::{ApiSurface, FunctionDef, TypeRef};
use ahash::AHashSet;

/// Generate an async free function binding for Magnus (block on runtime).
pub(in crate::backends::magnus::gen_bindings) fn gen_async_function(
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
    let return_annotation = mapper.wrap_return(&return_type, true);

    let can_delegate = crate::codegen::shared::can_auto_delegate_function(func, opaque_types);
    let serde_recoverable = !can_delegate && magnus_serde_recoverable(func, opaque_types, true);

    let needs_vec_named_let_binding = func.params.iter().any(|p| match &p.ty {
        TypeRef::Vec(inner) => matches!(inner.as_ref(), TypeRef::Named(name) if !opaque_types.contains(name.as_str())),
        _ => false,
    });

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

    let ahash_bindings = magnus_ahash_pre_call_bindings(&func.params);
    deser_lines.extend(ahash_bindings);

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
            crate::backends::magnus::template_env::render(
                "function_async_body.rs.jinja",
                minijinja::context! {
                    core_call => &core_call,
                    wrap => &result_wrap,
                    has_error => true,
                },
            )
        } else {
            crate::backends::magnus::template_env::render(
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
    crate::backends::magnus::template_env::render(
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
