//! WASM struct method code generation.

use crate::backends::wasm::type_map::WasmMapper;
use crate::codegen::type_mapper::TypeMapper;
use crate::codegen::{generators, naming::to_node_name, shared};
use crate::core::ir::{MethodDef, TypeDef, TypeRef};
use ahash::AHashSet;
use heck::ToPascalCase;

use super::functions::{emit_rustdoc, format_param_unused, wasm_wrap_return};

/// Generate a method binding for a struct method.
#[allow(clippy::too_many_arguments)]
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
) -> String {
    let has_mut_methods = typ
        .methods
        .iter()
        .any(|m| matches!(m.receiver.as_ref(), Some(crate::core::ir::ReceiverKind::RefMut)));

    let is_ref_mut_receiver = matches!(method.receiver.as_ref(), Some(crate::core::ir::ReceiverKind::RefMut));

    let can_delegate_base = shared::can_auto_delegate(method, opaque_types);
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
            let ty = mapper.map_type(&p.ty);
            let mapped_ty = if p.optional { format!("Option<{}>", ty) } else { ty };
            format_param_unused(&p.name, &mapped_ty, !can_delegate && !method.is_async)
        })
        .collect();

    let adapter_key_for_stream = format!("{}.{}", type_name, method.name);
    let stream_item = streaming_item_types.get(&adapter_key_for_stream);
    let return_type = if stream_item.is_some() {
        format!("{}Iterator", method.name.to_pascal_case())
    } else {
        mapper.map_type(&method.return_type)
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

    if method.is_async {
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
                        let ty = mapper.map_type(&p.ty);
                        let mapped_ty = if p.optional { format!("Option<{}>", ty) } else { ty };
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
                if let crate::core::ir::TypeRef::Named(name) = &p.ty {
                    if !opaque_types.contains(name.as_str()) {
                        let core_path = format!("{}::{}", core_import, name);
                        let err_conv = ".map_err(|e| wasm_bindgen::JsValue::from_str(&e.to_string()))";
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
                            let has_default = false;
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
                }
            }
        }

        let let_bindings = serde_bindings;
        let call_args = if let_bindings.is_empty() {
            generators::gen_call_args(&method.params, opaque_types)
        } else {
            generators::gen_call_args_with_let_bindings(&method.params, opaque_types)
        };
        let is_opaque_type = opaque_types.contains(type_name);
        let core_call = if is_opaque_type && has_mut_methods && !is_ref_mut_receiver {
            format!(
                "self.inner.lock().unwrap().{method_name}({call_args})",
                method_name = method.name
            )
        } else {
            format!(
                "{core_import}::{type_name}::from(self.clone()).{method_name}({call_args})",
                method_name = method.name
            )
        };
        let return_type_tf = to_turbofish_from(&return_type);
        let return_expr = format!("{return_type_tf}::from(result)");
        let body = crate::backends::wasm::template_env::render(
            "gen_result_body",
            minijinja::context! {
                let_bindings => &let_bindings,
                core_call => &core_call,
                return_expr => &return_expr,
                is_async => true,
                map_wasm_error => method.error_type.is_some(),
                map_js_error => false,
                ok_return => true,
            },
        );
        crate::backends::wasm::template_env::render(
            "gen_instance_method",
            minijinja::context! {
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
            let let_bindings = if crate::codegen::generators::has_named_params(&method.params, opaque_types) {
                crate::codegen::generators::gen_named_let_bindings_no_promote(&method.params, opaque_types, core_import)
            } else {
                String::new()
            };

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
                                minijinja::context! {
                                    param_name => &p.name,
                                },
                            ));
                            bindings.push_str("    ");
                        }
                        TypeRef::Map(_, _) => {
                            bindings.push_str(&crate::backends::wasm::template_env::render(
                                "lifetime_map_required",
                                minijinja::context! {
                                    param_name => &p.name,
                                },
                            ));
                            bindings.push_str("    ");
                        }
                        TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::String) => {
                            bindings.push_str(&crate::backends::wasm::template_env::render(
                                "lifetime_string_optional",
                                minijinja::context! {
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
                let base_call_args = if let_bindings.is_empty() {
                    generators::gen_call_args(&method.params, opaque_types)
                } else {
                    generators::gen_call_args_with_let_bindings(&method.params, opaque_types)
                };
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
                let base_call_args = if let_bindings.is_empty() {
                    generators::gen_call_args(&method.params, opaque_types)
                } else {
                    generators::gen_call_args_with_let_bindings(&method.params, opaque_types)
                };
                (base_call_args, method.name.clone())
            };

            let combined_let_bindings = format!("{let_bindings}{lifetime_bindings}");
            let core_call = format!("{core_import}::{type_name}::{actual_method_name}({call_args})");
            if method.error_type.is_some() {
                let wrap = wasm_wrap_return(
                    "result",
                    &method.return_type,
                    type_name,
                    opaque_types,
                    false,
                    method.returns_ref,
                    method.returns_cow,
                    prefix,
                    mutex_types,
                );
                crate::backends::wasm::template_env::render(
                    "gen_result_body",
                    minijinja::context! {
                        let_bindings => &combined_let_bindings,
                        core_call => &core_call,
                        return_expr => &wrap,
                        is_async => false,
                        map_wasm_error => false,
                        map_js_error => true,
                        ok_return => true,
                    },
                )
            } else {
                let return_expr = wasm_wrap_return(
                    &core_call,
                    &method.return_type,
                    type_name,
                    opaque_types,
                    false,
                    method.returns_ref,
                    method.returns_cow,
                    prefix,
                    mutex_types,
                );
                crate::backends::wasm::template_env::render(
                    "gen_direct_body",
                    minijinja::context! {
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
            minijinja::context! {
                attrs => &attrs,
                js_name_attr => &js_name_attr,
                method_name => &method.name,
                params => params.join(", "),
                return_annotation => &return_annotation,
                body => body.trim_end(),
            },
        )
    } else {
        let body = if can_delegate {
            let let_bindings = if crate::codegen::generators::has_named_params(&method.params, opaque_types) {
                crate::codegen::generators::gen_named_let_bindings_no_promote(&method.params, opaque_types, core_import)
            } else {
                String::new()
            };
            let call_args = if let_bindings.is_empty() {
                generators::gen_call_args(&method.params, opaque_types)
            } else {
                generators::gen_call_args_with_let_bindings(&method.params, opaque_types)
            };
            let is_opaque_type = opaque_types.contains(type_name);
            let core_call = if is_opaque_type && has_mut_methods && !is_ref_mut_receiver {
                format!(
                    "self.inner.lock().unwrap().{method_name}({call_args})",
                    method_name = method.name
                )
            } else {
                format!(
                    "{core_import}::{type_name}::from(self.clone()).{method_name}({call_args})",
                    method_name = method.name
                )
            };
            if method.error_type.is_some() {
                let wrap = wasm_wrap_return(
                    "result",
                    &method.return_type,
                    type_name,
                    opaque_types,
                    false,
                    method.returns_ref,
                    method.returns_cow,
                    prefix,
                    mutex_types,
                );
                crate::backends::wasm::template_env::render(
                    "gen_result_body",
                    minijinja::context! {
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
                let return_expr = wasm_wrap_return(
                    &core_call,
                    &method.return_type,
                    type_name,
                    opaque_types,
                    false,
                    method.returns_ref,
                    method.returns_cow,
                    prefix,
                    mutex_types,
                );
                crate::backends::wasm::template_env::render(
                    "gen_direct_body",
                    minijinja::context! {
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
            minijinja::context! {
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

/// Returns a type name in turbofish form for use before `::from(expr)`.
///
/// Rust requires turbofish when a type has generic parameters and sits before `::`:
///   `Vec<T>::from(x)` is a syntax error — `Vec::<T>::from(x)` is required.
/// Non-generic type names are returned unchanged.
fn to_turbofish_from(type_name: &str) -> String {
    if let Some(idx) = type_name.find('<') {
        format!("{}::{}", &type_name[..idx], &type_name[idx..])
    } else {
        type_name.to_string()
    }
}
