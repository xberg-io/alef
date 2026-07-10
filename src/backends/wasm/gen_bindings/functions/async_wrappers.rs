//! Async free-function wrapper generation for WASM.

use super::returns::{to_turbofish_from, type_has_default};
use crate::backends::wasm::type_map::WasmMapper;
use crate::codegen::generators;
use crate::codegen::type_mapper::TypeMapper;
use crate::core::ir::{ApiSurface, FunctionDef, TypeRef};
use ahash::AHashSet;

#[allow(clippy::too_many_arguments)]
pub(super) fn gen_async_free_function(
    input_dtos: &str,
    func: &FunctionDef,
    mapper: &WasmMapper,
    core_import: &str,
    opaque_types: &AHashSet<String>,
    mutex_types: &AHashSet<String>,
    api: &ApiSurface,
    params: &[String],
    return_type: &str,
    attrs: &str,
    js_name_attr: &str,
    return_annotation: &str,
    core_fn_path: &str,
) -> String {
    let has_named = crate::codegen::generators::has_named_params(&func.params, opaque_types);

    let async_params: Vec<String> = if has_named {
        func.params
            .iter()
            .map(|p| match &p.ty {
                TypeRef::Named(name) if !opaque_types.contains(name.as_str()) => {
                    let mapped_ty = if p.optional {
                        "Option<JsValue>".to_string()
                    } else {
                        "JsValue".to_string()
                    };
                    format!("{}: {}", p.name, mapped_ty)
                }
                _ => {
                    let ty = mapper.map_type(&p.ty);
                    let mapped_ty = if p.optional { format!("Option<{}>", ty) } else { ty };
                    format!("{}: {}", p.name, mapped_ty)
                }
            })
            .collect()
    } else {
        params.to_vec()
    };

    let mut serde_bindings = String::new();
    if has_named {
        for p in &func.params {
            if let TypeRef::Named(name) = &p.ty {
                if !opaque_types.contains(name.as_str()) {
                    let core_path = format!("{}::{}", core_import, name);
                    let err_conv = ".map_err(|e| JsValue::from_str(&e.to_string()))";
                    if p.optional {
                        serde_bindings.push_str(&crate::backends::wasm::template_env::render(
                            "serde_named_optional",
                            minijinja::context! {
                                param_name => &p.name,
                                core_path => &core_path,
                                err_conv => &err_conv,
                            },
                        ));
                        serde_bindings.push_str("    ");
                    } else {
                        let has_default = type_has_default(name, api);
                        serde_bindings.push_str(&crate::backends::wasm::template_env::render(
                            "serde_named_required",
                            minijinja::context! {
                                param_name => &p.name,
                                core_path => &core_path,
                                err_conv => &err_conv,
                                has_default => has_default,
                                is_mut => p.is_mut,
                            },
                        ));
                        serde_bindings.push_str("    ");
                    }
                }
            } else if let TypeRef::Vec(inner) = &p.ty
                && let TypeRef::Named(name) = inner.as_ref()
                && !opaque_types.contains(name.as_str())
            {
                let core_path = format!("{}::{}", core_import, name);
                let template_name = if p.optional {
                    "serde_vec_named_from_optional"
                } else {
                    "serde_vec_named_from_required"
                };
                serde_bindings.push_str(&crate::backends::wasm::template_env::render(
                    template_name,
                    minijinja::context! {
                        param_name => &p.name,
                        core_path => &core_path,
                    },
                ));
                serde_bindings.push_str("    ");
            }
        }
    }

    let let_bindings = serde_bindings;
    let call_args = if let_bindings.is_empty() {
        generators::gen_call_args(&func.params, opaque_types)
    } else {
        generators::gen_call_args_with_let_bindings(&func.params, opaque_types)
    };
    let core_call = format!("{core_fn_path}({call_args})");
    let return_expr = match &func.return_type {
        TypeRef::Vec(inner) => match inner.as_ref() {
            TypeRef::Named(n) if opaque_types.contains(n.as_str()) => {
                if mutex_types.contains(n.as_str()) {
                    format!(
                        "result.into_iter().map(|v| {} {{ inner: Arc::new(std::sync::Mutex::new(v)) }}).collect::<Vec<_>>()",
                        mapper.map_type(inner)
                    )
                } else {
                    format!(
                        "result.into_iter().map(|v| {} {{ inner: Arc::new(v) }}).collect::<Vec<_>>()",
                        mapper.map_type(inner)
                    )
                }
            }
            TypeRef::Named(_) => {
                let inner_mapped = mapper.map_type(inner);
                format!("result.into_iter().map({inner_mapped}::from).collect::<Vec<_>>()")
            }
            _ => "result".to_string(),
        },
        TypeRef::Named(n) if opaque_types.contains(n.as_str()) => {
            let prefixed = mapper.map_type(&func.return_type);
            if mutex_types.contains(n.as_str()) {
                format!("{prefixed} {{ inner: Arc::new(std::sync::Mutex::new(result)) }}")
            } else {
                format!("{prefixed} {{ inner: Arc::new(result) }}")
            }
        }
        TypeRef::Named(_) => {
            format!("{}::from(result)", to_turbofish_from(return_type))
        }
        TypeRef::Unit => "result".to_string(),
        _ => "result".to_string(),
    };
    let body = crate::backends::wasm::template_env::render(
        "gen_result_body",
        minijinja::context! {
            let_bindings => &let_bindings,
            core_call => &core_call,
            return_expr => &return_expr,
            is_async => true,
            map_wasm_error => false,
            map_js_error => func.error_type.is_some(),
            ok_return => func.error_type.is_some(),
        },
    );
    let fn_code = crate::backends::wasm::template_env::render(
        "gen_free_function",
        minijinja::context! {
            attrs => &attrs,
            js_name_attr => &js_name_attr,
            is_async => true,
            function_name => &func.name,
            params => async_params.join(", "),
            return_annotation => &return_annotation,
            body => body.trim_end(),
        },
    );
    format!("{input_dtos}{fn_code}")
}
