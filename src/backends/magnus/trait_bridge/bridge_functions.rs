use crate::core::config::TraitBridgeConfig;
use crate::core::ir::ApiSurface;

/// A `Named` type, or a `Named` type behind one layer of `Optional`, extracts to its name;
/// anything else (primitives, `Vec`, opaque handles, deeper nesting) is not a candidate for the
/// `let {name}_core = ...` binding `gen_bridge_function` emits for non-opaque named params. Shared
/// by the fallibility check and the binding emission below so the two consult the exact same
/// notion of "named", instead of two hand-written pattern matches that can drift apart. ~keep
fn named_type_name(ty: &crate::core::ir::TypeRef) -> Option<&str> {
    match ty {
        crate::core::ir::TypeRef::Named(n) => Some(n.as_str()),
        crate::core::ir::TypeRef::Optional(inner) => match inner.as_ref() {
            crate::core::ir::TypeRef::Named(n) => Some(n.as_str()),
            _ => None,
        },
        _ => None,
    }
}

/// Use a companion `name: String` parameter as the bridge's cached plugin identity when present.
/// Per-call callback bridges generally have no identity and retain the historical empty name.
fn bridge_name_expr(func: &crate::core::ir::FunctionDef, bridge_param_idx: usize) -> &'static str {
    let has_name = func.params.iter().enumerate().any(|(idx, param)| {
        idx != bridge_param_idx
            && param.name == "name"
            && !param.optional
            && matches!(param.ty, crate::core::ir::TypeRef::String)
    });
    if has_name { "name.clone()" } else { "String::new()" }
}

