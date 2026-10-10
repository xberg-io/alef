//! Async free-function wrapper generation for WASM.

use super::params::{wasm_call_args, wasm_mapped_param_type, wasm_newtype_param_bindings};
use super::returns::{type_has_default, wasm_mapped_return_type, wasm_wrap_return_fn, wrap_jsvalue_mapped_return};
use crate::backends::wasm::type_map::WasmMapper;
use crate::codegen::generators::binding_helpers::apply_return_newtype_unwrap;
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
    attrs: &str,
    js_name_attr: &str,
    return_annotation: &str,
    core_fn_path: &str,
    source_crate_remaps: &[(&str, &str)],
    type_paths: &ahash::AHashMap<String, String>,
) -> String {
    let has_named = crate::codegen::generators::has_named_params(&func.params, opaque_types);

    let async_params: Vec<String> = if has_named {
        func.params
            .iter()
            .map(|p| match &p.ty {
                TypeRef::Named(name) if !opaque_types.contains(name.as_str()) => {
                    let wasm_ty = mapper.map_type(&p.ty);
                    if crate::codegen::shared::maps_to_js_value(&wasm_ty) {
                        let mapped_ty = if p.optional {
                            "Option<JsValue>".to_string()
                        } else {
                            "JsValue".to_string()
                        };
                        format!("{}: {}", p.name, mapped_ty)
                    } else {
                        // A dedicated `#[wasm_bindgen]` wrapper exists for this type (with a
                        // `From<Wasm{name}>` conversion into the core type), so the parameter
                        // takes the typed wrapper instead of a round-trip through
                        // `serde_wasm_bindgen::from_value`. That round-trip deserializes by
                        // asking the JS object for each of the core type's *snake_case* serde
                        // field names, which never matches the camelCase getters wasm-bindgen
                        // emits for the wrapper's own fields — every multi-word field silently
                        // falls back to its serde default. `Option<...>` regardless of
                        // `p.optional` preserves the existing "omit -> use default" ergonomics,
                        // mirrored in the binding body below. ~keep
                        format!("{}: Option<{}>", p.name, wasm_ty)
                    }
                }
                _ => {
                    let ty = wasm_mapped_param_type(p, mapper);
                    let mapped_ty = if p.optional {
                        format!("Option<{}>", ty)
                    } else {
                        super::params::borrow_opaque_param(p, &ty, opaque_types)
                    };
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
                    let core_path = type_paths
                        .get(name)
                        .cloned()
                        .unwrap_or_else(|| format!("{}::{}", core_import, name));
                    let wasm_ty = mapper.map_type(&p.ty);
                    if crate::codegen::shared::maps_to_js_value(&wasm_ty) {
                        let err_conv = ".map_err(|e| JsValue::from_str(&e.to_string()))";
                        if p.optional {
                            serde_bindings.push_str(&crate::backends::wasm::template_env::render(
                                "serde_named_optional",
                                crate::alef_context! {
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
                                crate::alef_context! {
                                    param_name => &p.name,
                                    core_path => &core_path,
                                    err_conv => &err_conv,
                                    has_default => has_default,
                                    is_mut => p.is_mut,
                                },
                            ));
                            serde_bindings.push_str("    ");
                        }
                    } else if p.optional {
                        serde_bindings.push_str(&crate::backends::wasm::template_env::render(
                            "wasm_typed_named_optional",
                            crate::alef_context! {
                                param_name => &p.name,
                                core_path => &core_path,
                            },
                        ));
                        serde_bindings.push_str("    ");
                    } else {
                        let has_default = type_has_default(name, api);
                        serde_bindings.push_str(&crate::backends::wasm::template_env::render(
                            "wasm_typed_named_required",
                            crate::alef_context! {
                                param_name => &p.name,
                                core_path => &core_path,
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
                let core_path = type_paths
                    .get(name)
                    .cloned()
                    .unwrap_or_else(|| format!("{}::{}", core_import, name));
                let template_name = if p.optional {
                    "serde_vec_named_from_optional"
                } else {
                    "serde_vec_named_from_required"
                };
                serde_bindings.push_str(&crate::backends::wasm::template_env::render(
                    template_name,
                    crate::alef_context! {
                        param_name => &p.name,
                        core_path => &core_path,
                    },
                ));
                serde_bindings.push_str("    ");
            }
        }
    }

    let mut let_bindings = serde_bindings;
    let_bindings.push_str(&wasm_newtype_param_bindings(
        &func.params,
        mapper,
        core_import,
        source_crate_remaps,
        type_paths,
    ));
    let call_args = wasm_call_args(&func.params, opaque_types, has_named);
    let core_call = format!("{core_fn_path}({call_args})");
    let result = apply_return_newtype_unwrap("result", &func.return_newtype_wrapper);
    let mapped_return = wasm_mapped_return_type(&func.return_type, &func.return_newtype_wrapper, mapper);
    let return_expr = wrap_jsvalue_mapped_return(&result, &mapped_return).unwrap_or_else(|| {
        wasm_wrap_return_fn(
            &result,
            &func.return_type,
            opaque_types,
            func.returns_ref,
            func.returns_cow,
            &mapper.prefix,
            mutex_types,
        )
    });
    let body = crate::backends::wasm::template_env::render(
        "gen_result_body",
        crate::alef_context! {
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
        crate::alef_context! {
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
