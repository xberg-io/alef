//! Elixir (Rustler) specific trait bridge code generation.
//!
//! Generates Rust wrapper structs that implement Rust traits by delegating
//! to Elixir module-based callbacks via Rustler term dispatch.
//!
//! Because Elixir NIFs cannot call back into arbitrary Elixir code during a NIF call
//! without dirty scheduling, the visitor bridge here accepts an Elixir map
//! (`rustler::Term`) that encodes visitor overrides as function references
//! (anonymous functions / `fn/arity` captures). The bridge calls each override
//! via `rustler::Env::run_gc()` — this is the closest Rustler equivalent to the
//! PyO3/NAPI callback patterns.
//!
//! For the generated code this means the bridge param becomes `Option<rustler::Term<'_>>`.

use alef_core::config::TraitBridgeConfig;
use alef_core::ir::{ApiSurface, MethodDef, TypeDef, TypeRef};
use std::fmt::Write;

/// Generate all trait bridge code for a given trait type and bridge config.
pub fn gen_trait_bridge(
    trait_type: &TypeDef,
    bridge_cfg: &TraitBridgeConfig,
    core_import: &str,
    api: &ApiSurface,
) -> String {
    let mut out = String::with_capacity(8192);
    let struct_name = format!("Elixir{}Bridge", bridge_cfg.trait_name);
    let trait_path = trait_type.rust_path.replace('-', "_");

    // Build type name → rust_path lookup
    let type_paths: std::collections::HashMap<&str, &str> = api
        .types
        .iter()
        .map(|t| (t.name.as_str(), t.rust_path.as_str()))
        .chain(api.enums.iter().map(|e| (e.name.as_str(), e.rust_path.as_str())))
        .collect();

    // Visitor-style bridge: all methods have defaults, no registry, no super-trait.
    let is_visitor_bridge = bridge_cfg.type_alias.is_some()
        && bridge_cfg.register_fn.is_none()
        && bridge_cfg.super_trait.is_none()
        && trait_type.methods.iter().all(|m| m.has_default_impl);

    if is_visitor_bridge {
        gen_visitor_bridge(
            &mut out,
            trait_type,
            bridge_cfg,
            &struct_name,
            &trait_path,
            core_import,
            &type_paths,
        );
    }

    out
}

