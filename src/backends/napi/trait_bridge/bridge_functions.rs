use crate::core::config::TraitBridgeConfig;
use crate::core::ir::ApiSurface;

#[allow(clippy::too_many_arguments)]
pub fn gen_bridge_function(
    api: &ApiSurface,
    func: &crate::core::ir::FunctionDef,
    bridge_param_idx: usize,
    bridge_cfg: &TraitBridgeConfig,
    mapper: &dyn crate::codegen::type_mapper::TypeMapper,
    _cfg: &crate::codegen::generators::RustBindingConfig<'_>,
    _adapter_bodies: &crate::codegen::generators::AdapterBodies,
    opaque_types: &ahash::AHashSet<String>,
    core_import: &str,
) -> String {
    use crate::core::ir::TypeRef;

    let struct_name = crate::codegen::generators::trait_bridge::bridge_wrapper_name("Js", bridge_cfg);
    let handle_path = crate::codegen::generators::trait_bridge::bridge_handle_path(api, bridge_cfg, core_import);
    let param_name = &func.params[bridge_param_idx].name;
    let bridge_param = &func.params[bridge_param_idx];
    let is_optional = bridge_param.optional || matches!(&bridge_param.ty, TypeRef::Optional(_));

    let is_options_field_binding = matches!(bridge_cfg.bind_via, crate::core::config::BridgeBinding::OptionsField);

    let options_param_idx = if is_options_field_binding {
        func.params.iter().enumerate().find(|(_, p)| {
            matches!(&p.ty, TypeRef::Named(n) if bridge_cfg.options_type.as_ref().is_some_and(|opt_type| n == opt_type))
        }).map(|(i, _)| i)
    } else {
        None
    };

    let mut sig_parts = vec![];
    for (idx, p) in func.params.iter().enumerate() {
        if is_options_field_binding && Some(idx) == options_param_idx {
            let ty = if p.optional || (idx > 0 && func.params[..idx].iter().any(|pp| pp.optional)) {
                format!("Option<{}>", mapper.map_type(&p.ty))
            } else {
                mapper.map_type(&p.ty)
            };
            sig_parts.push(format!("{}: {}", p.name, ty));
        } else if idx == bridge_param_idx {
            if is_optional {
                sig_parts.push(format!("{}: Option<napi::bindgen_prelude::Object>", p.name));
            } else {
                sig_parts.push(format!("{}: napi::bindgen_prelude::Object", p.name));
            }
        } else {
            let promoted = idx > bridge_param_idx || func.params[..idx].iter().any(|pp| pp.optional);
            let ty = if p.optional || promoted {
                format!("Option<{}>", mapper.map_type(&p.ty))
            } else {
                mapper.map_type(&p.ty)
            };
            sig_parts.push(format!("{}: {}", p.name, ty));
        }
    }

    let params_str = sig_parts.join(", ");
    let return_type = mapper.map_type(&func.return_type);
    let ret = mapper.wrap_return(&return_type, func.error_type.is_some());

    let err_conv = ".map_err(|e| napi::Error::new(napi::Status::GenericFailure, e.to_string()))";

    let bridge_wrap = if is_optional {
        crate::backends::napi::template_env::render(
            "bridge_optional_wrap.jinja",
            minijinja::context! {
                param_name => param_name,
                struct_name => struct_name,
                handle_path => handle_path,
            },
        )
    } else {
        crate::backends::napi::template_env::render(
            "bridge_required_wrap.jinja",
            minijinja::context! {
                param_name => param_name,
                struct_name => struct_name,
                handle_path => handle_path,
            },
        )
    };

    let serde_bindings: String = func
        .params
        .iter()
        .enumerate()
        .filter(|(idx, p)| {
            if *idx == bridge_param_idx {
                return false;
            }
            let named = match &p.ty {
                TypeRef::Named(n) => Some(n.as_str()),
                TypeRef::Optional(inner) => {
                    if let TypeRef::Named(n) = inner.as_ref() {
                        Some(n.as_str())
                    } else {
                        None
                    }
                }
                _ => None,
            };
            named.is_some_and(|n| !opaque_types.contains(n))
        })
        .map(|(_, p)| {
            let name = &p.name;
            let core_path = format!(
                "{core_import}::{}",
                match &p.ty {
                    TypeRef::Named(n) => n.clone(),
                    TypeRef::Optional(inner) =>
                        if let TypeRef::Named(n) = inner.as_ref() {
                            n.clone()
                        } else {
                            String::new()
                        },
                    _ => String::new(),
                }
            );
            let template_name = if p.optional || matches!(&p.ty, TypeRef::Optional(_)) {
                "named_core_binding_optional.jinja"
            } else {
                "named_core_binding_required.jinja"
            };
            crate::backends::napi::template_env::render(
                template_name,
                minijinja::context! {
                    name => name,
                    core_path => core_path,
                },
            )
        })
        .collect();

    let call_args: Vec<String> = func
        .params
        .iter()
        .enumerate()
        .map(|(idx, p)| {
            if idx == bridge_param_idx {
                return p.name.clone();
            }
            match &p.ty {
                TypeRef::Named(n) if opaque_types.contains(n.as_str()) => {
                    if p.optional {
                        format!("{}.as_ref().map(|v| &v.inner)", p.name)
                    } else {
                        format!("&{}.inner", p.name)
                    }
                }
                TypeRef::Named(_) => format!("{}_core", p.name),
                TypeRef::Optional(inner) => {
                    if let TypeRef::Named(n) = inner.as_ref() {
                        if opaque_types.contains(n.as_str()) {
                            format!("{}.as_ref().map(|v| &v.inner)", p.name)
                        } else {
                            format!("{}_core", p.name)
                        }
                    } else {
                        p.name.clone()
                    }
                }
                TypeRef::String | TypeRef::Char => {
                    if p.is_ref {
                        format!("&{}", p.name)
                    } else {
                        p.name.clone()
                    }
                }
                _ => p.name.clone(),
            }
        })
        .collect();
    let call_args_str = call_args.join(", ");

    let core_fn_path = {
        let path = func.rust_path.replace('-', "_");
        if path.starts_with(core_import) {
            path
        } else {
            format!("{core_import}::{}", func.name)
        }
    };
    let core_call = format!("{core_fn_path}({call_args_str})");

    let return_wrap = match &func.return_type {
        TypeRef::Named(name) if opaque_types.contains(name.as_str()) => {
            format!("{name} {{ inner: std::sync::Arc::new(val) }}")
        }
        TypeRef::Named(_) => "val.into()".to_string(),
        TypeRef::String | TypeRef::Bytes => "val.into()".to_string(),
        _ => "val".to_string(),
    };

    let body = render_bridge_function_body(
        func.error_type.is_some(),
        &return_wrap,
        &bridge_wrap,
        &serde_bindings,
        &core_call,
        err_conv,
    );

    let js_name = {
        let mut result = String::with_capacity(func.name.len());
        let mut capitalize_next = false;
        for (i, c) in func.name.chars().enumerate() {
            if c == '_' {
                capitalize_next = true;
            } else if capitalize_next {
                result.extend(c.to_uppercase());
                capitalize_next = false;
            } else if i == 0 {
                result.extend(c.to_lowercase());
            } else {
                result.push(c);
            }
        }
        result
    };
    let js_name_attr = if js_name != func.name {
        format!("(js_name = \"{}\")", js_name)
    } else {
        String::new()
    };

    let func_name = &func.name;
    crate::backends::napi::template_env::render(
        "bridge_function.jinja",
        minijinja::context! {
            has_error => func.error_type.is_some(),
            js_name_attr => js_name_attr,
            func_name => func_name,
            params_str => params_str,
            ret => ret,
            body => body,
        },
    )
}

fn render_bridge_function_body(
    has_error: bool,
    return_wrap: &str,
    bridge_wrap: &str,
    serde_bindings: &str,
    core_call: &str,
    err_conv: &str,
) -> String {
    let template_name = match (has_error, return_wrap == "val") {
        (true, true) => "bridge_function_body_error.jinja",
        (true, false) => "bridge_function_body_error_mapped.jinja",
        (false, _) => "bridge_function_body_plain.jinja",
    };
    crate::backends::napi::template_env::render(
        template_name,
        minijinja::context! {
            bridge_wrap => bridge_wrap,
            serde_bindings => serde_bindings,
            core_call => core_call,
            err_conv => err_conv,
            return_wrap => return_wrap,
        },
    )
}
