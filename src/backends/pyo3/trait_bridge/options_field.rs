use crate::core::config::TraitBridgeConfig;
use crate::core::ir::ApiSurface;

/// Whether the owning struct's bridge field survives into the Python binding's own DTO.
///
/// Only then can the wrapper fall back to a handle the caller set on the options object
/// itself; without the field there is nothing to read. `cfg`-gated fields are excluded for the
/// same reason the binding omits them. ~keep
fn bridge_field_reaches_binding(api: &ApiSurface, bridge_cfg: &TraitBridgeConfig, field_name: &str) -> bool {
    let Some(options_type) = bridge_cfg.options_type.as_deref() else {
        return false;
    };
    api.types
        .iter()
        .filter(|t| t.name == options_type)
        .flat_map(|t| t.fields.iter())
        .any(|f| f.cfg.is_none() && f.name == field_name)
}

/// The expression that reads an already-set bridge handle off the options argument.
///
/// Two shapes, because the parameter is `Option<Options>` in one case and a bare `Options` in
/// the other; emitting the `Option` shape unconditionally is what made the required-parameter
/// wrapper fail to compile (alef #476). ~keep
fn options_field_access(options_param: &str, field_name: &str, param_is_optional: bool) -> String {
    if param_is_optional {
        format!("{options_param}.as_ref().and_then(|o| o.{field_name}.as_ref())")
    } else {
        format!("{options_param}.{field_name}.as_ref()")
    }
}

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

    let bridge_struct = crate::codegen::generators::trait_bridge::bridge_wrapper_name("Py", bridge_cfg);
    let handle_path = crate::codegen::generators::trait_bridge::bridge_handle_path(api, bridge_cfg, core_import);

    let visitor_kwarg = bridge_cfg.param_name.as_deref().unwrap_or("visitor");
    let options_param = &bridge_match.param_name;
    let options_type = &bridge_match.options_type;
    let field_name = &bridge_match.field_name;

    // One answer to "is the options argument optional", used by the Rust signature, the serde
    // prelude, the `#[pyo3(signature = ...)]` list and the handle-injection body alike. The
    // three used to be spelled three different ways, and the body's spelling assumed `Option`
    // even when the signature had just declared a bare type (alef #476). ~keep
    let mut seen_optional = false;
    let optional_flags: Vec<bool> = func
        .params
        .iter()
        .map(|p| {
            if p.optional || matches!(&p.ty, TypeRef::Optional(_)) {
                seen_optional = true;
            }
            seen_optional
        })
        .collect();
    let options_is_optional = optional_flags
        .get(bridge_match.param_index)
        .copied()
        .unwrap_or(bridge_match.param_is_optional);

    let is_async = func.is_async && cfg.async_pattern == AsyncPattern::Pyo3FutureIntoPy;
    let lifetime = if is_async { "<'py>" } else { "" };

    let mut sig_parts = Vec::new();
    if is_async {
        sig_parts.push("py: Python<'py>".to_string());
    }
    for (p, is_optional) in func.params.iter().zip(&optional_flags) {
        let ty = if *is_optional {
            format!("Option<{}>", mapper.map_type(&p.ty))
        } else {
            mapper.map_type(&p.ty)
        };
        sig_parts.push(format!("{}: {}", p.name, ty));
    }
    sig_parts.push(format!("{visitor_kwarg}: Option<Py<PyAny>>"));

    let params_str = sig_parts.join(", ");
    let return_type = mapper.map_type(&func.return_type);
    let ret = if is_async {
        "PyResult<Bound<'py, PyAny>>".to_string()
    } else {
        mapper.wrap_return(&return_type, func.error_type.is_some())
    };

    let serde_err_conv = ".map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))";
    let serde_bindings: Vec<String> = func
        .params
        .iter()
        .zip(&optional_flags)
        .filter(|(p, _)| {
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
        .map(|(p, is_optional)| {
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
            if *is_optional {
                format!(
                    "let {name}_core: Option<{core_path}> = {name}.map(|v| {{ \
                     let json = serde_json::to_string(&v){serde_err_conv}?; \
                     serde_json::from_str(&json){serde_err_conv} }}).transpose()?;"
                )
            } else {
                format!(
                    "let {name}_json = serde_json::to_string(&{name}){serde_err_conv}?; \
                     let {name}_core: {core_path} = serde_json::from_str(&{name}_json){serde_err_conv}?;"
                )
            }
        })
        .collect();

    let core_options_type = format!("{core_import}::{options_type}");

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

    let wrap_expr = |var: &str| match &func.return_type {
        TypeRef::Named(name) if opaque_types.contains(name.as_str()) => {
            Some(format!("{name} {{ inner: std::sync::Arc::new({var}) }}"))
        }
        TypeRef::Named(_) | TypeRef::String | TypeRef::Bytes => Some(format!("{var}.into()")),
        _ => None,
    };
    let sync_return_wrap = wrap_expr("val");
    let async_return_wrap = wrap_expr("result");

    let err_conv = func.error_type.as_ref().map(|error_type| {
        if error_type.contains("::") || error_type == "Error" {
            if error_converters.len() == 1 {
                format!(".map_err({})", error_converters[0])
            } else {
                ".map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))".to_string()
            }
        } else {
            // Fall back to the generic conversion when no `{error}_to_py_err` was emitted --
            // naming one that does not exist is an E0425 in the generated crate. Mirrors the
            // same guard in `bridge_methods.rs`. ~keep
            let converter = format!("{}_to_py_err", crate::codegen::naming::pascal_to_snake(error_type));
            if error_converters.contains(&converter) {
                format!(".map_err({converter})")
            } else {
                ".map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))".to_string()
            }
        }
    });

    let attr_inner = cfg
        .function_attr
        .trim_start_matches('#')
        .trim_start_matches('[')
        .trim_end_matches(']');

    let sig_str = if cfg.needs_signature {
        func.params
            .iter()
            .zip(&optional_flags)
            .map(|(p, is_optional)| {
                if *is_optional {
                    format!("{}=None", p.name)
                } else {
                    p.name.clone()
                }
            })
            .chain(std::iter::once(format!("{visitor_kwarg}=None")))
            .collect::<Vec<_>>()
            .join(", ")
    } else {
        String::new()
    };

    // The bridge wrapper has two constructors: the visitor shape returns `Self`, every other
    // shape returns `PyResult<Self>` (it validates the host object's required methods). The
    // wrapper used to bind the constructor's return value directly and cast it to the handle
    // type, which is an E0277 for the fallible shape. An infallible wrapper has nowhere to
    // put the `PyErr`, so it surfaces as a panic -- pyo3 turns that into a
    // `PanicException` the caller can actually read, which is the point. ~keep
    let ctor_is_fallible = crate::codegen::generators::trait_bridge::find_trait_def(bridge_cfg, api)
        .is_none_or(|trait_def| !crate::backends::pyo3::trait_bridge::is_visitor_bridge(trait_def, bridge_cfg));
    let ctor_suffix = match (ctor_is_fallible, is_async || func.error_type.is_some()) {
        (false, _) => String::new(),
        (true, true) => "?".to_string(),
        (true, false) => format!(
            ".expect(\"`{visitor_kwarg}` does not provide the `{}` methods the bridge calls\")",
            bridge_cfg.trait_name
        ),
    };

    crate::backends::pyo3::template_env::render(
        "trait_bridge/options_field_wrapper.jinja",
        minijinja::context! {
            has_error => func.error_type.is_some(),
            attr_inner => attr_inner,
            needs_signature => cfg.needs_signature,
            signature_prefix => cfg.signature_prefix,
            sig_str => sig_str,
            signature_suffix => cfg.signature_suffix,
            func_name => &func.name,
            lifetime => lifetime,
            params_str => params_str,
            ret => ret,
            visitor_kwarg => visitor_kwarg,
            handle_path => handle_path,
            bridge_struct => bridge_struct,
            field_fallback => bridge_field_reaches_binding(api, bridge_cfg, field_name),
            field_access => options_field_access(options_param, field_name, options_is_optional),
            serde_bindings => serde_bindings,
            options_param => options_param,
            options_is_optional => options_is_optional,
            core_options_type => core_options_type,
            field_name => field_name,
            is_async => is_async,
            is_unit => matches!(func.return_type, TypeRef::Unit),
            needs_wrapped_binding => sync_return_wrap.is_some(),
            sync_return_wrap => sync_return_wrap.unwrap_or_else(|| "val".to_string()),
            async_return_wrap => async_return_wrap.unwrap_or_else(|| "result".to_string()),
            return_type => return_type,
            core_call => core_call,
            err_conv => err_conv.clone().unwrap_or_default(),
            err_try => if err_conv.is_some() { "?" } else { "" },
            ctor_suffix => ctor_suffix,
        },
    )
}