/// Generate a visitor-style bridge wrapping a `rustler::OwnedEnv` + `rustler::Term`.
///
/// The Elixir caller passes a map where keys are atom method names and values are
/// anonymous functions (closures) that accept the NodeContext map and return an atom string.
/// The bridge looks up each method name in the map and, if found, calls the function.
fn gen_visitor_bridge(
    out: &mut String,
    trait_type: &TypeDef,
    _bridge_cfg: &TraitBridgeConfig,
    struct_name: &str,
    trait_path: &str,
    core_crate: &str,
    type_paths: &std::collections::HashMap<&str, &str>,
) {
    // Helper: convert NodeContext to a Rustler NifMap term inside an OwnedEnv
    writeln!(out, "fn nodecontext_to_elixir_map<'a>(").unwrap();
    writeln!(out, "    env: rustler::Env<'a>,").unwrap();
    writeln!(out, "    ctx: &{core_crate}::visitor::NodeContext,").unwrap();
    writeln!(out, ") -> rustler::Term<'a> {{").unwrap();
    writeln!(
        out,
        "    let mut pairs: Vec<(rustler::Term<'a>, rustler::Term<'a>)> = Vec::new();"
    )
    .unwrap();
    writeln!(
        out,
        "    pairs.push((rustler::types::atom::Atom::from_str(env, \"node_type\").unwrap().to_term(env), format!(\"{{:?}}\", ctx.node_type).encode(env)));"
    )
    .unwrap();
    writeln!(
        out,
        "    pairs.push((rustler::types::atom::Atom::from_str(env, \"tag_name\").unwrap().to_term(env), ctx.tag_name.encode(env)));"
    )
    .unwrap();
    writeln!(
        out,
        "    pairs.push((rustler::types::atom::Atom::from_str(env, \"depth\").unwrap().to_term(env), (ctx.depth as i64).encode(env)));"
    )
    .unwrap();
    writeln!(
        out,
        "    pairs.push((rustler::types::atom::Atom::from_str(env, \"index_in_parent\").unwrap().to_term(env), (ctx.index_in_parent as i64).encode(env)));"
    )
    .unwrap();
    writeln!(
        out,
        "    pairs.push((rustler::types::atom::Atom::from_str(env, \"is_inline\").unwrap().to_term(env), ctx.is_inline.encode(env)));"
    )
    .unwrap();
    writeln!(
        out,
        "    let parent_tag_term = match &ctx.parent_tag {{ Some(s) => s.encode(env), None => rustler::types::atom::Atom::from_str(env, \"nil\").unwrap().to_term(env) }};"
    )
    .unwrap();
    writeln!(
        out,
        "    pairs.push((rustler::types::atom::Atom::from_str(env, \"parent_tag\").unwrap().to_term(env), parent_tag_term));"
    )
    .unwrap();
    writeln!(
        out,
        "    let attrs_pairs: Vec<(rustler::Term<'a>, rustler::Term<'a>)> = ctx.attributes.iter().map(|(k, v)| (k.encode(env), v.encode(env))).collect();"
    )
    .unwrap();
    writeln!(
        out,
        "    let attrs_map = rustler::Term::map_from_pairs(env, &attrs_pairs).unwrap_or_else(|_| rustler::types::atom::Atom::from_str(env, \"nil\").unwrap().to_term(env));"
    )
    .unwrap();
    writeln!(
        out,
        "    pairs.push((rustler::types::atom::Atom::from_str(env, \"attributes\").unwrap().to_term(env), attrs_map));"
    )
    .unwrap();
    writeln!(
        out,
        "    rustler::Term::map_from_pairs(env, &pairs).unwrap_or_else(|_| rustler::types::atom::Atom::from_str(env, \"nil\").unwrap().to_term(env))"
    )
    .unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // Bridge struct holds an OwnedEnv (for lifetime extension) and the visitor term
    writeln!(out, "pub struct {struct_name} {{").unwrap();
    writeln!(out, "    env: rustler::OwnedEnv,").unwrap();
    writeln!(out, "    visitor_term: rustler::SavedTerm,").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // Manual Debug impl
    writeln!(out, "impl std::fmt::Debug for {struct_name} {{").unwrap();
    writeln!(
        out,
        "    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{"
    )
    .unwrap();
    writeln!(out, "        write!(f, \"{struct_name}\")").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // Constructor
    writeln!(out, "impl {struct_name} {{").unwrap();
    writeln!(
        out,
        "    pub fn new(env: rustler::Env<'_>, visitor_term: rustler::Term<'_>) -> Self {{"
    )
    .unwrap();
    writeln!(out, "        let owned = rustler::OwnedEnv::new();").unwrap();
    writeln!(out, "        let saved = owned.save(visitor_term);").unwrap();
    writeln!(out, "        Self {{ env: owned, visitor_term: saved }}").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // Trait impl
    writeln!(out, "impl {trait_path} for {struct_name} {{").unwrap();
    for method in &trait_type.methods {
        if method.trait_source.is_some() {
            continue;
        }
        gen_visitor_method_rustler(out, method, type_paths);
    }
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}

/// Map a visitor method parameter type to the correct Rust type string.
fn visitor_param_type(
    ty: &TypeRef,
    is_ref: bool,
    optional: bool,
    tp: &std::collections::HashMap<&str, &str>,
) -> String {
    if optional && matches!(ty, TypeRef::String) && is_ref {
        return "Option<&str>".to_string();
    }
    if is_ref {
        if let TypeRef::Vec(inner) = ty {
            let inner_str = param_type(inner, "", false, tp);
            return format!("&[{inner_str}]");
        }
    }
    param_type(ty, "", is_ref, tp)
}

