//! Ruby (Magnus) specific trait bridge code generation.
//!
//! Generates Rust wrapper structs that implement Rust traits by delegating
//! to Ruby objects via Magnus `respond_to` checks and `funcall`.

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
    let struct_name = format!("Rb{}Bridge", bridge_cfg.trait_name);
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

/// Generate a visitor-style bridge wrapping a Magnus `magnus::Value`.
///
/// Every trait method checks if the Ruby object responds to a snake_case method,
/// then calls it via `funcall` and maps the return value to `VisitResult`.
fn gen_visitor_bridge(
    out: &mut String,
    trait_type: &TypeDef,
    _bridge_cfg: &TraitBridgeConfig,
    struct_name: &str,
    trait_path: &str,
    core_crate: &str,
    type_paths: &std::collections::HashMap<&str, &str>,
) {
    // Helper: convert NodeContext to a Ruby hash (magnus::RHash)
    writeln!(out, "fn nodecontext_to_rb_hash(").unwrap();
    writeln!(out, "    ctx: &{core_crate}::visitor::NodeContext,").unwrap();
    writeln!(out, ") -> magnus::RHash {{").unwrap();
    writeln!(out, "    let ruby = unsafe {{ magnus::Ruby::get_unchecked() }};").unwrap();
    writeln!(out, "    let h = ruby.hash_new();").unwrap();
    writeln!(
        out,
        "    h.aset(ruby.to_symbol(\"node_type\"), format!(\"{{:?}}\", ctx.node_type)).ok();"
    )
    .unwrap();
    writeln!(
        out,
        "    h.aset(ruby.to_symbol(\"tag_name\"), ctx.tag_name.as_str()).ok();"
    )
    .unwrap();
    writeln!(out, "    h.aset(ruby.to_symbol(\"depth\"), ctx.depth as i64).ok();").unwrap();
    writeln!(
        out,
        "    h.aset(ruby.to_symbol(\"index_in_parent\"), ctx.index_in_parent as i64).ok();"
    )
    .unwrap();
    writeln!(out, "    h.aset(ruby.to_symbol(\"is_inline\"), ctx.is_inline).ok();").unwrap();
    writeln!(
        out,
        "    h.aset(ruby.to_symbol(\"parent_tag\"), ctx.parent_tag.as_deref().map(|s| magnus::Value::from(ruby.str_new(s)))).ok();"
    )
    .unwrap();
    writeln!(out, "    let attrs = ruby.hash_new();").unwrap();
    writeln!(out, "    for (k, v) in &ctx.attributes {{").unwrap();
    writeln!(out, "        attrs.aset(ruby.str_new(k), ruby.str_new(v)).ok();").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "    h.aset(ruby.to_symbol(\"attributes\"), attrs).ok();").unwrap();
    writeln!(out, "    h").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // Bridge struct
    writeln!(out, "pub struct {struct_name} {{").unwrap();
    writeln!(out, "    rb_obj: magnus::Value,").unwrap();
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
    writeln!(out, "    pub fn new(rb_obj: magnus::Value) -> Self {{").unwrap();
    writeln!(out, "        Self {{ rb_obj }}").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // Trait impl
    writeln!(out, "impl {trait_path} for {struct_name} {{").unwrap();
    for method in &trait_type.methods {
        if method.trait_source.is_some() {
            continue;
        }
        gen_visitor_method_magnus(out, method, type_paths);
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

/// Generate a single visitor method that checks Ruby respond_to and calls via funcall.
fn gen_visitor_method_magnus(out: &mut String, method: &MethodDef, type_paths: &std::collections::HashMap<&str, &str>) {
    let name = &method.name;
    // Ruby uses snake_case method names (same as Rust)

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

    // Check if the Ruby object responds to this method
    writeln!(
        out,
        "        let responds = self.rb_obj.respond_to(\"{name}\", false).unwrap_or(false);"
    )
    .unwrap();
    writeln!(out, "        if !responds {{").unwrap();
    writeln!(out, "            return {ret_ty}::Continue;").unwrap();
    writeln!(out, "        }}").unwrap();

    // Build the funcall args tuple
    if method.params.is_empty() {
        writeln!(
            out,
            "        let result: Result<magnus::Value, magnus::Error> = magnus::method::Method::funcall(self.rb_obj, \"{name}\", ());"
        )
        .unwrap();
    } else {
        // Build args as a tuple
        let args_exprs: Vec<String> = method.params.iter().map(build_magnus_arg).collect();
        let args_tuple = if args_exprs.len() == 1 {
            format!("({},)", args_exprs[0])
        } else {
            format!("({})", args_exprs.join(", "))
        };
        writeln!(
            out,
            "        let result: Result<magnus::Value, magnus::Error> = magnus::method::Method::funcall(self.rb_obj, \"{name}\", {args_tuple});"
        )
        .unwrap();
    }

    // Parse result
    writeln!(out, "        match result {{").unwrap();
    writeln!(out, "            Err(_) => {ret_ty}::Continue,").unwrap();
    writeln!(out, "            Ok(val) => {{").unwrap();
    writeln!(out, "                let s: String = val.to_string();").unwrap();
    writeln!(out, "                match s.to_lowercase().as_str() {{").unwrap();
    writeln!(out, "                    \"continue\" => {ret_ty}::Continue,").unwrap();
    writeln!(out, "                    \"skip\" => {ret_ty}::Skip,").unwrap();
    writeln!(
        out,
        "                    \"preserve_html\" | \"preservehtml\" => {ret_ty}::PreserveHtml,"
    )
    .unwrap();
    writeln!(out, "                    other => {ret_ty}::Custom(other.to_string()),").unwrap();
    writeln!(out, "                }}").unwrap();
    writeln!(out, "            }}").unwrap();
    writeln!(out, "        }}").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out).unwrap();
}

