use crate::core::config::TraitBridgeConfig;
use crate::core::ir::ApiSurface;

#[allow(clippy::too_many_arguments)]
pub fn gen_options_field_bridge_function(
    api: &ApiSurface,
    func: &crate::core::ir::FunctionDef,
    options_param_idx: usize,
    bridge_cfg: &TraitBridgeConfig,
    mapper: &dyn crate::codegen::type_mapper::TypeMapper,
    _cfg: &crate::codegen::generators::RustBindingConfig<'_>,
    opaque_types: &ahash::AHashSet<String>,
    core_import: &str,
) -> String {
    use crate::core::ir::TypeRef;

    let struct_name = crate::codegen::generators::trait_bridge::bridge_wrapper_name("Js", bridge_cfg);
    let handle_path = crate::codegen::generators::trait_bridge::bridge_handle_path(api, bridge_cfg, core_import);
    let options_param = &func.params[options_param_idx];
    let options_name = &options_param.name;

    let ir_param_optional = matches!(&options_param.ty, TypeRef::Optional(_));

    let visitor_kwarg = bridge_cfg.param_name.as_deref().unwrap_or("visitor");
    let field_name = bridge_cfg.resolved_options_field().unwrap_or(visitor_kwarg);
    let options_type = bridge_cfg
        .options_type
        .as_deref()
        .unwrap_or_else(|| match &options_param.ty {
            TypeRef::Named(name) => name.as_str(),
            TypeRef::Optional(inner) => match inner.as_ref() {
                TypeRef::Named(name) => name.as_str(),
                _ => "Options",
            },
            _ => "Options",
        });
    let options_path = format!("{core_import}::{options_type}");

    let params_str = {
        let mut sig_parts = vec![];
        for (i, p) in func.params.iter().enumerate() {
            let ty = mapper.map_type(&p.ty);
            if i == options_param_idx && !ir_param_optional {
                sig_parts.push(format!("{}: Option<{ty}>", p.name));
            } else {
                sig_parts.push(format!("{}: {ty}", p.name));
            }
        }
        sig_parts.join(", ")
    };

    let return_type = mapper.map_type(&func.return_type);
    let ret = mapper.wrap_return(&return_type, func.error_type.is_some());

    let err_conv = ".map_err(|e| napi::Error::new(napi::Status::GenericFailure, e.to_string()))";

    let call_args: String = func
        .params
        .iter()
        .enumerate()
        .map(|(idx, p)| {
            if idx == options_param_idx {
                format!("{options_name}_core")
            } else {
                match &p.ty {
                    TypeRef::Named(n) if opaque_types.contains(n.as_str()) => {
                        if p.optional {
                            format!("{}.as_ref().map(|v| &v.inner)", p.name)
                        } else {
                            format!("&{}.inner", p.name)
                        }
                    }
                    TypeRef::Named(_) => format!("{}.into()", p.name),
                    TypeRef::Optional(inner) => {
                        if let TypeRef::Named(n) = inner.as_ref() {
                            if opaque_types.contains(n.as_str()) {
                                format!("{}.as_ref().map(|v| &v.inner)", p.name)
                            } else {
                                format!("{}.map(Into::into)", p.name)
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
            }
        })
        .collect::<Vec<_>>()
        .join(", ");

    let core_fn_path = {
        let path = func.rust_path.replace('-', "_");
        if path.starts_with(core_import) {
            path
        } else {
            format!("{core_import}::{}", func.name)
        }
    };
    let core_call = format!("{core_fn_path}({call_args})");

    let return_wrap = match &func.return_type {
        TypeRef::Named(name) if opaque_types.contains(name.as_str()) => {
            format!("{name} {{ inner: std::sync::Arc::new(val) }}")
        }
        TypeRef::Named(_) => "val.into()".to_string(),
        TypeRef::String | TypeRef::Bytes => "val.into()".to_string(),
        _ => "val".to_string(),
    };

    let body = crate::backends::napi::template_env::render(
        "options_field_bridge_body.jinja",
        minijinja::context! {
            has_error => func.error_type.is_some(),
            maps_return => return_wrap != "val",
            visitor_kwarg => visitor_kwarg,
            handle_path => handle_path,
            struct_name => struct_name,
            options_name => options_name,
            options_path => options_path,
            field_name => field_name,
            core_call => core_call,
            err_conv => err_conv,
            return_wrap => return_wrap,
        },
    );

    let mut out = String::with_capacity(1024);
    if func.error_type.is_some() {
        out.push_str("#[allow(clippy::missing_errors_doc)]\n");
    }
    out.push_str("#[napi]\n");
    let func_name = &func.name;
    out.push_str(&crate::backends::napi::template_env::render(
        "trait_bridge_fn_wrapper.jinja",
        minijinja::context! {
            func_name => func_name,
            params_str => params_str,
            return_type => ret,
            body => body,
        },
    ));

    out
}
