//! PHP (ext-php-rs) specific trait bridge code generation.
//!
//! Generates Rust wrapper structs that implement Rust traits by delegating
//! to PHP objects via ext-php-rs Zval method calls.

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
    let struct_name = format!("Php{}Bridge", bridge_cfg.trait_name);
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

/// Generate a visitor-style bridge wrapping a PHP `Zval` object reference.
///
/// Every trait method checks if the PHP object has a matching camelCase method,
/// then calls it and maps the PHP return value to `VisitResult`.
fn gen_visitor_bridge(
    out: &mut String,
    trait_type: &TypeDef,
    _bridge_cfg: &TraitBridgeConfig,
    struct_name: &str,
    trait_path: &str,
    core_crate: &str,
    type_paths: &std::collections::HashMap<&str, &str>,
) {
    // Helper: convert NodeContext to a PHP array (Zval)
    writeln!(out, "fn nodecontext_to_php_array(").unwrap();
    writeln!(out, "    ctx: &{core_crate}::visitor::NodeContext,").unwrap();
    writeln!(out, ") -> ext_php_rs::boxed::ZBox<ext_php_rs::types::ZendHashTable> {{").unwrap();
    writeln!(out, "    let mut arr = ext_php_rs::types::ZendHashTable::new();").unwrap();
    writeln!(
        out,
        "    arr.insert(\"nodeType\", ext_php_rs::types::Zval::try_from(format!(\"{{:?}}\", ctx.node_type)).unwrap_or_default()).ok();"
    )
    .unwrap();
    writeln!(
        out,
        "    arr.insert(\"tagName\", ext_php_rs::types::Zval::try_from(ctx.tag_name.clone()).unwrap_or_default()).ok();"
    )
    .unwrap();
    writeln!(
        out,
        "    arr.insert(\"depth\", ext_php_rs::types::Zval::try_from(ctx.depth as i64).unwrap_or_default()).ok();"
    )
    .unwrap();
    writeln!(
        out,
        "    arr.insert(\"indexInParent\", ext_php_rs::types::Zval::try_from(ctx.index_in_parent as i64).unwrap_or_default()).ok();"
    )
    .unwrap();
    writeln!(
        out,
        "    arr.insert(\"isInline\", ext_php_rs::types::Zval::try_from(ctx.is_inline).unwrap_or_default()).ok();"
    )
    .unwrap();
    writeln!(out, "    if let Some(ref pt) = ctx.parent_tag {{").unwrap();
    writeln!(
        out,
        "        arr.insert(\"parentTag\", ext_php_rs::types::Zval::try_from(pt.clone()).unwrap_or_default()).ok();"
    )
    .unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "    let mut attrs = ext_php_rs::types::ZendHashTable::new();").unwrap();
    writeln!(out, "    for (k, v) in &ctx.attributes {{").unwrap();
    writeln!(
        out,
        "        attrs.insert(k.as_str(), ext_php_rs::types::Zval::try_from(v.clone()).unwrap_or_default()).ok();"
    )
    .unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "    let mut attrs_zval = ext_php_rs::types::Zval::new();").unwrap();
    writeln!(out, "    attrs_zval.set_array(attrs);").unwrap();
    writeln!(out, "    arr.insert(\"attributes\", attrs_zval).ok();").unwrap();
    writeln!(out, "    arr").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // Bridge struct
    writeln!(out, "pub struct {struct_name} {{").unwrap();
    writeln!(
        out,
        "    php_obj: ext_php_rs::boxed::ZBox<ext_php_rs::types::ZendObject>,"
    )
    .unwrap();
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
        "    pub fn new(php_obj: ext_php_rs::boxed::ZBox<ext_php_rs::types::ZendObject>) -> Self {{"
    )
    .unwrap();
    writeln!(out, "        Self {{ php_obj }}").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // Trait impl
    writeln!(out, "impl {trait_path} for {struct_name} {{").unwrap();
    for method in &trait_type.methods {
        if method.trait_source.is_some() {
            continue;
        }
        gen_visitor_method_php(out, method, type_paths);
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

/// Generate a single visitor method that checks for a camelCase PHP method and calls it.
fn gen_visitor_method_php(out: &mut String, method: &MethodDef, type_paths: &std::collections::HashMap<&str, &str>) {
    let name = &method.name;
    let php_name = to_camel_case(name);

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

    // Check if the PHP object has the method
    writeln!(
        out,
        "        let has_method = self.php_obj.has_method(\"{php_name}\").unwrap_or(false);"
    )
    .unwrap();
    writeln!(out, "        if !has_method {{").unwrap();
    writeln!(out, "            return {ret_ty}::Continue;").unwrap();
    writeln!(out, "        }}").unwrap();

    // Build args array
    let has_args = !method.params.is_empty();
    if has_args {
        writeln!(out, "        let mut args: Vec<ext_php_rs::types::Zval> = Vec::new();").unwrap();
        for p in &method.params {
            if let TypeRef::Named(n) = &p.ty {
                if n == "NodeContext" {
                    writeln!(
                        out,
                        "        let ctx_arr = nodecontext_to_php_array({}{});",
                        if p.is_ref { "" } else { "&" },
                        p.name
                    )
                    .unwrap();
                    writeln!(
                        out,
                        "        args.push(ext_php_rs::types::Zval::try_from(ctx_arr).unwrap_or_default());"
                    )
                    .unwrap();
                    continue;
                }
            }
            if matches!(&p.ty, TypeRef::String) {
                if p.is_ref {
                    writeln!(
                        out,
                        "        args.push(ext_php_rs::types::Zval::try_from({}.to_string()).unwrap_or_default());",
                        p.name
                    )
                    .unwrap();
                } else {
                    writeln!(
                        out,
                        "        args.push(ext_php_rs::types::Zval::try_from({}.clone()).unwrap_or_default());",
                        p.name
                    )
                    .unwrap();
                }
                continue;
            }
            if p.optional && matches!(&p.ty, TypeRef::String) && p.is_ref {
                writeln!(
                    out,
                    "        args.push(match {0} {{ Some(s) => ext_php_rs::types::Zval::try_from(s.to_string()).unwrap_or_default(), None => ext_php_rs::types::Zval::default() }});",
                    p.name
                )
                .unwrap();
                continue;
            }
            if matches!(&p.ty, TypeRef::Primitive(alef_core::ir::PrimitiveType::Bool)) {
                writeln!(
                    out,
                    "        args.push(ext_php_rs::types::Zval::try_from({}).unwrap_or_default());",
                    p.name
                )
                .unwrap();
                continue;
            }
            // Default: format as string
            writeln!(
                out,
                "        args.push(ext_php_rs::types::Zval::try_from(format!(\"{{:?}}\", {})).unwrap_or_default());",
                p.name
            )
            .unwrap();
        }
    }

    // Call the PHP method
    let args_expr = if has_args {
        "args.iter_mut().collect::<Vec<_>>().as_mut_slice()"
    } else {
        "&mut []"
    };
    writeln!(
        out,
        "        let result = self.php_obj.call_method(\"{php_name}\", {args_expr});"
    )
    .unwrap();

    // Parse result
    writeln!(out, "        match result {{").unwrap();
    writeln!(out, "            Err(_) | Ok(None) => {ret_ty}::Continue,").unwrap();
    writeln!(out, "            Ok(Some(val)) => {{").unwrap();
    writeln!(
        out,
        "                let s = val.string().unwrap_or_default().to_lowercase();"
    )
    .unwrap();
    writeln!(out, "                match s.as_str() {{").unwrap();
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

/// Convert snake_case to camelCase.
fn to_camel_case(name: &str) -> String {
    let mut result = String::with_capacity(name.len());
    let mut capitalize_next = false;
    for (i, c) in name.chars().enumerate() {
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

/// Generate a PHP static method that has one parameter replaced by
/// `Option<ext_php_rs::boxed::ZBox<ext_php_rs::types::ZendObject>>` (a trait bridge).
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

    let struct_name = format!("Php{}Bridge", bridge_cfg.trait_name);
    let handle_path = format!("{core_import}::visitor::VisitorHandle");
    let param_name = &func.params[bridge_param_idx].name;
    let bridge_param = &func.params[bridge_param_idx];
    let is_optional = bridge_param.optional || matches!(&bridge_param.ty, TypeRef::Optional(_));

    // Build parameter list, hiding bridge params from signature
    let mut sig_parts = Vec::new();
    for (idx, p) in func.params.iter().enumerate() {
        if idx == bridge_param_idx {
            // Bridge param is visible in PHP as an optional object
            let php_obj_ty = "ext_php_rs::boxed::ZBox<ext_php_rs::types::ZendObject>";
            if is_optional {
                sig_parts.push(format!("{}: Option<{php_obj_ty}>", p.name));
            } else {
                sig_parts.push(format!("{}: {php_obj_ty}", p.name));
            }
        } else {
            let promoted = idx > bridge_param_idx || func.params[..idx].iter().any(|pp| pp.optional);
            let base = mapper.map_type(&p.ty);
            // #[php_class] types (non-opaque Named) only implement FromZvalMut for &mut T,
            // not for owned T — so we must use &mut T in the function signature.
            let ty = match &p.ty {
                TypeRef::Named(n) if !opaque_types.contains(n.as_str()) => {
                    if p.optional || promoted {
                        format!("Option<&mut {base}>")
                    } else {
                        format!("&mut {base}")
                    }
                }
                TypeRef::Optional(inner) => {
                    if let TypeRef::Named(n) = inner.as_ref() {
                        if !opaque_types.contains(n.as_str()) {
                            format!("Option<&mut {base}>")
                        } else if p.optional || promoted {
                            format!("Option<{base}>")
                        } else {
                            base
                        }
                    } else if p.optional || promoted {
                        format!("Option<{base}>")
                    } else {
                        base
                    }
                }
                _ => {
                    if p.optional || promoted {
                        format!("Option<{base}>")
                    } else {
                        base
                    }
                }
            };
            sig_parts.push(format!("{}: {}", p.name, ty));
        }
    }

    let params_str = sig_parts.join(", ");
    let return_type = mapper.map_type(&func.return_type);
    let ret = mapper.wrap_return(&return_type, func.error_type.is_some());

    let err_conv = ".map_err(|e| ext_php_rs::exception::PhpException::default(e.to_string()))";

    // Bridge wrapping code
    let bridge_wrap = if is_optional {
        format!(
            "let {param_name} = {param_name}.map(|v| {{\n        \
             let bridge = {struct_name}::new(v);\n        \
             std::rc::Rc::new(std::cell::RefCell::new(bridge)) as {handle_path}\n    \
             }});"
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
                    "let {name}_core: Option<{core_path}> = {name}.map(|v| {{\n        \
                     let json = serde_json::to_string(&v){err_conv}?;\n        \
                     serde_json::from_str(&json){err_conv}\n    \
                     }}).transpose()?;\n    "
                )
            } else {
                format!(
                    "let {name}_json = serde_json::to_string(&{name}){err_conv}?;\n    \
                     let {name}_core: {core_path} = serde_json::from_str(&{name}_json){err_conv}?;\n    "
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