/// Build a single Magnus funcall arg expression for a visitor method parameter.
fn build_magnus_arg(p: &alef_core::ir::ParamDef) -> String {
    if let TypeRef::Named(n) = &p.ty {
        if n == "NodeContext" {
            return format!("nodecontext_to_rb_hash({}{})", if p.is_ref { "" } else { "&" }, p.name);
        }
    }
    if matches!(&p.ty, TypeRef::String) && p.is_ref {
        return format!("magnus::RString::new({})", p.name);
    }
    if matches!(&p.ty, TypeRef::String) {
        return format!("magnus::RString::new({}.as_str())", p.name);
    }
    if p.optional && matches!(&p.ty, TypeRef::String) && p.is_ref {
        return format!(
            "match {} {{ Some(s) => magnus::Value::from(magnus::RString::new(s)), None => magnus::Value::nil() }}",
            p.name
        );
    }
    if matches!(&p.ty, TypeRef::Primitive(alef_core::ir::PrimitiveType::Bool)) {
        return format!("magnus::Value::from({})", p.name);
    }
    format!("magnus::Value::from({})", p.name)
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

/// Generate a Magnus free function that has one parameter replaced by `magnus::Value` (a trait
/// bridge). The bridge is constructed before calling the core function.
#[allow(clippy::too_many_arguments)]
pub fn gen_bridge_function(
    func: &alef_core::ir::FunctionDef,
    bridge_param_idx: usize,
    bridge_cfg: &TraitBridgeConfig,
    mapper: &dyn alef_codegen::type_mapper::TypeMapper,
    opaque_types: &ahash::AHashSet<String>,
    core_import: &str,
) -> String {
    use alef_core::ir::TypeRef;

    let struct_name = format!("Rb{}Bridge", bridge_cfg.trait_name);
    let handle_path = format!("{core_import}::visitor::VisitorHandle");
    let param_name = &func.params[bridge_param_idx].name;
    let bridge_param = &func.params[bridge_param_idx];
    let is_optional = bridge_param.optional || matches!(&bridge_param.ty, TypeRef::Optional(_));

    // Build parameter list
    let mut sig_parts = Vec::new();
    for (idx, p) in func.params.iter().enumerate() {
        if idx == bridge_param_idx {
            if is_optional {
                sig_parts.push(format!("{}: Option<magnus::Value>", p.name));
            } else {
                sig_parts.push(format!("{}: magnus::Value", p.name));
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
    // Magnus functions with errors always return Result
    let has_error = func.error_type.is_some();
    let ret = mapper.wrap_return(&return_type, has_error);

    let err_conv = ".map_err(|e| magnus::Error::new(unsafe { magnus::Ruby::get_unchecked() }.exception_runtime_error(), e.to_string()))";

    // Bridge wrapping code
    let bridge_wrap = if is_optional {
        format!(
            "let {param_name}: Option<{handle_path}> = match {param_name} {{\n        \
             Some(v) if !v.is_nil() => {{\n            \
             let bridge = {struct_name}::new(v);\n            \
             Some(std::rc::Rc::new(std::cell::RefCell::new(bridge)) as {handle_path})\n        \
             }},\n        \
             _ => None,\n    \
             }};"
        )
    } else {
        format!(
            "let {param_name} = {{\n        \
             let bridge = {struct_name}::new({param_name});\n        \
             std::rc::Rc::new(std::cell::RefCell::new(bridge)) as {handle_path}\n    \
             }};"
        )
    };

    // Serde let bindings for non-bridge Named params
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
                    "let {name}_core: Option<{core_path}> = {name}.as_deref().filter(|s| *s != \"nil\").map(|s| serde_json::from_str(s){err_conv}).transpose()?;\n    "
                )
            } else {
                format!(
                    "let {name}_core: {core_path} = serde_json::from_str(&{name}){err_conv}?;\n    "
                )
            }
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
        if return_wrap == "val" {
            format!("{bridge_wrap}\n    {serde_bindings}{core_call}{err_conv}")
        } else {
            format!("{bridge_wrap}\n    {serde_bindings}{core_call}.map(|val| {return_wrap}){err_conv}")
        }
    } else {
        format!("{bridge_wrap}\n    {serde_bindings}{core_call}")
    };

    let func_name = &func.name;
    let mut out = String::with_capacity(1024);
    if func.error_type.is_some() {
        writeln!(out, "#[allow(clippy::missing_errors_doc)]").ok();
    }
    writeln!(out, "pub fn {func_name}({params_str}) -> {ret} {{").ok();
    writeln!(out, "    {body}").ok();
    writeln!(out, "}}").ok();

    out
}
