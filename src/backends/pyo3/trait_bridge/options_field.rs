use crate::core::config::TraitBridgeConfig;
use crate::core::ir::ApiSurface;

#[allow(clippy::too_many_arguments)]
pub fn gen_bridge_field_function(
    api: &ApiSurface,
    func: &crate::core::ir::FunctionDef,
    bridge_match: &crate::codegen::generators::trait_bridge::BridgeFieldMatch<'_>,
    bridge_cfg: &TraitBridgeConfig,
    mapper: &dyn crate::codegen::type_mapper::TypeMapper,
    cfg: &crate::codegen::generators::RustBindingConfig<'_>,
    opaque_types: &ahash::AHashSet<String>,
    core_import: &str,
    error_converters: &[String],
) -> String {
    use crate::codegen::generators::AsyncPattern;
    use crate::core::ir::TypeRef;

    let struct_name = crate::codegen::generators::trait_bridge::bridge_wrapper_name("Py", bridge_cfg);
    let handle_path = crate::codegen::generators::trait_bridge::bridge_handle_path(api, bridge_cfg, core_import);

    let visitor_kwarg = bridge_cfg.param_name.as_deref().unwrap_or("visitor");
    let options_param = &bridge_match.param_name;
    let options_type = &bridge_match.options_type;
    let field_name = &bridge_match.field_name;
    let param_is_optional = bridge_match.param_is_optional;

    let func_needs_py = func.is_async && cfg.async_pattern == AsyncPattern::Pyo3FutureIntoPy;
    let lifetime = if func_needs_py { "<'py>" } else { "" };

    let mut sig_parts = Vec::new();
    if func_needs_py {
        sig_parts.push("py: Python<'py>".to_string());
    }
    for p in func.params.iter() {
        let ty = if p.optional || matches!(&p.ty, TypeRef::Optional(_)) {
            format!("Option<{}>", mapper.map_type(&p.ty))
        } else {
            mapper.map_type(&p.ty)
        };
        sig_parts.push(format!("{}: {}", p.name, ty));
    }
    sig_parts.push(format!("{visitor_kwarg}: Option<Py<PyAny>>"));

    let params_str = sig_parts.join(", ");
    let return_type = mapper.map_type(&func.return_type);
    let ret = mapper.wrap_return(&return_type, func.error_type.is_some());
    let ret = if func_needs_py {
        "PyResult<Bound<'py, PyAny>>".to_string()
    } else {
        ret
    };

    let visitor_wrap = format!(
        "let {visitor_kwarg}_handle: Option<{handle_path}> = {visitor_kwarg}.map(|v| {{\n        \
         let bridge = {struct_name}::new(v);\n        \
         std::sync::Arc::new(std::sync::Mutex::new(bridge)) as {handle_path}\n    \
         }});"
    );

    let serde_err_conv = ".map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))";
    let serde_bindings: String = func
        .params
        .iter()
        .filter(|p| {
            if p.name == *options_param {
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
        .map(|p| {
            let name = &p.name;
            let core_type_name = match &p.ty {
                TypeRef::Named(n) => n.clone(),
                TypeRef::Optional(inner) => {
                    if let TypeRef::Named(n) = inner.as_ref() {
                        n.clone()
                    } else {
                        String::new()
                    }
                }
                _ => String::new(),
            };
            let core_path = format!("{core_import}::{core_type_name}");
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

    let core_options_type = format!("{core_import}::{options_type}");
    let options_core_binding = if param_is_optional {
        format!(
            "let {options_param}_core: Option<{core_options_type}> = {options_param}.map(|v| v.into());\n    \
             // Inject the visitor handle: upgrade existing options or construct defaults.\n    \
             let {options_param}_core: Option<{core_options_type}> = if let Some(handle) = {visitor_kwarg}_handle {{\n        \
             let mut opts = {options_param}_core.unwrap_or_default();\n        \
             opts.{field_name} = Some(handle);\n        \
             Some(opts)\n    \
             }} else {{\n        \
             {options_param}_core\n    \
             }};"
        )
    } else {
        format!(
            "let mut {options_param}_core: {core_options_type} = match &{options_param} {{\n        \
             Some(opts) => opts.clone().into(),\n        \
             None => {core_options_type}::default(),\n    \
             }};\n    \
             if let Some(handle) = {visitor_kwarg}_handle {{\n        \
             {options_param}_core.{field_name} = Some(handle);\n    \
             }}"
        )
    };

    let call_args: Vec<String> = func
        .params
        .iter()
        .map(|p| {
            if p.name == *options_param {
                return format!("{options_param}_core");
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
            format!("{visitor_wrap}\n    {serde_bindings}{options_core_binding}\n    {core_call}{core_err_conv}")
        } else {
            format!(
                "{visitor_wrap}\n    {serde_bindings}{options_core_binding}\n    {core_call}.map(|val| {return_wrap}){core_err_conv}"
            )
        }
    } else {
        format!("{visitor_wrap}\n    {serde_bindings}{options_core_binding}\n    {core_call}")
    };

    let attr_inner = cfg
        .function_attr
        .trim_start_matches('#')
        .trim_start_matches('[')
        .trim_end_matches(']');

    let mut sig_str = String::new();
    if cfg.needs_signature {
        // #[pyo3(signature = (...))] — all params from the IR plus the extra visitor kwarg.
        let mut seen_optional = false;
        let mut sig_items: Vec<String> = func
            .params
            .iter()
            .map(|p| {
                if p.optional {
                    seen_optional = true;
                }
                if p.optional || seen_optional {
                    format!("{}=None", p.name)
                } else {
                    p.name.clone()
                }
            })
            .collect();
        sig_items.push(format!("{visitor_kwarg}=None"));
        sig_str = sig_items.join(", ");
    }
    let func_name = &func.name;
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
