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

/// ~keep The async wrapper's return type is fallible unconditionally, independent of whether the
/// core function declares an error type: every emitted body runs a tokio runtime without the GVL
/// and closes with `Ok(..)` (see `function_async_body.rs.jinja`), so the
/// delegable path — the authority on the signature — only compiles inside `Result<T, Error>`. The
/// unimplemented body must be selected against this same fact, or it emits `compile_error!`/a bare
/// `()` into a `Result` slot for a non-delegable function with no declared error type.
pub(in crate::backends::magnus::gen_bindings) const ASYNC_RETURN_IS_FALLIBLE: bool = true;

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
            if let TypeRef::Named(name) = ty
                && !opaque_types.contains(name.as_str())
            {
                return "magnus::Value".to_string();
            }
            mapper.map_type(ty)
        })
    };
    let return_type = mapper.map_type(&func.return_type);
    let return_annotation = mapper.wrap_return(&return_type, ASYNC_RETURN_IS_FALLIBLE);

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
            } else if let TypeRef::Vec(inner) = &p.ty
                && let TypeRef::Named(name) = inner.as_ref()
                && !opaque_types.contains(name.as_str())
            {
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
            ASYNC_RETURN_IS_FALLIBLE,
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

#[cfg(test)]
mod unimplemented_body_matches_signature_tests {
    use super::gen_async_function;
    use crate::backends::magnus::type_map::MagnusMapper;
    use crate::core::ir::{ApiSurface, CoreWrapper, FunctionDef, ParamDef, TypeRef};

    fn empty_api() -> ApiSurface {
        ApiSurface {
            crate_name: "test_lib".to_string(),
            version: "0.1.0".to_string(),
            types: vec![],
            functions: vec![],
            enums: vec![],
            errors: vec![],
            excluded_type_paths: ::std::collections::BTreeMap::new(),
            excluded_trait_names: ::std::collections::HashSet::new(),
            services: vec![],
            handler_contracts: vec![],
            unsupported_public_items: Vec::new(),
        }
    }

    /// A function the auto-delegation check rejects: the single param is `sanitized`, which fails
    /// `can_auto_delegate_function` and — because it is not a `Vec<String>` — also fails
    /// `magnus_serde_recoverable`, so generation falls through to the unimplemented body. ~keep
    fn non_delegable_async_func(name: &str, return_type: TypeRef, error: bool) -> FunctionDef {
        FunctionDef {
            name: name.to_string(),
            rust_path: format!("test_lib::{name}"),
            original_rust_path: String::new(),
            params: vec![ParamDef {
                name: "input".to_string(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                sanitized: true,
                typed_default: None,
                is_ref: false,
                is_mut: false,
                newtype_wrapper: None,
                original_type: Some("Secret".to_string()),
                map_is_ahash: false,
                map_key_is_cow: false,
                vec_inner_is_ref: false,
                map_is_btree: false,
                core_wrapper: CoreWrapper::None,
            }],
            return_type,
            is_async: true,
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
            version: Default::default(),
        }
    }

    fn generate(func: &FunctionDef) -> String {
        gen_async_function(
            func,
            &MagnusMapper,
            &Default::default(),
            &Default::default(),
            "test_lib",
            &empty_api(),
        )
    }

    const EXPECTED_ERR_BODY: &str = concat!(
        "Err(magnus::Error::new(unsafe { Ruby::get_unchecked() }.exception_runtime_error(), ",
        "\"Not implemented: process_async\"))"
    );

    #[test]
    fn unit_return_without_error_type_emits_err_not_bare_unit() {
        let code = generate(&non_delegable_async_func("process", TypeRef::Unit, false));
        assert!(
            code.contains("fn process_async(input: String) -> Result<(), Error> {"),
            "async wrapper signature must stay fallible, got: {code}"
        );
        assert!(
            code.contains(EXPECTED_ERR_BODY),
            "body must be the Err arm that fits `Result<(), Error>`, got: {code}"
        );
        assert!(
            !code.contains("compile_error!"),
            "an Err arm fits this signature, so compile_error! must not be selected: {code}"
        );
    }

    #[test]
    fn value_return_without_error_type_emits_err_not_compile_error() {
        let code = generate(&non_delegable_async_func("process", TypeRef::String, false));
        assert!(
            code.contains("fn process_async(input: String) -> Result<String, Error> {"),
            "async wrapper signature must stay fallible, got: {code}"
        );
        assert!(
            code.contains(EXPECTED_ERR_BODY),
            "body must be the Err arm that fits `Result<String, Error>`, got: {code}"
        );
        assert!(
            !code.contains("compile_error!"),
            "an Err arm fits this signature, so compile_error! must not be selected: {code}"
        );
    }

    #[test]
    fn value_return_with_error_type_still_emits_err() {
        let code = generate(&non_delegable_async_func("process", TypeRef::String, true));
        assert!(
            code.contains("fn process_async(input: String) -> Result<String, Error> {"),
            "async wrapper signature must stay fallible, got: {code}"
        );
        assert!(
            code.contains(EXPECTED_ERR_BODY),
            "declared-error case must keep emitting the Err arm, got: {code}"
        );
        assert!(
            !code.contains("compile_error!"),
            "declared-error case must not regress into compile_error!: {code}"
        );
    }
}
