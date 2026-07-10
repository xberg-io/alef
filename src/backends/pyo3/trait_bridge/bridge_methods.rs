use crate::core::config::TraitBridgeConfig;
use crate::core::ir::ApiSurface;

#[allow(clippy::too_many_arguments)]
pub fn gen_bridge_function(
    api: &ApiSurface,
    func: &crate::core::ir::FunctionDef,
    bridge_param_idx: usize,
    bridge_cfg: &TraitBridgeConfig,
    mapper: &dyn crate::codegen::type_mapper::TypeMapper,
    cfg: &crate::codegen::generators::RustBindingConfig<'_>,
    adapter_bodies: &crate::codegen::generators::AdapterBodies,
    opaque_types: &ahash::AHashSet<String>,
    core_import: &str,
    error_converters: &[String],
) -> String {
    use crate::codegen::generators::AsyncPattern;
    use crate::core::ir::TypeRef;

    let struct_name = crate::codegen::generators::trait_bridge::bridge_wrapper_name("Py", bridge_cfg);
    let handle_path = crate::codegen::generators::trait_bridge::bridge_handle_path(api, bridge_cfg, core_import);

    let param_name = &func.params[bridge_param_idx].name;
    let bridge_param = &func.params[bridge_param_idx];
    let is_optional = bridge_param.optional || matches!(&bridge_param.ty, TypeRef::Optional(_));

    let mut sig_parts = Vec::new();
    let func_needs_py = func.is_async && cfg.async_pattern == AsyncPattern::Pyo3FutureIntoPy;
    if func_needs_py {
        sig_parts.push("py: Python<'py>".to_string());
    }

    for (idx, p) in func.params.iter().enumerate() {
        if idx == bridge_param_idx {
            if is_optional {
                sig_parts.push(format!("{}: Option<Py<PyAny>>", p.name));
            } else {
                sig_parts.push(format!("{}: Py<PyAny>", p.name));
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
    let ret = if func_needs_py {
        "PyResult<Bound<'py, PyAny>>".to_string()
    } else {
        ret
    };
    let lifetime = if func_needs_py { "<'py>" } else { "" };

    let bridge_wrap = if is_optional {
        format!(
            "let {param_name} = {param_name}.map(|v| {{\n        \
             let bridge = {struct_name}::new(v);\n        \
             std::sync::Arc::new(std::sync::Mutex::new(bridge)) as {handle_path}\n    \
             }});"
        )
    } else {
        format!(
            "let {param_name} = {{\n        \
             let bridge = {struct_name}::new({param_name});\n        \
             std::sync::Arc::new(std::sync::Mutex::new(bridge)) as {handle_path}\n    \
             }};"
        )
    };

    let serde_err_conv = ".map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))";

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
            if p.optional || matches!(&p.ty, TypeRef::Optional(_)) {
                format!(
                    "let {name}_core: Option<{core_path}> = {name}.map(|v| {{\n        \
                 let json = serde_json::to_string(&v){serde_err_conv}?;\n        \
                 serde_json::from_str(&json){serde_err_conv}\n    \
                 }}).transpose()?;\n    "
                )
            } else {
                format!(
                    "let {name}_json = serde_json::to_string(&{name}){serde_err_conv}?;\n    \
                 let {name}_core: {core_path} = serde_json::from_str(&{name}_json){serde_err_conv}?;\n    "
                )
            }
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

    let body = if let Some(ref error_type) = func.error_type {
        let core_err_conv = if error_type.contains("::") || error_type == "Error" {
            if error_converters.len() == 1 {
                format!(".map_err({})", error_converters[0])
            } else {
                ".map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))".to_string()
            }
        } else {
            let snake_error = {
                let mut s = String::with_capacity(error_type.len() + 4);
                for (i, c) in error_type.chars().enumerate() {
                    if c.is_uppercase() {
                        if i > 0 {
                            s.push('_');
                        }
                        s.push(c.to_ascii_lowercase());
                    } else {
                        s.push(c);
                    }
                }
                s
            };
            format!(".map_err({snake_error}_to_py_err)")
        };
        if return_wrap == "val" {
            format!("{bridge_wrap}\n    {serde_bindings}{core_call}{core_err_conv}")
        } else {
            format!("{bridge_wrap}\n    {serde_bindings}{core_call}.map(|val| {return_wrap}){core_err_conv}")
        }
    } else {
        format!("{bridge_wrap}\n    {serde_bindings}{core_call}")
    };

    let attr_inner = cfg
        .function_attr
        .trim_start_matches('#')
        .trim_start_matches('[')
        .trim_end_matches(']');

    let mut sig_str = String::new();
    if cfg.needs_signature {
        let mut seen_optional = false;
        let sig_parts: Vec<String> = func
            .params
            .iter()
            .enumerate()
            .map(|(idx, p)| {
                let this_optional = if idx == bridge_param_idx {
                    is_optional
                } else {
                    p.optional
                };
                if this_optional {
                    seen_optional = true;
                }
                if this_optional || seen_optional {
                    format!("{}=None", p.name)
                } else {
                    p.name.clone()
                }
            })
            .collect();
        sig_str = sig_parts.join(", ");
    }

    let func_name = &func.name;

    let _ = adapter_bodies;

    crate::backends::pyo3::template_env::render(
        "trait_bridge/function_wrapper.jinja",
        minijinja::context! {
            has_error => func.error_type.is_some(),
            attr_inner => attr_inner,
            needs_signature => cfg.needs_signature,
            signature_prefix => cfg.signature_prefix,
            sig_str => sig_str,
            signature_suffix => cfg.signature_suffix,
            func_name => func_name,
            lifetime => lifetime,
            params_str => params_str,
            ret => ret,
            body => body,
        },
    )
}
