//! WASM struct method code generation.

use crate::backends::wasm::type_map::WasmMapper;
use crate::codegen::generators::binding_helpers::apply_return_newtype_unwrap;
use crate::codegen::type_mapper::TypeMapper;
use crate::codegen::{generators, naming::to_node_name, shared};
use crate::core::ir::{MethodDef, TypeDef, TypeRef};
use ahash::AHashSet;
use heck::ToPascalCase;

use super::functions::{
    borrow_opaque_param, emit_rustdoc, format_param_unused, wasm_call_args, wasm_mapped_param_type,
    wasm_mapped_return_type, wasm_newtype_param_bindings, wasm_wrap_return, wrap_jsvalue_mapped_return,
};

#[cfg(test)]
#[path = "methods_tests.rs"]
mod methods_tests;

/// Generate a method binding for a struct method.
#[allow(clippy::too_many_arguments)]
#[cfg(test)]
pub(super) fn gen_method(
    method: &MethodDef,
    mapper: &WasmMapper,
    type_name: &str,
    core_import: &str,
    opaque_types: &AHashSet<String>,
    prefix: &str,
    typ: &TypeDef,
    mutex_types: &AHashSet<String>,
    streaming_item_types: &ahash::AHashMap<String, String>,
    source_crate_remaps: &[(&str, &str)],
) -> String {
    let type_paths = [(
        typ.name.clone(),
        crate::codegen::conversions::core_type_path_remapped(typ, core_import, source_crate_remaps),
    )]
    .into_iter()
    .collect();
    gen_method_with_type_paths(
        method,
        mapper,
        type_name,
        core_import,
        opaque_types,
        prefix,
        typ,
        mutex_types,
        streaming_item_types,
        source_crate_remaps,
        &type_paths,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn gen_method_with_type_paths(
    method: &MethodDef,
    mapper: &WasmMapper,
    type_name: &str,
    core_import: &str,
    opaque_types: &AHashSet<String>,
    prefix: &str,
    typ: &TypeDef,
    mutex_types: &AHashSet<String>,
    streaming_item_types: &ahash::AHashMap<String, String>,
    source_crate_remaps: &[(&str, &str)],
    type_paths: &ahash::AHashMap<String, String>,
) -> String {
    // `type_name` is the bare IR name (`typ.name`) and is only safe to interpolate as
    // `{core_import}::{type_name}` when the type happens to be re-exported at the core crate
    // root. A type nested in a private module (e.g. `core::config::extraction::RenderOptions`)
    // is not reachable that way — rustc reports "cannot find `RenderOptions` in `sample_core`"
    // even though `sample_core` genuinely has the feature enabled. `core_type_path` is the
    // shared authority for this: it walks `typ.rust_path` and only falls back to
    // `{core_import}::{name}` when the type actually lives at the crate root. ~keep
    // ~keep Remapped, not bare: with `core_crate_override` set, IR `rust_path` values still
    // carry the *source* crate prefix, and the binding crate depends only on the override. The
    // unremapped form emitted `source_crate::Type::from(..)` into a crate with no such
    // dependency -- E0433 at every delegating method and `Default` impl.
    let qualified_type_path =
        crate::codegen::conversions::core_type_path_remapped(typ, core_import, source_crate_remaps);
    let has_mut_methods = typ
        .methods
        .iter()
        .any(|m| matches!(m.receiver.as_ref(), Some(crate::core::ir::ReceiverKind::RefMut)));

    let is_ref_mut_receiver = matches!(method.receiver.as_ref(), Some(crate::core::ir::ReceiverKind::RefMut));

    let can_delegate_base = shared::can_auto_delegate_with_named_let_bindings(method, opaque_types);
    let can_delegate = if is_ref_mut_receiver && has_mut_methods {
        !method.sanitized
            && method
                .params
                .iter()
                .all(|p| !p.sanitized && crate::codegen::shared::is_delegatable_param(&p.ty, opaque_types))
            && crate::codegen::shared::is_opaque_delegatable_type(&method.return_type)
    } else {
        can_delegate_base
    };

    let params: Vec<String> = method
        .params
        .iter()
        .map(|p| {
            let ty = wasm_mapped_param_type(p, mapper);
            let mapped_ty = if p.optional {
                format!("Option<{}>", ty)
            } else {
                borrow_opaque_param(p, &ty, opaque_types)
            };
            format_param_unused(&p.name, &mapped_ty, !can_delegate && !method.is_async)
        })
        .collect();

    let adapter_key_for_stream = format!("{}.{}", type_name, method.name);
    let stream_item = streaming_item_types.get(&adapter_key_for_stream);
    let return_type = if stream_item.is_some() {
        format!("{}Iterator", method.name.to_pascal_case())
    } else {
        wasm_mapped_return_type(&method.return_type, &method.return_newtype_wrapper, mapper)
    };
    let return_annotation = mapper.wrap_return(&return_type, method.error_type.is_some());

    let js_name = to_node_name(&method.name);
    let js_name_attr = if js_name != method.name {
        format!("(js_name = \"{}\")", js_name)
    } else {
        String::new()
    };

    let mut attrs = emit_rustdoc(&method.doc);
    let effective_param_count = if method.is_static {
        method.params.len()
    } else {
        method.params.len() + 1
    };
    if effective_param_count > 7 {
        attrs.push_str("#[allow(clippy::too_many_arguments)]\n");
    }
    if method.error_type.is_some() {
        attrs.push_str("#[allow(clippy::missing_errors_doc)]\n");
    }
    if generators::is_trait_method_name(&method.name) {
        attrs.push_str("#[allow(clippy::should_implement_trait)]\n");
    }

    if method.is_async && !method.is_static {
        let has_named = crate::codegen::generators::has_named_params(&method.params, opaque_types);

        let async_params: Vec<String> = if has_named {
            method
                .params
                .iter()
                .map(|p| match &p.ty {
                    TypeRef::Named(name) if !opaque_types.contains(name.as_str()) => {
                        let mapped_ty = if p.optional {
                            "Option<wasm_bindgen::JsValue>".to_string()
                        } else {
                            "wasm_bindgen::JsValue".to_string()
                        };
                        format!("{}: {}", p.name, mapped_ty)
                    }
                    _ => {
                        let ty = wasm_mapped_param_type(p, mapper);
                        let mapped_ty = if p.optional {
                            format!("Option<{}>", ty)
                        } else {
                            borrow_opaque_param(p, &ty, opaque_types)
                        };
                        format!("{}: {}", p.name, mapped_ty)
                    }
                })
                .collect()
        } else {
            params.clone()
        };

        let mut serde_bindings = String::new();
        if has_named {
            for p in &method.params {
                if let crate::core::ir::TypeRef::Named(name) = &p.ty
                    && !opaque_types.contains(name.as_str())
                {
                    let core_path = type_paths
                        .get(name)
                        .cloned()
                        .unwrap_or_else(|| format!("{}::{}", core_import, name));
                    let err_conv = ".map_err(|e| wasm_bindgen::JsValue::from_str(&e.to_string()))";
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
                        let has_default = false;
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
                }
            }
        }

        let mut let_bindings = serde_bindings;
        let_bindings.push_str(&wasm_newtype_param_bindings(
            &method.params,
            mapper,
            core_import,
            source_crate_remaps,
            type_paths,
        ));
        let call_args = wasm_call_args(&method.params, opaque_types, has_named);
        let is_opaque_type = opaque_types.contains(type_name);
        let core_call = if is_opaque_type && has_mut_methods && !is_ref_mut_receiver {
            format!(
                "self.inner.lock().unwrap().{method_name}({call_args})",
                method_name = method.name
            )
        } else {
            format!(
                "{qualified_type_path}::from(self.clone()).{method_name}({call_args})",
                method_name = method.name
            )
        };
        let result = apply_return_newtype_unwrap("result", &method.return_newtype_wrapper);
        let return_expr = wrap_jsvalue_mapped_return(&result, &return_type).unwrap_or_else(|| {
            wasm_wrap_return(
                &result,
                &method.return_type,
                type_name,
                opaque_types,
                false,
                method.returns_ref,
                method.returns_cow,
                prefix,
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
                map_wasm_error => method.error_type.is_some(),
                map_js_error => false,
                ok_return => method.error_type.is_some(),
            },
        );
        crate::backends::wasm::template_env::render(
            "gen_instance_method",
            crate::alef_context! {
                attrs => &attrs,
                js_name_attr => &js_name_attr,
                is_async => true,
                method_name => &method.name,
                params => async_params.join(", "),
                return_annotation => &return_annotation,
                body => body.trim_end(),
            },
        )
    } else if method.is_static {
        let body = if can_delegate {
            let has_named = crate::codegen::generators::has_named_params(&method.params, opaque_types);
            let let_bindings = if has_named {
                crate::codegen::generators::gen_named_let_bindings_no_promote(&method.params, opaque_types, core_import)
            } else {
                String::new()
            };
            let non_lifetime_container_params = method
                .params
                .iter()
                .filter(|param| {
                    !typ.has_lifetime_params
                        || !matches!(param.ty, TypeRef::Map(_, _))
                        || param.newtype_wrapper.is_some()
                })
                .cloned()
                .collect::<Vec<_>>();
            let newtype_bindings = wasm_newtype_param_bindings(
                &non_lifetime_container_params,
                mapper,
                core_import,
                source_crate_remaps,
                type_paths,
            );

            let is_borrowed_to_owned = method.name.contains("borrowed_attributes");
            let lifetime_bindings = if typ.has_lifetime_params {
                let mut bindings = String::new();
                for p in &method.params {
                    match &p.ty {
                        TypeRef::String => {
                            let template_name = if p.optional {
                                "lifetime_string_optional"
                            } else {
                                "lifetime_string_required"
                            };
                            bindings.push_str(&crate::backends::wasm::template_env::render(
                                template_name,
                                crate::alef_context! {
                                    param_name => &p.name,
                                },
                            ));
                            bindings.push_str("    ");
                        }
                        TypeRef::Map(_, _) if p.newtype_wrapper.is_none() => {
                            bindings.push_str(&crate::backends::wasm::template_env::render(
                                "lifetime_map_required",
                                crate::alef_context! {
                                    param_name => &p.name,
                                },
                            ));
                            bindings.push_str("    ");
                        }
                        TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::String) => {
                            bindings.push_str(&crate::backends::wasm::template_env::render(
                                "lifetime_string_optional",
                                crate::alef_context! {
                                    param_name => &p.name,
                                },
                            ));
                            bindings.push_str("    ");
                        }
                        _ => {}
                    }
                }
                bindings
            } else {
                String::new()
            };

            let (call_args, actual_method_name) = if !lifetime_bindings.is_empty() {
                let base_call_args = wasm_call_args(&method.params, opaque_types, has_named);
                let mut adjusted = base_call_args;
                for p in &method.params {
                    match &p.ty {
                        TypeRef::Map(_, _) => {
                            if is_borrowed_to_owned && p.is_ref {
                                adjusted = adjusted.replace(&format!("&{}", p.name), &format!("{}_converted", p.name));
                            } else {
                                adjusted = adjusted.replace(p.name.as_str(), &format!("{}_converted", p.name));
                            }
                        }
                        TypeRef::String => {
                            adjusted = adjusted.replace(p.name.as_str(), &format!("{}_converted", p.name));
                        }
                        TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::String) => {
                            adjusted = adjusted.replace(p.name.as_str(), &format!("{}_converted", p.name));
                        }
                        _ => {}
                    }
                }
                let method_name = if is_borrowed_to_owned {
                    method.name.replace("borrowed", "owned")
                } else {
                    method.name.clone()
                };
                (adjusted, method_name)
            } else {
                let base_call_args = wasm_call_args(&method.params, opaque_types, has_named);
                (base_call_args, method.name.clone())
            };

            let combined_let_bindings = format!("{let_bindings}{newtype_bindings}{lifetime_bindings}");
            let core_call = format!("{qualified_type_path}::{actual_method_name}({call_args})");
            if method.error_type.is_some() {
                let result = apply_return_newtype_unwrap("result", &method.return_newtype_wrapper);
                let wrap = wrap_jsvalue_mapped_return(&result, &return_type).unwrap_or_else(|| {
                    wasm_wrap_return(
                        &result,
                        &method.return_type,
                        type_name,
                        opaque_types,
                        false,
                        method.returns_ref,
                        method.returns_cow,
                        prefix,
                        mutex_types,
                    )
                });
                crate::backends::wasm::template_env::render(
                    "gen_result_body",
                    crate::alef_context! {
                        let_bindings => &combined_let_bindings,
                        core_call => &core_call,
                        return_expr => &wrap,
                        is_async => method.is_async,
                        map_wasm_error => false,
                        map_js_error => true,
                        ok_return => true,
                    },
                )
            } else if method.is_async {
                let result = apply_return_newtype_unwrap("result", &method.return_newtype_wrapper);
                let return_expr = wrap_jsvalue_mapped_return(&result, &return_type).unwrap_or_else(|| {
                    wasm_wrap_return(
                        &result,
                        &method.return_type,
                        type_name,
                        opaque_types,
                        false,
                        method.returns_ref,
                        method.returns_cow,
                        prefix,
                        mutex_types,
                    )
                });
                crate::backends::wasm::template_env::render(
                    "gen_result_body",
                    crate::alef_context! {
                        let_bindings => &combined_let_bindings,
                        core_call => &core_call,
                        return_expr => &return_expr,
                        is_async => true,
                        map_wasm_error => false,
                        map_js_error => false,
                        ok_return => false,
                    },
                )
            } else {
                let result = apply_return_newtype_unwrap(&core_call, &method.return_newtype_wrapper);
                let return_expr = wrap_jsvalue_mapped_return(&result, &return_type).unwrap_or_else(|| {
                    wasm_wrap_return(
                        &result,
                        &method.return_type,
                        type_name,
                        opaque_types,
                        false,
                        method.returns_ref,
                        method.returns_cow,
                        prefix,
                        mutex_types,
                    )
                });
                crate::backends::wasm::template_env::render(
                    "gen_direct_body",
                    crate::alef_context! {
                        let_bindings => &combined_let_bindings,
                        return_expr => &return_expr,
                    },
                )
            }
        } else {
            super::functions::gen_wasm_unimplemented_body(
                &method.return_type,
                &method.name,
                method.error_type.is_some(),
            )
        };
        crate::backends::wasm::template_env::render(
            "gen_static_method",
            crate::alef_context! {
                attrs => &attrs,
                js_name_attr => &js_name_attr,
                method_name => &method.name,
                params => params.join(", "),
                return_annotation => &return_annotation,
                body => body.trim_end(),
                is_async => method.is_async,
            },
        )
    } else {
        let body = if can_delegate {
            let has_named = crate::codegen::generators::has_named_params(&method.params, opaque_types);
            let mut let_bindings = if has_named {
                crate::codegen::generators::gen_named_let_bindings_no_promote(&method.params, opaque_types, core_import)
            } else {
                String::new()
            };
            let_bindings.push_str(&wasm_newtype_param_bindings(
                &method.params,
                mapper,
                core_import,
                source_crate_remaps,
                type_paths,
            ));
            let call_args = wasm_call_args(&method.params, opaque_types, has_named);
            let is_opaque_type = opaque_types.contains(type_name);
            let core_call = if is_opaque_type && has_mut_methods && !is_ref_mut_receiver {
                format!(
                    "self.inner.lock().unwrap().{method_name}({call_args})",
                    method_name = method.name
                )
            } else {
                format!(
                    "{qualified_type_path}::from(self.clone()).{method_name}({call_args})",
                    method_name = method.name
                )
            };
            if method.error_type.is_some() {
                let result = apply_return_newtype_unwrap("result", &method.return_newtype_wrapper);
                let wrap = wrap_jsvalue_mapped_return(&result, &return_type).unwrap_or_else(|| {
                    wasm_wrap_return(
                        &result,
                        &method.return_type,
                        type_name,
                        opaque_types,
                        false,
                        method.returns_ref,
                        method.returns_cow,
                        prefix,
                        mutex_types,
                    )
                });
                crate::backends::wasm::template_env::render(
                    "gen_result_body",
                    crate::alef_context! {
                        let_bindings => &let_bindings,
                        core_call => &core_call,
                        return_expr => &wrap,
                        is_async => false,
                        map_wasm_error => false,
                        map_js_error => true,
                        ok_return => true,
                    },
                )
            } else {
                let result = apply_return_newtype_unwrap(&core_call, &method.return_newtype_wrapper);
                let return_expr = wrap_jsvalue_mapped_return(&result, &return_type).unwrap_or_else(|| {
                    wasm_wrap_return(
                        &result,
                        &method.return_type,
                        type_name,
                        opaque_types,
                        false,
                        method.returns_ref,
                        method.returns_cow,
                        prefix,
                        mutex_types,
                    )
                });
                crate::backends::wasm::template_env::render(
                    "gen_direct_body",
                    crate::alef_context! {
                        let_bindings => &let_bindings,
                        return_expr => &return_expr,
                    },
                )
            }
        } else {
            super::functions::gen_wasm_unimplemented_body(
                &method.return_type,
                &method.name,
                method.error_type.is_some(),
            )
        };
        crate::backends::wasm::template_env::render(
            "gen_instance_method",
            crate::alef_context! {
                attrs => &attrs,
                js_name_attr => &js_name_attr,
                is_async => false,
                method_name => &method.name,
                params => params.join(", "),
                return_annotation => &return_annotation,
                body => body.trim_end(),
            },
        )
    }
}
