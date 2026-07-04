use crate::core::config::TraitBridgeConfig;
use crate::core::ir::ApiSurface;

/// Generate a Rustler NIF function that has one parameter replaced by
/// `Option<rustler::Term<'_>>` (a trait bridge). The bridge is constructed before
/// calling the core function.
#[allow(clippy::too_many_arguments)]
pub fn gen_bridge_function(
    api: &ApiSurface,
    func: &crate::core::ir::FunctionDef,
    bridge_param_idx: usize,
    bridge_cfg: &TraitBridgeConfig,
    mapper: &dyn crate::codegen::type_mapper::TypeMapper,
    opaque_types: &ahash::AHashSet<String>,
    default_types: &ahash::AHashSet<String>,
    core_import: &str,
) -> String {
    use crate::core::ir::TypeRef;

    let struct_name = format!("Elixir{}Bridge", bridge_cfg.trait_name);
    let handle_path = crate::codegen::generators::trait_bridge::bridge_handle_path(api, bridge_cfg, core_import);
    let param_name = &func.params[bridge_param_idx].name;
    let bridge_param = &func.params[bridge_param_idx];
    let is_optional = bridge_param.optional || matches!(&bridge_param.ty, TypeRef::Optional(_));

    // Build parameter list — Rustler NIFs always have `env: rustler::Env<'_>` as first param
    let mut sig_parts = vec!["env: rustler::Env<'_>".to_string()];
    for (idx, p) in func.params.iter().enumerate() {
        if idx == bridge_param_idx {
            if is_optional {
                sig_parts.push(format!("{}: Option<rustler::Term<'_>>", p.name));
            } else {
                sig_parts.push(format!("{}: rustler::Term<'_>", p.name));
            }
        } else {
            // Use the same type mapping as gen_nif_function
            if let TypeRef::Named(n) = &p.ty {
                if opaque_types.contains(n) {
                    let promoted = idx > bridge_param_idx;
                    if promoted || p.optional {
                        sig_parts.push(format!("{}: Option<rustler::ResourceArc<{}>>", p.name, n));
                    } else {
                        sig_parts.push(format!("{}: rustler::ResourceArc<{}>", p.name, n));
                    }
                    continue;
                }
                if default_types.contains(n) {
                    sig_parts.push(format!("{}: Option<String>", p.name));
                    continue;
                }
            }
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

    let err_conv = ".map_err(|e| e.to_string())";

    // Bridge wrapping code
    let bridge_wrap_template = if is_optional {
        "trait_optional_bridge_wrap.rs.jinja"
    } else {
        "trait_required_bridge_wrap.rs.jinja"
    };
    let bridge_wrap = crate::backends::rustler::template_env::render(
        bridge_wrap_template,
        minijinja::context! {
            param_name => param_name,
            handle_path => handle_path,
            struct_name => struct_name,
        },
    )
    .trim_end()
    .to_string();

    // Let bindings for non-bridge params that need deserialization
    let deser_bindings: String = func
        .params
        .iter()
        .enumerate()
        .filter(|(idx, p)| {
            if *idx == bridge_param_idx {
                return false;
            }
            match &p.ty {
                TypeRef::Named(n) => !opaque_types.contains(n.as_str()),
                TypeRef::Optional(inner) => {
                    if let TypeRef::Named(n) = inner.as_ref() {
                        !opaque_types.contains(n.as_str())
                    } else {
                        false
                    }
                }
                _ => false,
            }
        })
        .map(|(_, p)| {
            let name = &p.name;
            if let TypeRef::Named(n) = &p.ty {
                if default_types.contains(n) {
                    let core_ty = format!("{core_import}::{n}");
                    return format!(
                        "let {name}_core: Option<{core_ty}> = {name}.map(|s| serde_json::from_str::<{core_ty}>(&s){err_conv}).transpose(){err_conv}?;\n    "
                    );
                }
                let core_ty = format!("{core_import}::{n}");
                if p.optional {
                    return format!("let {name}_core: Option<{core_ty}> = {name}.map(Into::into);\n    ");
                }
                return format!("let {name}_core: {core_ty} = {name}.into();\n    ");
            }
            String::new()
        })
        .collect();

    // Build call args
    let call_args: Vec<String> = func
        .params
        .iter()
        .enumerate()
        .map(|(idx, p)| {
            if idx == bridge_param_idx {
                return p.name.clone();
            }
            if let TypeRef::Named(n) = &p.ty {
                if opaque_types.contains(n.as_str()) {
                    if p.optional {
                        return format!(
                            "{}.as_ref().map(|v| &v.inner.read().unwrap_or_else(|e| e.into_inner()).clone())",
                            p.name
                        );
                    }
                    return format!("&{}.inner.read().unwrap_or_else(|e| e.into_inner()).clone()", p.name);
                }
                if default_types.contains(n) {
                    return format!("{}_core", p.name);
                }
            }
            match &p.ty {
                TypeRef::Named(_) => format!("{}_core", p.name),
                TypeRef::Optional(inner) => {
                    if let TypeRef::Named(n) = inner.as_ref() {
                        if opaque_types.contains(n.as_str()) {
                            format!(
                                "{}.as_ref().map(|v| &v.inner.read().unwrap_or_else(|e| e.into_inner()).clone())",
                                p.name
                            )
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
            format!(
                "rustler::ResourceArc::new({name}Inner {{ inner: std::sync::Arc::new(std::sync::RwLock::new(val)) }})"
            )
        }
        TypeRef::Named(_) => "val.into()".to_string(),
        TypeRef::String | TypeRef::Bytes => "val.into()".to_string(),
        _ => "val".to_string(),
    };

    let body = if func.error_type.is_some() {
        if return_wrap == "val" {
            format!("{bridge_wrap}\n    {deser_bindings}{core_call}{err_conv}")
        } else {
            format!("{bridge_wrap}\n    {deser_bindings}{core_call}.map(|val| {return_wrap}){err_conv}")
        }
    } else {
        format!("{bridge_wrap}\n    {deser_bindings}{core_call}")
    };

    let func_name = &func.name;
    let mut out = String::with_capacity(2048);
    let ctx = minijinja::context! {
        func_name => func_name,
        params_str => params_str,
        ret => ret,
        body => body
    };
    out.push_str(&crate::backends::rustler::template_env::render(
        "nif_function.rs.jinja",
        ctx,
    ));

    // Generate the async visitor NIF only when the bridge parameter is the visitor.
    // This NIF spawns a system thread, builds the bridge from the caller PID + visitor term,
    // runs conversion, and sends the result back as a {:ok, result} / {:error, reason} message.
    if is_optional {
        // Build the non-bridge params signature for convert_with_visitor
        // (bridge param replaced by the concrete Elixir term — never nil here).
        let mut with_sig_parts = vec!["env: rustler::Env<'_>".to_string()];
        for (idx, p) in func.params.iter().enumerate() {
            if idx == bridge_param_idx {
                // visitor is required (not optional) in convert_with_visitor
                with_sig_parts.push(format!("{}: rustler::Term<'_>", p.name));
            } else if let TypeRef::Named(n) = &p.ty {
                if default_types.contains(n) {
                    with_sig_parts.push(format!("{}: Option<String>", p.name));
                } else {
                    let mapped = mapper.map_type(&p.ty);
                    if p.optional {
                        with_sig_parts.push(format!("{}: Option<{}>", p.name, mapped));
                    } else {
                        with_sig_parts.push(format!("{}: {}", p.name, mapped));
                    }
                }
            } else {
                let mapped = mapper.map_type(&p.ty);
                if p.optional {
                    with_sig_parts.push(format!("{}: Option<{}>", p.name, mapped));
                } else {
                    with_sig_parts.push(format!("{}: {}", p.name, mapped));
                }
            }
        }
        let with_params_str = with_sig_parts.join(", ");

        // Build the deser bindings string for non-bridge, non-opaque named params.
        let with_deser: String = func
            .params
            .iter()
            .enumerate()
            .filter(|(idx, _)| *idx != bridge_param_idx)
            .filter_map(|(_, p)| {
                if let TypeRef::Named(n) = &p.ty {
                    if default_types.contains(n) {
                        let core_ty = format!("{core_import}::{n}");
                        return Some(format!(
                            "let {0}_core: Option<{1}> = {0}.map(|s| serde_json::from_str::<{1}>(&s).map_err(|e| e.to_string())).transpose().map_err(|e| e.to_string())?;\n    ",
                            p.name, core_ty
                        ));
                    }
                }
                None
            })
            .collect();

        // Build the call args, replacing the bridge param with the configured handle.
        let with_call_args: Vec<String> = func
            .params
            .iter()
            .enumerate()
            .map(|(idx, p)| {
                if idx == bridge_param_idx {
                    // visitor_handle is built inside the thread closure
                    p.name.clone()
                } else if let TypeRef::Named(n) = &p.ty {
                    if default_types.contains(n) {
                        format!("{}_core", p.name)
                    } else {
                        match &p.ty {
                            TypeRef::String | TypeRef::Char if p.is_ref => format!("&{}", p.name),
                            _ => p.name.clone(),
                        }
                    }
                } else {
                    match &p.ty {
                        TypeRef::String | TypeRef::Char if p.is_ref => format!("&{}", p.name),
                        _ => p.name.clone(),
                    }
                }
            })
            .collect();
        let with_call_args_str = with_call_args.join(", ");

        // Clone non-bridge params before moving into the thread.
        let clone_stmts: String = func
            .params
            .iter()
            .enumerate()
            .filter(|(idx, _)| *idx != bridge_param_idx)
            .map(|(_, p)| {
                if let TypeRef::Named(n) = &p.ty {
                    if default_types.contains(n) {
                        return format!("let {0}_core = {0}_core;\n    ", p.name);
                    }
                }
                match &p.ty {
                    TypeRef::String | TypeRef::Char => format!("let {0} = {0}.clone();\n    ", p.name),
                    _ => String::new(),
                }
            })
            .collect();

        out.push('\n');
        let ctx = minijinja::context! {
            func_name => func_name,
            with_params_str => with_params_str,
            with_deser => with_deser,
            param_name => param_name,
            clone_stmts => clone_stmts,
            struct_name => struct_name,
            handle_path => handle_path,
            core_fn_path => core_fn_path,
            with_call_args_str => with_call_args_str,
            return_type => return_type.clone(),
        };
        out.push_str(&crate::backends::rustler::template_env::render(
            "nif_with_visitor_async_body.rs.jinja",
            ctx,
        ));
    }

    out
}

/// Generate NIF functions for an `options_field` visitor bridge.
///
/// For `options_field` bridges the visitor is embedded in the options struct
/// rather than being a direct function parameter.  We generate two NIFs:
///
/// 1. The plain NIF: `fn convert(html, options)` — no visitor, just deserialises
///    options and calls the core function directly (same as `gen_nif_function`).
///
/// 2. The async visitor NIF: `fn convert_with_visitor(env, html, options, visitor)`
///    — pops the visitor from the Elixir caller, builds the bridge struct, injects
///    it as `options.visitor`, then spawns a system thread, runs conversion, and
///    sends the result as `{:ok, result}` / `{:error, reason}` to the BEAM process.
///
/// The Elixir public-API wrapper in `sample_markdown.ex` calls
/// `Native.convert_with_visitor(html, clean_opts, visitor)` when a visitor map
/// is present, or falls back to `Native.convert(html, opts_json)` otherwise.
#[allow(clippy::too_many_arguments)]
pub fn gen_bridge_field_function(
    api: &ApiSurface,
    func: &crate::core::ir::FunctionDef,
    bridge_match: &crate::codegen::generators::trait_bridge::BridgeFieldMatch<'_>,
    bridge_cfg: &TraitBridgeConfig,
    mapper: &crate::backends::rustler::type_map::RustlerMapper,
    opaque_types: &ahash::AHashSet<String>,
    _default_types: &ahash::AHashSet<String>,
    core_import: &str,
) -> String {
    use crate::codegen::type_mapper::TypeMapper;
    use crate::core::ir::TypeRef;

    let struct_name = format!("Elixir{}Bridge", bridge_cfg.trait_name);
    let handle_path = crate::codegen::generators::trait_bridge::bridge_handle_path(api, bridge_cfg, core_import);
    let func_name = &func.name;
    let field_name = &bridge_match.field_name;
    let options_param = &bridge_match.param_name;
    let options_type = &bridge_match.options_type;
    let core_options_type = format!("{core_import}::options::{options_type}");

    // Whether the core function expects Option<ParseOptions> (optional=true).
    let options_param_is_optional = func
        .params
        .iter()
        .find(|p| p.name == *options_param)
        .is_some_and(|p| p.optional || matches!(&p.ty, TypeRef::Optional(_)));

    let core_fn_path = {
        let path = func.rust_path.replace('-', "_");
        if path.starts_with(core_import) {
            path
        } else {
            format!("{core_import}::{}", func.name)
        }
    };

    // ── 1. Plain NIF (no visitor) ─────────────────────────────────────────────
    // Parameters: all original params. Options type is passed as Option<String> (JSON).
    let mut plain_sig: Vec<String> = Vec::new();
    for p in &func.params {
        let ty = if p.name == *options_param {
            "Option<String>".to_string()
        } else if p.optional || matches!(&p.ty, TypeRef::Optional(_)) {
            format!("Option<{}>", mapper.map_type(&p.ty))
        } else if let TypeRef::Named(n) = &p.ty {
            if opaque_types.contains(n) {
                format!("rustler::ResourceArc<{n}>")
            } else {
                mapper.map_type(&p.ty)
            }
        } else {
            mapper.map_type(&p.ty)
        };
        plain_sig.push(format!("{}: {}", p.name, ty));
    }
    let plain_params_str = plain_sig.join(", ");

    let return_type = mapper.map_type(&func.return_type);
    let ret = mapper.wrap_return(&return_type, func.error_type.is_some());
    let err_conv = ".map_err(|e| e.to_string())";

    // Build call args for the plain NIF.
    let plain_call_args: Vec<String> = func
        .params
        .iter()
        .map(|p| {
            if p.name == *options_param {
                // Deserialise options JSON → core type.
                // When the core param is Option<T>, keep it wrapped; otherwise unwrap to Default.
                if options_param_is_optional {
                    format!(
                        "{options_param}.map(|s| serde_json::from_str::<{core_options_type}>(&s).unwrap_or_default())"
                    )
                } else {
                    format!(
                        "{options_param}.map(|s| serde_json::from_str::<{core_options_type}>(&s).unwrap_or_default()).unwrap_or_default()"
                    )
                }
            } else {
                match &p.ty {
                    TypeRef::Named(n) if opaque_types.contains(n) => format!("&{}.inner.read().unwrap_or_else(|e| e.into_inner()).clone()", p.name),
                    TypeRef::String | TypeRef::Char if p.is_ref => format!("&{}", p.name),
                    _ => p.name.clone(),
                }
            }
        })
        .collect();
    let plain_call_args_str = plain_call_args.join(", ");

    let plain_body = if func.error_type.is_some() {
        format!("{core_fn_path}({plain_call_args_str})\n        .map(|val| val.into()){err_conv}")
    } else {
        format!("{core_fn_path}({plain_call_args_str}).into()")
    };

    let mut out = String::with_capacity(2048);
    let ctx = minijinja::context! {
        func_name => func_name,
        params_str => plain_params_str,
        ret => ret,
        body => plain_body
    };
    out.push_str(&crate::backends::rustler::template_env::render(
        "dirty_cpu_nif_function.rs.jinja",
        ctx,
    ));

    // ── 2. Async visitor NIF ──────────────────────────────────────────────────
    // Signature: env + original params + `visitor: Term<'_>` at the end.
    let mut vis_sig: Vec<String> = vec!["env: rustler::Env<'_>".to_string()];
    for p in &func.params {
        let ty = if p.name == *options_param {
            "Option<String>".to_string()
        } else if p.optional || matches!(&p.ty, TypeRef::Optional(_)) {
            format!("Option<{}>", mapper.map_type(&p.ty))
        } else if let TypeRef::Named(n) = &p.ty {
            if opaque_types.contains(n) {
                format!("rustler::ResourceArc<{n}>")
            } else {
                mapper.map_type(&p.ty)
            }
        } else {
            mapper.map_type(&p.ty)
        };
        vis_sig.push(format!("{}: {}", p.name, ty));
    }
    vis_sig.push("visitor: rustler::Term<'_>".to_string());
    let vis_params_str = vis_sig.join(", ");

    // Clone stmts for non-String params that need to move into thread.
    let clone_stmts: String = func
        .params
        .iter()
        .map(|p| {
            if p.name == *options_param {
                return String::new();
            }
            match &p.ty {
                TypeRef::String | TypeRef::Char => format!("let {} = {}.clone();\n    ", p.name, p.name),
                _ => String::new(),
            }
        })
        .collect();

    // Deser stmts: parse options JSON and set visitor field.
    let deser_stmts = crate::backends::rustler::template_env::render(
        "visitor_field_options_setup.rs.jinja",
        minijinja::context! {
            options_param => options_param,
            core_options_type => core_options_type,
            struct_name => struct_name,
            field_name => field_name,
            handle_path => handle_path,
        },
    );

    // Build call args for the visitor variant.
    let vis_call_args: Vec<String> = func
        .params
        .iter()
        .map(|p| {
            if p.name == *options_param {
                // Core expects Option<T> when optional=true.
                if options_param_is_optional {
                    format!("Some({options_param}_core)")
                } else {
                    format!("{options_param}_core")
                }
            } else {
                match &p.ty {
                    TypeRef::Named(n) if opaque_types.contains(n) => {
                        format!("&{}.inner.read().unwrap_or_else(|e| e.into_inner()).clone()", p.name)
                    }
                    TypeRef::String | TypeRef::Char if p.is_ref => format!("&{}", p.name),
                    _ => p.name.clone(),
                }
            }
        })
        .collect();
    let vis_call_args_str = vis_call_args.join(", ");

    out.push('\n');
    let ctx = minijinja::context! {
        func_name => func_name,
        vis_params_str => vis_params_str,
        clone_stmts => clone_stmts,
        deser_stmts => deser_stmts,
        core_fn_path => core_fn_path,
        vis_call_args_str => vis_call_args_str,
        return_type => return_type.clone(),
    };
    out.push_str(&crate::backends::rustler::template_env::render(
        "nif_with_visitor_field_async_body.rs.jinja",
        ctx,
    ));

    out
}
