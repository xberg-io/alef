//! NAPI-RS function and method code generation.

mod adapter_wrappers;
mod call_args;
mod conversion_bindings;
mod return_wrapping;

pub(super) use adapter_wrappers::{gen_adapter_wrapper, gen_tokio_runtime};
pub(super) use call_args::{
    core_prim_str, napi_apply_primitive_casts_to_call_args, napi_gen_call_args, needs_napi_cast,
};
use call_args::{is_bytes_param, needs_vec_f32_conversion};
use conversion_bindings::{gen_napi_buffer_conversion_bindings, gen_vec_f32_conversion_bindings};
pub(super) use return_wrapping::{napi_wrap_return, napi_wrap_return_fn};

use crate::codegen::generators::{self, RustBindingConfig};
use crate::codegen::naming::to_node_name;
use crate::codegen::shared::function_params;
use crate::codegen::type_mapper::TypeMapper;
use crate::core::ir::{FunctionDef, TypeRef};
use ahash::AHashSet;

use crate::backends::napi::type_map::NapiMapper;

#[allow(clippy::too_many_arguments)]
pub(super) fn gen_function(
    func: &FunctionDef,
    mapper: &NapiMapper,
    cfg: &RustBindingConfig,
    opaque_types: &AHashSet<String>,
    default_types: &AHashSet<String>,
    prefix: &str,
    capsule_types: &std::collections::HashMap<String, crate::core::config::NodeCapsuleTypeConfig>,
    mutex_types: &AHashSet<String>,
) -> String {
    let augmented_params: Vec<crate::core::ir::ParamDef> = func
        .params
        .iter()
        .map(|p| {
            let mut p2 = p.clone();
            if !p2.optional {
                if let TypeRef::Named(n) = &p2.ty {
                    if default_types.contains(n.as_str()) && !opaque_types.contains(n.as_str()) {
                        p2.optional = true;
                    }
                }
            }
            p2
        })
        .collect();
    let params = function_params(&augmented_params, &|ty| {
        if let TypeRef::Named(n) = ty {
            if capsule_types.contains_key(n.as_str()) {
                if let Some(capsule_cfg) = capsule_types.get(n.as_str()) {
                    return capsule_cfg.from_module.clone();
                }
            }
            if opaque_types.contains(n.as_str()) {
                return format!("&{prefix}{n}");
            }
        }
        mapper.map_type(ty)
    });
    let default_coerce_prefix: String = augmented_params
        .iter()
        .zip(func.params.iter())
        .enumerate()
        .filter_map(|(idx, (aug, orig))| {
            if aug.optional && !orig.optional && !crate::codegen::shared::is_promoted_optional(&func.params, idx) {
                let is_named_non_opaque = matches!(&orig.ty,
                    TypeRef::Named(n) if !opaque_types.contains(n.as_str())
                );
                if is_named_non_opaque {
                    return None;
                }
                let mut_kw = if orig.is_mut { "mut " } else { "" };
                Some(format!(
                    "    let {}{} = {}.unwrap_or_default();\n",
                    mut_kw, orig.name, orig.name
                ))
            } else {
                None
            }
        })
        .collect();
    let return_type = mapper.map_type(&func.return_type);
    let return_annotation = mapper.wrap_return(&return_type, func.error_type.is_some());

    let js_name = to_node_name(&func.name);
    let js_name_attr = if js_name != func.name {
        format!("(js_name = \"{}\")", js_name)
    } else {
        String::new()
    };

    let core_import = cfg.core_import;
    let core_fn_path = {
        let path = func.rust_path.replace('-', "_");
        if path.starts_with(core_import) {
            path
        } else {
            format!("{core_import}::{}", func.name)
        }
    };

    let use_let_bindings = generators::has_named_params(&func.params, opaque_types)
        || func.params.iter().any(|p| needs_vec_f32_conversion(&p.ty))
        || func.params.iter().any(|p| is_bytes_param(&p.ty));
    let call_args = if use_let_bindings {
        let base_args = generators::gen_call_args_with_let_bindings_mutex(&func.params, opaque_types, mutex_types);
        napi_apply_primitive_casts_to_call_args(&base_args, &func.params)
    } else {
        napi_gen_call_args(&func.params, opaque_types)
    };

    let can_delegate_fn = crate::codegen::shared::can_auto_delegate_function(func, opaque_types)
        || can_delegate_with_named_let_bindings(func, opaque_types);

    let err_conv = ".map_err(|e| napi::Error::new(napi::Status::GenericFailure, e.to_string()))";

    let async_kw = if func.is_async { "async " } else { "" };

    let body = if !can_delegate_fn {
        if cfg.has_serde && use_let_bindings && func.error_type.is_some() {
            let serde_bindings =
                generators::gen_serde_let_bindings(&func.params, opaque_types, core_import, err_conv, "    ");
            let vec_str_bindings: String = func.params.iter().filter(|p| {
                p.is_ref && p.vec_inner_is_ref && matches!(&p.ty, TypeRef::Vec(inner) if matches!(inner.as_ref(), TypeRef::String | TypeRef::Char))
            }).map(|p| {
                format!("let {}_refs: Vec<&str> = {}.iter().map(|s| s.as_str()).collect();\n    ", p.name, p.name)
            }).collect();
            let core_call = format!("{core_fn_path}({call_args})");
            let await_kw = if func.is_async { ".await" } else { "" };

            if matches!(func.return_type, TypeRef::Unit) {
                format!("{vec_str_bindings}{serde_bindings}{core_call}{await_kw}{err_conv}?;\n    Ok(())")
            } else {
                let wrapped = napi_wrap_return_fn(
                    "val",
                    &func.return_type,
                    opaque_types,
                    func.returns_ref,
                    prefix,
                    Some(capsule_types),
                    mutex_types,
                );
                if wrapped == "val" {
                    format!("{vec_str_bindings}{serde_bindings}{core_call}{await_kw}{err_conv}")
                } else {
                    format!("{vec_str_bindings}{serde_bindings}{core_call}{await_kw}.map(|val| {wrapped}){err_conv}")
                }
            }
        } else {
            generators::gen_unimplemented_body(
                &func.return_type,
                &func.name,
                func.error_type.is_some(),
                cfg,
                &func.params,
                opaque_types,
            )
        }
    } else if func.is_async {
        let mut let_bindings = if use_let_bindings {
            generators::gen_named_let_bindings_with_augmented(
                &augmented_params,
                &func.params,
                opaque_types,
                core_import,
            )
        } else {
            String::new()
        };
        let_bindings.push_str(&gen_vec_f32_conversion_bindings(&func.params));
        let_bindings.push_str(&gen_napi_buffer_conversion_bindings(&func.params));
        let core_call = format!("{core_fn_path}({call_args})");
        let return_wrap = napi_wrap_return_fn(
            "result",
            &func.return_type,
            opaque_types,
            func.returns_ref,
            prefix,
            Some(capsule_types),
            mutex_types,
        );
        let return_type = mapper.map_type(&func.return_type);
        generators::gen_async_body(
            &core_call,
            cfg,
            func.error_type.is_some(),
            &return_wrap,
            false,
            &let_bindings,
            matches!(func.return_type, TypeRef::Unit),
            Some(&return_type),
        )
    } else {
        let core_call = format!("{core_fn_path}({call_args})");
        let mut let_bindings = if use_let_bindings {
            generators::gen_named_let_bindings_with_augmented(
                &augmented_params,
                &func.params,
                opaque_types,
                core_import,
            )
        } else {
            String::new()
        };
        let_bindings.push_str(&gen_vec_f32_conversion_bindings(&func.params));
        let_bindings.push_str(&gen_napi_buffer_conversion_bindings(&func.params));

        if func.error_type.is_some() {
            let wrapped = napi_wrap_return_fn(
                "val",
                &func.return_type,
                opaque_types,
                func.returns_ref,
                prefix,
                Some(capsule_types),
                mutex_types,
            );
            if wrapped == "val" {
                format!("{let_bindings}{core_call}{err_conv}")
            } else {
                format!("{let_bindings}{core_call}.map(|val| {wrapped}){err_conv}")
            }
        } else {
            format!(
                "{let_bindings}{}",
                napi_wrap_return_fn(
                    &core_call,
                    &func.return_type,
                    opaque_types,
                    func.returns_ref,
                    prefix,
                    Some(capsule_types),
                    mutex_types
                )
            )
        }
    };

    let mut attrs = String::new();
    let sanitized_doc =
        crate::codegen::doc_emission::sanitize_rust_idioms(&func.doc, crate::codegen::doc_emission::DocTarget::TsDoc);
    crate::codegen::doc_emission::emit_rustdoc(&mut attrs, &sanitized_doc, "");
    if func.params.len() > 7 {
        attrs.push_str("#[allow(clippy::too_many_arguments)]\n");
    }
    if func.error_type.is_some() {
        attrs.push_str("#[allow(clippy::missing_errors_doc)]\n");
    }
    let body = if default_coerce_prefix.is_empty() {
        body
    } else {
        format!("{}{}", default_coerce_prefix, body)
    };
    crate::backends::napi::template_env::render(
        "function_wrapper.jinja",
        minijinja::context! {
            attrs => attrs,
            js_name_attr => js_name_attr,
            async_kw => async_kw,
            func_name => &func.name,
            params => params,
            return_annotation => return_annotation,
            body => body,
        },
    )
}

fn can_delegate_with_named_let_bindings(func: &FunctionDef, opaque_types: &AHashSet<String>) -> bool {
    !func.sanitized
        && func
            .params
            .iter()
            .all(|p| !p.sanitized && crate::codegen::shared::is_delegatable_param(&p.ty, opaque_types))
        && crate::codegen::shared::is_delegatable_return(&func.return_type)
}

#[cfg(test)]
mod tests {
    use super::gen_tokio_runtime;

    /// gen_tokio_runtime produces a static runtime with an enlarged worker stack so deep
    /// consumer futures (e.g. an OCR pipeline) do not overflow the default ~2 MB stack (SIGBUS).
    #[test]
    fn gen_tokio_runtime_contains_runtime() {
        let result = gen_tokio_runtime();
        assert!(result.contains("TOKIO_RUNTIME") || result.contains("Runtime") || result.contains("tokio"));
        assert!(
            result.contains("thread_stack_size"),
            "worker pool must enlarge the stack:\n{result}"
        );
    }
}