/// Generate a single visitor method that looks up the method atom in the visitor map
/// and calls the stored anonymous function.
fn gen_visitor_method_rustler(
    out: &mut String,
    method: &MethodDef,
    type_paths: &std::collections::HashMap<&str, &str>,
) {
    let name = &method.name;

    let mut sig_parts = vec!["&mut self".to_string()];
    for p in &method.params {
        let ty_str = visitor_param_type(&p.ty, p.is_ref, p.optional, type_paths);
        sig_parts.push(format!("{}: {}", p.name, ty_str));
    }
    let sig = sig_parts.join(", ");

    let ret_ty = match &method.return_type {
        TypeRef::Named(n) => type_paths
            .get(n.as_str())
            .map(|p| p.replace('-', "_"))
            .unwrap_or_else(|| n.clone()),
        other => param_type(other, "", false, type_paths),
    };

    writeln!(out, "    fn {name}({sig}) -> {ret_ty} {{").unwrap();

    // Run in the owned env
    writeln!(out, "        let result_str = self.env.run(|env| {{").unwrap();
    writeln!(out, "            let visitor = self.visitor_term.load(env);").unwrap();
    writeln!(
        out,
        "            let key = rustler::types::atom::Atom::from_str(env, \"{name}\").ok()?;"
    )
    .unwrap();
    writeln!(
        out,
        "            let func_term: rustler::Term<'_> = rustler::Term::map_get(visitor, key).ok()??;"
    )
    .unwrap();
    writeln!(out, "            if func_term.is_nil() {{ return None; }}").unwrap();

    // Build the args tuple encoding
    if method.params.is_empty() {
        writeln!(out, "            let args: Vec<rustler::Term<'_>> = Vec::new();").unwrap();
    } else {
        writeln!(out, "            let args: Vec<rustler::Term<'_>> = vec![").unwrap();
        for p in &method.params {
            let arg = build_rustler_arg(p);
            writeln!(out, "                {arg},").unwrap();
        }
        writeln!(out, "            ];").unwrap();
    }

    writeln!(
        out,
        "            let result: rustler::Term<'_> = rustler::types::Pid::spawn_monitor(env, func_term, args.as_slice()).ok()?.0;"
    )
    .unwrap();
    writeln!(out, "            result.decode::<String>().ok()").unwrap();
    writeln!(out, "        }});").unwrap();

    // Parse result
    writeln!(out, "        match result_str {{").unwrap();
    writeln!(out, "            None | Some(Err(_)) => {ret_ty}::Continue,").unwrap();
    writeln!(out, "            Some(Ok(s)) => match s.to_lowercase().as_str() {{").unwrap();
    writeln!(out, "                \"continue\" => {ret_ty}::Continue,").unwrap();
    writeln!(out, "                \"skip\" => {ret_ty}::Skip,").unwrap();
    writeln!(
        out,
        "                \"preserve_html\" | \"preservehtml\" => {ret_ty}::PreserveHtml,"
    )
    .unwrap();
    writeln!(out, "                other => {ret_ty}::Custom(other.to_string()),").unwrap();
    writeln!(out, "            }},").unwrap();
    writeln!(out, "        }}").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out).unwrap();
}

/// Build a single Rustler arg expression encoded as a term.
fn build_rustler_arg(p: &alef_core::ir::ParamDef) -> String {
    if let TypeRef::Named(n) = &p.ty {
        if n == "NodeContext" {
            return format!(
                "nodecontext_to_elixir_map(env, {}{})",
                if p.is_ref { "" } else { "&" },
                p.name
            );
        }
    }
    if matches!(&p.ty, TypeRef::String) && p.is_ref {
        return format!("{}.encode(env)", p.name);
    }
    if matches!(&p.ty, TypeRef::String) {
        return format!("{}.as_str().encode(env)", p.name);
    }
    if p.optional && matches!(&p.ty, TypeRef::String) && p.is_ref {
        return format!(
            "match {} {{ Some(s) => s.encode(env), None => rustler::types::atom::Atom::from_str(env, \"nil\").unwrap().to_term(env) }}",
            p.name
        );
    }
    if matches!(&p.ty, TypeRef::Primitive(alef_core::ir::PrimitiveType::Bool)) {
        return format!("{}.encode(env)", p.name);
    }
    format!("format!(\"{{:?}}\", {}).encode(env)", p.name)
}

/// Map TypeRef to a Rust type string.
fn param_type(ty: &TypeRef, ci: &str, is_ref: bool, tp: &std::collections::HashMap<&str, &str>) -> String {
    match ty {
        TypeRef::Bytes if is_ref => "&[u8]".into(),
        TypeRef::Bytes => "Vec<u8>".into(),
        TypeRef::String if is_ref => "&str".into(),
        TypeRef::String => "String".into(),
        TypeRef::Path if is_ref => "&std::path::Path".into(),
        TypeRef::Path => "std::path::PathBuf".into(),
        TypeRef::Named(n) => {
            let qualified = tp
                .get(n.as_str())
                .map(|p| p.replace('-', "_"))
                .unwrap_or_else(|| format!("{ci}::{n}"));
            if is_ref { format!("&{qualified}") } else { qualified }
        }
        TypeRef::Vec(inner) => format!("Vec<{}>", param_type(inner, ci, false, tp)),
        TypeRef::Optional(inner) => format!("Option<{}>", param_type(inner, ci, false, tp)),
        TypeRef::Primitive(p) => prim(p).into(),
        TypeRef::Unit => "()".into(),
        TypeRef::Char => "char".into(),
        TypeRef::Map(k, v) => format!(
            "std::collections::HashMap<{}, {}>",
            param_type(k, ci, false, tp),
            param_type(v, ci, false, tp)
        ),
        TypeRef::Json => "serde_json::Value".into(),
        TypeRef::Duration => "std::time::Duration".into(),
    }
}

