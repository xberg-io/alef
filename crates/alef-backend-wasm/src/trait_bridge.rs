//! WebAssembly (wasm-bindgen) specific trait bridge code generation.
//!
//! Generates Rust wrapper structs that implement Rust traits by delegating
//! to JavaScript objects via `js_sys::Reflect` and `js_sys::Function`.

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
    let struct_name = format!("Wasm{}Bridge", bridge_cfg.trait_name);
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

/// Generate a visitor-style bridge wrapping a `wasm_bindgen::JsValue` object.
///
/// Every trait method checks if the JS object has a matching camelCase property,
/// then calls it via `js_sys::Reflect` and maps the return value to `VisitResult`.
fn gen_visitor_bridge(
    out: &mut String,
    trait_type: &TypeDef,
    _bridge_cfg: &TraitBridgeConfig,
    struct_name: &str,
    trait_path: &str,
    core_crate: &str,
    type_paths: &std::collections::HashMap<&str, &str>,
) {
    // Helper: convert NodeContext to a JS object via js_sys::Object
    writeln!(out, "fn nodecontext_to_js_value(").unwrap();
    writeln!(out, "    ctx: &{core_crate}::visitor::NodeContext,").unwrap();
    writeln!(out, ") -> wasm_bindgen::JsValue {{").unwrap();
    writeln!(out, "    let obj = js_sys::Object::new();").unwrap();
    writeln!(
        out,
        "    js_sys::Reflect::set(&obj, &wasm_bindgen::JsValue::from_str(\"nodeType\"), &wasm_bindgen::JsValue::from_str(&format!(\"{{:?}}\", ctx.node_type))).ok();"
    )
    .unwrap();
    writeln!(
        out,
        "    js_sys::Reflect::set(&obj, &wasm_bindgen::JsValue::from_str(\"tagName\"), &wasm_bindgen::JsValue::from_str(&ctx.tag_name)).ok();"
    )
    .unwrap();
    writeln!(
        out,
        "    js_sys::Reflect::set(&obj, &wasm_bindgen::JsValue::from_str(\"depth\"), &wasm_bindgen::JsValue::from_f64(ctx.depth as f64)).ok();"
    )
    .unwrap();
    writeln!(
        out,
        "    js_sys::Reflect::set(&obj, &wasm_bindgen::JsValue::from_str(\"indexInParent\"), &wasm_bindgen::JsValue::from_f64(ctx.index_in_parent as f64)).ok();"
    )
    .unwrap();
    writeln!(
        out,
        "    js_sys::Reflect::set(&obj, &wasm_bindgen::JsValue::from_str(\"isInline\"), &wasm_bindgen::JsValue::from_bool(ctx.is_inline)).ok();"
    )
    .unwrap();
    writeln!(
        out,
        "    let parent_tag_val = match &ctx.parent_tag {{\n        Some(s) => wasm_bindgen::JsValue::from_str(s),\n        None => wasm_bindgen::JsValue::null(),\n    }};"
    )
    .unwrap();
    writeln!(
        out,
        "    js_sys::Reflect::set(&obj, &wasm_bindgen::JsValue::from_str(\"parentTag\"), &parent_tag_val).ok();"
    )
    .unwrap();
    writeln!(out, "    let attrs = js_sys::Object::new();").unwrap();
    writeln!(out, "    for (k, v) in &ctx.attributes {{").unwrap();
    writeln!(
        out,
        "        js_sys::Reflect::set(&attrs, &wasm_bindgen::JsValue::from_str(k), &wasm_bindgen::JsValue::from_str(v)).ok();"
    )
    .unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(
        out,
        "    js_sys::Reflect::set(&obj, &wasm_bindgen::JsValue::from_str(\"attributes\"), &attrs).ok();"
    )
    .unwrap();
    writeln!(out, "    obj.into()").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // Bridge struct
    writeln!(out, "pub struct {struct_name} {{").unwrap();
    writeln!(out, "    js_obj: wasm_bindgen::JsValue,").unwrap();
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
    writeln!(out, "    pub fn new(js_obj: wasm_bindgen::JsValue) -> Self {{").unwrap();
    writeln!(out, "        Self {{ js_obj }}").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // Trait impl
    writeln!(out, "impl {trait_path} for {struct_name} {{").unwrap();
    for method in &trait_type.methods {
        if method.trait_source.is_some() {
            continue;
        }
        gen_visitor_method_wasm(out, method, type_paths);
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

/// Generate a single visitor method that checks for a camelCase JS property and calls it.
fn gen_visitor_method_wasm(out: &mut String, method: &MethodDef, type_paths: &std::collections::HashMap<&str, &str>) {
    let name = &method.name;
    let js_name = to_camel_case(name);

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

    // Check if the JS object has the method via Reflect
    writeln!(out, "        let key = wasm_bindgen::JsValue::from_str(\"{js_name}\");").unwrap();
    writeln!(
        out,
        "        let has_method = js_sys::Reflect::has(&self.js_obj, &key).unwrap_or(false);"
    )
    .unwrap();
    writeln!(out, "        if !has_method {{").unwrap();
    writeln!(out, "            return {ret_ty}::Continue;").unwrap();
    writeln!(out, "        }}").unwrap();

    // Get the JS function
    writeln!(
        out,
        "        let func_val = match js_sys::Reflect::get(&self.js_obj, &key) {{"
    )
    .unwrap();
    writeln!(out, "            Ok(f) => f,").unwrap();
    writeln!(out, "            Err(_) => return {ret_ty}::Continue,").unwrap();
    writeln!(out, "        }};").unwrap();
    writeln!(out, "        let func: js_sys::Function = match func_val.dyn_into() {{").unwrap();
    writeln!(out, "            Ok(f) => f,").unwrap();
    writeln!(out, "            Err(_) => return {ret_ty}::Continue,").unwrap();
    writeln!(out, "        }};").unwrap();

    // Build args array
    writeln!(out, "        let args = js_sys::Array::new();").unwrap();
    for p in &method.params {
        let arg_val = build_wasm_arg(p);
        writeln!(out, "        args.push(&{arg_val});").unwrap();
    }

    // Call the function
    writeln!(out, "        let result = func.apply(&self.js_obj, &args);").unwrap();

    // Parse result
    writeln!(out, "        match result {{").unwrap();
    writeln!(out, "            Err(_) => {ret_ty}::Continue,").unwrap();
    writeln!(out, "            Ok(val) => {{").unwrap();
    writeln!(out, "                if let Some(s) = val.as_string() {{").unwrap();
    writeln!(out, "                    match s.to_lowercase().as_str() {{").unwrap();
    writeln!(out, "                        \"continue\" => {ret_ty}::Continue,").unwrap();
    writeln!(out, "                        \"skip\" => {ret_ty}::Skip,").unwrap();
    writeln!(
        out,
        "                        \"preserve_html\" | \"preservehtml\" => {ret_ty}::PreserveHtml,"
    )
    .unwrap();
    writeln!(
        out,
        "                        other => {ret_ty}::Custom(other.to_string()),"
    )
    .unwrap();
    writeln!(out, "                    }}").unwrap();
    writeln!(out, "                }} else {{").unwrap();
    writeln!(out, "                    {ret_ty}::Continue").unwrap();
    writeln!(out, "                }}").unwrap();
    writeln!(out, "            }}").unwrap();
    writeln!(out, "        }}").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out).unwrap();
}

/// Build a single wasm arg expression for a visitor method parameter.
fn build_wasm_arg(p: &alef_core::ir::ParamDef) -> String {
    if let TypeRef::Named(n) = &p.ty {
        if n == "NodeContext" {
            return format!("nodecontext_to_js_value({}{})", if p.is_ref { "" } else { "&" }, p.name);
        }
    }
    // Optional &str must be checked before non-optional &str — otherwise Option<&str>
    // would be passed to JsValue::from_str which expects &str, causing a type error.
    if p.optional && matches!(&p.ty, TypeRef::String) && p.is_ref {
        return format!(
            "match {} {{ Some(s) => wasm_bindgen::JsValue::from_str(s), None => wasm_bindgen::JsValue::null() }}",
            p.name
        );
    }
    if matches!(&p.ty, TypeRef::String) && p.is_ref {
        return format!("wasm_bindgen::JsValue::from_str({})", p.name);
    }
    if matches!(&p.ty, TypeRef::String) {
        return format!("wasm_bindgen::JsValue::from_str({}.as_str())", p.name);
    }
    if matches!(&p.ty, TypeRef::Primitive(alef_core::ir::PrimitiveType::Bool)) {
        return format!("wasm_bindgen::JsValue::from_bool({})", p.name);
    }
    format!("wasm_bindgen::JsValue::from_str(&format!(\"{{:?}}\", {}))", p.name)
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

/// Generate a WASM free function that has one parameter replaced by
/// `wasm_bindgen::JsValue` (a trait bridge).
#[allow(clippy::too_many_arguments)]
pub fn gen_bridge_function(
    func: &alef_core::ir::FunctionDef,
    bridge_param_idx: usize,
    bridge_cfg: &TraitBridgeConfig,
    mapper: &dyn alef_codegen::type_mapper::TypeMapper,
    opaque_types: &ahash::AHashSet<String>,
    core_import: &str,
    prefix: &str,
) -> String {
    use alef_core::ir::TypeRef;

    let struct_name = format!("Wasm{}Bridge", bridge_cfg.trait_name);
    let handle_path = format!("{core_import}::visitor::VisitorHandle");
    let param_name = &func.params[bridge_param_idx].name;
    let bridge_param = &func.params[bridge_param_idx];
    let is_optional = bridge_param.optional || matches!(&bridge_param.ty, TypeRef::Optional(_));

    // Build parameter list
    let mut sig_parts = Vec::new();
    for (idx, p) in func.params.iter().enumerate() {
        if idx == bridge_param_idx {
            if is_optional {
                sig_parts.push(format!("{}: Option<wasm_bindgen::JsValue>", p.name));
            } else {
                sig_parts.push(format!("{}: wasm_bindgen::JsValue", p.name));
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

    let err_conv = ".map_err(|e| wasm_bindgen::JsValue::from_str(&e.to_string()))";

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

    // From conversion let bindings for non-bridge Named params.
    // Uses the generated From<WasmType> impl to convert binding types to core types,
    // which avoids requiring serde::Serialize on WASM binding types (many contain JsValue
    // which cannot be serialized).
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
                format!("let {name}_core: Option<{core_path}> = {name}.map({core_path}::from);\n    ")
            } else {
                format!("let {name}_core: {core_path} = {core_path}::from({name});\n    ")
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
            format!("{prefix}{name} {{ inner: std::sync::Arc::new(val) }}")
        }
        TypeRef::Named(_) => "val.into()".to_string(),
        TypeRef::String | TypeRef::Bytes => "val.into()".to_string(),
        _ => "val".to_string(),
    };

    let js_name = to_camel_case(&func.name);
    let js_name_attr = if js_name != func.name {
        format!("(js_name = \"{}\")", js_name)
    } else {
        String::new()
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
    writeln!(out, "#[wasm_bindgen{js_name_attr}]").ok();
    writeln!(out, "pub fn {func_name}({params_str}) -> {ret} {{").ok();
    writeln!(out, "    {body}").ok();
    writeln!(out, "}}").ok();

    out
}