/// Generate a Magnus free function that has one parameter replaced by `magnus::Value` (a trait
/// bridge). The bridge is constructed before calling the core function.
#[allow(clippy::too_many_arguments)]
pub fn gen_bridge_function(
    api: &ApiSurface,
    func: &crate::core::ir::FunctionDef,
    bridge_param_idx: usize,
    bridge_cfg: &TraitBridgeConfig,
    mapper: &dyn crate::codegen::type_mapper::TypeMapper,
    opaque_types: &ahash::AHashSet<String>,
    default_types: &std::collections::HashSet<&str>,
    core_import: &str,
) -> String {
    use crate::core::ir::TypeRef;

    let struct_name = crate::codegen::generators::trait_bridge::bridge_wrapper_name("Rb", bridge_cfg);
    let handle_path = crate::codegen::generators::trait_bridge::bridge_handle_path(api, bridge_cfg, core_import);
    let param_name = &func.params[bridge_param_idx].name;
    let bridge_param = &func.params[bridge_param_idx];
    let is_optional = bridge_param.optional || matches!(&bridge_param.ty, TypeRef::Optional(_));
    let bridge_name = bridge_name_expr(func, bridge_param_idx);
    let (bridge_handle_type, bridge_value) = if bridge_param.core_wrapper == crate::core::ir::CoreWrapper::Arc {
        (
            format!("std::sync::Arc<dyn {handle_path}>"),
            "std::sync::Arc::new(bridge)".to_string(),
        )
    } else {
        (
            handle_path.clone(),
            "std::sync::Arc::new(std::sync::Mutex::new(bridge))".to_string(),
        )
    };

    let mut sig_parts = Vec::new();
    for (idx, p) in func.params.iter().enumerate() {
        if idx == bridge_param_idx {
            if is_optional {
                sig_parts.push(format!("{}: Option<magnus::Value>", p.name));
            } else {
                sig_parts.push(format!("{}: magnus::Value", p.name));
            }
        } else {
            let promoted = (is_optional && idx > bridge_param_idx) || func.params[..idx].iter().any(|pp| pp.optional);
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

    // Non-bridge params that get a `let {name}_core = ...` binding below: a `Named`/`Optional<Named>`
    // param that isn't already an opaque handle. Computed once and reused by both the fallibility
    // check and `serde_bindings` itself, so they can't independently drift the way `has_error` used
    // to (it used to recompute a lookalike condition that dropped this entirely). ~keep
    let deser_params: Vec<_> = func
        .params
        .iter()
        .enumerate()
        .filter(|(idx, p)| {
            *idx != bridge_param_idx && named_type_name(&p.ty).is_some_and(|n| !opaque_types.contains(n))
        })
        .collect();

    // Of those, only a "default type" (`is_dt` below) skips the `?` — it gets a plain `.into()`.
    // Every other entry goes through a fallible `serde_json::from_str(...)?` / `.transpose()?`, so
    // the annotation must be `Result`-shaped whenever at least one does, independent of
    // `func.error_type`. ~keep
    let params_need_fallible_deser = deser_params
        .iter()
        .copied()
        .any(|(_, p)| !default_types.contains(named_type_name(&p.ty).unwrap_or_default()));

    let has_error = func.error_type.is_some() || params_need_fallible_deser;
    let ret = mapper.wrap_return(&return_type, has_error);

    let err_conv = ".map_err(|e| magnus::Error::new(unsafe { magnus::Ruby::get_unchecked() }.exception_runtime_error(), e.to_string()))";

    let bridge_wrap = if is_optional {
        format!(
            "let {param_name}: Option<{bridge_handle_type}> = match {param_name} {{\n        \
             Some(v) if !v.is_nil() => {{\n            \
             let bridge = {struct_name}::new(v, {bridge_name})?;\n            \
             Some({bridge_value} as {bridge_handle_type})\n        \
             }},\n        \
             _ => None,\n    \
             }};"
        )
    } else {
        format!(
            "let {param_name} = {{\n        \
             let bridge = {struct_name}::new({param_name}, {bridge_name})?;\n        \
             {bridge_value} as {bridge_handle_type}\n    \
             }};"
        )
    };

    let serde_bindings: String = deser_params
        .into_iter()
        .map(|(_, p)| {
            let name = &p.name;
            let named_type = named_type_name(&p.ty).unwrap_or_default().to_string();
            let core_path = format!("{core_import}::{named_type}");
            let is_dt = default_types.contains(named_type.as_str());
            if is_dt {
                if p.optional || matches!(&p.ty, TypeRef::Optional(_)) {
                    format!("let {name}_core: Option<{core_path}> = {name}.map(Into::into);\n    ")
                } else {
                    format!("let {name}_core: {core_path} = {name}.into();\n    ")
                }
            } else if p.optional || matches!(&p.ty, TypeRef::Optional(_)) {
                format!(
                    "let {name}_core: Option<{core_path}> = {name}.as_deref().filter(|s| *s != \"nil\").map(|s| serde_json::from_str(s){err_conv}).transpose()?;\n    "
                )
            } else {
                format!(
                    "let {name}_core: {core_path} = serde_json::from_str(&{name}){err_conv}?;\n    "
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

    let body = if func.error_type.is_some() {
        // The core call itself already returns `Result`, so chaining `.map`/`.map_err` directly
        // onto it is already `Result`-typed — no extra `Ok(..)` wrap needed. ~keep
        if return_wrap == "val" {
            format!("{bridge_wrap}\n    {serde_bindings}{core_call}{err_conv}")
        } else {
            format!("{bridge_wrap}\n    {serde_bindings}{core_call}.map(|val| {return_wrap}){err_conv}")
        }
    } else if has_error {
        // The core call returns a bare value, but `serde_bindings` above used `?` on at least one
        // param (`params_need_fallible_deser`), which forced the signature to be `Result`-shaped.
        // The tail expression must match that: wrap the plain call in `Ok(..)` instead of handing
        // back the bare value the `Result<T, Error>` signature above no longer fits. ~keep
        if return_wrap == "val" {
            format!("{bridge_wrap}\n    {serde_bindings}Ok({core_call})")
        } else {
            format!("{bridge_wrap}\n    {serde_bindings}let val = {core_call};\n    Ok({return_wrap})")
        }
    } else {
        format!("{bridge_wrap}\n    {serde_bindings}{core_call}")
    };

    let func_name = &func.name;
    let mut out = String::with_capacity(1024);
    if func.error_type.is_some() {
        out.push_str("#[allow(clippy::missing_errors_doc)]\n");
    }
    out.push_str("#[allow(unused_variables)]\n");
    let sig = format!("pub fn {func_name}({params_str}) -> {ret} {{\n    {body}\n}}\n");
    out.push_str(&sig);

    out
}