fn prim(p: &alef_core::ir::PrimitiveType) -> &'static str {
    use alef_core::ir::PrimitiveType::*;
    match p {
        Bool => "bool",
        U8 => "u8",
        U16 => "u16",
        U32 => "u32",
        U64 => "u64",
        I8 => "i8",
        I16 => "i16",
        I32 => "i32",
        I64 => "i64",
        F32 => "f32",
        F64 => "f64",
        Usize => "usize",
        Isize => "isize",
    }
}

/// Find the first parameter index and bridge config where the parameter's named type
/// matches a trait bridge's `type_alias`.
///
/// Returns `None` when no bridge applies.
pub fn find_bridge_param<'a>(
    func: &alef_core::ir::FunctionDef,
    bridges: &'a [TraitBridgeConfig],
) -> Option<(usize, &'a TraitBridgeConfig)> {
    for (idx, param) in func.params.iter().enumerate() {
        let named = match &param.ty {
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
        for bridge in bridges {
            if let Some(type_name) = named {
                if bridge.type_alias.as_deref() == Some(type_name) {
                    return Some((idx, bridge));
                }
            }
            if bridge.param_name.as_deref() == Some(param.name.as_str()) {
                return Some((idx, bridge));
            }
        }
    }
    None
}

/// Generate a Rustler NIF function that has one parameter replaced by
/// `Option<rustler::Term<'_>>` (a trait bridge). The bridge is constructed before
/// calling the core function.
#[allow(clippy::too_many_arguments)]
pub fn gen_bridge_function(
    func: &alef_core::ir::FunctionDef,
    bridge_param_idx: usize,
    bridge_cfg: &TraitBridgeConfig,
    mapper: &dyn alef_codegen::type_mapper::TypeMapper,
    opaque_types: &ahash::AHashSet<String>,
    default_types: &ahash::AHashSet<String>,
    core_import: &str,
) -> String {
    use alef_core::ir::TypeRef;

    let struct_name = format!("Elixir{}Bridge", bridge_cfg.trait_name);
    let handle_path = format!("{core_import}::visitor::VisitorHandle");
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
    let bridge_wrap = if is_optional {
        format!(
            "let {param_name}: Option<{handle_path}> = match {param_name} {{\n        \
             Some(term) if !term.is_nil() => {{\n            \
             let bridge = {struct_name}::new(env, term);\n            \
             Some(std::rc::Rc::new(std::cell::RefCell::new(bridge)) as {handle_path})\n        \
             }},\n        \
             _ => None,\n    \
             }};"
        )
    } else {
        format!(
            "let {param_name} = {{\n        \
             let bridge = {struct_name}::new(env, {param_name});\n        \
             std::rc::Rc::new(std::cell::RefCell::new(bridge)) as {handle_path}\n    \
             }};"
        )
    };

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
                        return format!("{}.as_ref().map(|v| &v.inner)", p.name);
                    }
                    return format!("&{}.inner", p.name);
                }
                if default_types.contains(n) {
                    if p.is_ref {
                        return format!("{}_core.as_ref().unwrap_or(&Default::default())", p.name);
                    }
                    return format!("{}_core.unwrap_or_default()", p.name);
                }
            }
            match &p.ty {
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
            format!("rustler::ResourceArc::new({name}Inner {{ inner: std::sync::Arc::new(val) }})")
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
    let mut out = String::with_capacity(1024);
    writeln!(out, "#[rustler::nif]").ok();
    writeln!(out, "pub fn {func_name}({params_str}) -> {ret} {{").ok();
    writeln!(out, "    {body}").ok();
    writeln!(out, "}}").ok();

    out
}
