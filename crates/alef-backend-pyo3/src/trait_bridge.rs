//! PyO3-specific trait bridge code generation.
//!
//! Generates Rust wrapper structs that implement Rust traits by delegating
//! to Python objects via PyO3.

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
    let struct_name = format!("Py{}Bridge", bridge_cfg.trait_name);
    let trait_path = trait_type.rust_path.replace('-', "_");

    // Build type name → rust_path lookup for qualifying Named types in signatures
    let type_paths: std::collections::HashMap<&str, &str> = api
        .types
        .iter()
        .map(|t| (t.name.as_str(), t.rust_path.as_str()))
        .chain(api.enums.iter().map(|e| (e.name.as_str(), e.rust_path.as_str())))
        .collect();

    // Determine bridge pattern: visitor-style (all methods have defaults, no registry) vs
    // plugin-style (cached fields, registry, super-trait).
    let is_visitor_bridge = bridge_cfg.type_alias.is_some()
        && bridge_cfg.register_fn.is_none()
        && bridge_cfg.super_trait.is_none()
        && trait_type.methods.iter().all(|m| m.has_default_impl);

    if is_visitor_bridge {
        gen_visitor_bridge(&mut out, trait_type, bridge_cfg, &struct_name, &trait_path, &type_paths);
    } else {
        gen_plugin_bridge(
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

/// Generate a visitor-style bridge: thin wrapper over `Py<PyAny>` where every trait method
/// tries to call the corresponding Python method, falling back to the default if absent.
///
/// This pattern is used for traits where:
/// - All methods have default implementations (e.g., `HtmlVisitor`)
/// - No registration function is needed (per-call construction via `type_alias`)
/// - No super-trait forwarding
fn gen_visitor_bridge(
    out: &mut String,
    trait_type: &TypeDef,
    bridge_cfg: &TraitBridgeConfig,
    struct_name: &str,
    trait_path: &str,
    type_paths: &std::collections::HashMap<&str, &str>,
) {
    let core_crate = trait_path
        .split("::")
        .next()
        .unwrap_or("html_to_markdown_rs")
        .to_string();

    // Emit a helper function for converting NodeContext to a Python dict.
    // This is emitted once per visitor bridge (always alongside it).
    writeln!(out, "fn nodecontext_to_py_dict<'py>(").unwrap();
    writeln!(out, "    py: Python<'py>,").unwrap();
    writeln!(out, "    ctx: &{core_crate}::visitor::NodeContext,").unwrap();
    writeln!(out, ") -> pyo3::Bound<'py, pyo3::types::PyDict> {{").unwrap();
    writeln!(out, "    let d = pyo3::types::PyDict::new(py);").unwrap();
    writeln!(out, "    d.set_item(\"node_type\", format!(\"{{:?}}\", ctx.node_type)).unwrap_or(());").unwrap();
    writeln!(out, "    d.set_item(\"tag_name\", &ctx.tag_name).unwrap_or(());").unwrap();
    writeln!(out, "    d.set_item(\"depth\", ctx.depth).unwrap_or(());").unwrap();
    writeln!(out, "    d.set_item(\"index_in_parent\", ctx.index_in_parent).unwrap_or(());").unwrap();
    writeln!(out, "    d.set_item(\"is_inline\", ctx.is_inline).unwrap_or(());").unwrap();
    writeln!(out, "    d.set_item(\"parent_tag\", ctx.parent_tag.as_deref()).unwrap_or(());").unwrap();
    writeln!(out, "    let attrs = pyo3::types::PyDict::new(py);").unwrap();
    writeln!(out, "    for (k, v) in &ctx.attributes {{").unwrap();
    writeln!(out, "        attrs.set_item(k, v).unwrap_or(());").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "    d.set_item(\"attributes\", attrs).unwrap_or(());").unwrap();
    writeln!(out, "    d").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // Struct with only the Python object — no cached fields needed
    writeln!(out, "#[derive(Debug)]").unwrap();
    writeln!(out, "pub struct {struct_name} {{").unwrap();
    writeln!(out, "    python_obj: Py<PyAny>,").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // Constructor
    writeln!(out, "impl {struct_name} {{").unwrap();
    writeln!(out, "    pub fn new(python_obj: Py<PyAny>) -> Self {{").unwrap();
    writeln!(out, "        Self {{ python_obj }}").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // Trait impl — all methods try Python dispatch, fall back to Continue
    let _ = &bridge_cfg.trait_name;
    writeln!(out, "impl {trait_path} for {struct_name} {{").unwrap();
    for method in &trait_type.methods {
        if method.trait_source.is_some() {
            continue;
        }
        gen_visitor_method(out, method, trait_path, type_paths);
    }
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}

/// Generate a plugin-style bridge: cached fields, optional super-trait, optional registration fn.
fn gen_plugin_bridge(
    out: &mut String,
    trait_type: &TypeDef,
    bridge_cfg: &TraitBridgeConfig,
    struct_name: &str,
    trait_path: &str,
    core_import: &str,
    type_paths: &std::collections::HashMap<&str, &str>,
) {
    // Wrapper struct with cached fields
    writeln!(out, "pub struct {struct_name} {{").unwrap();
    writeln!(out, "    python_obj: Py<PyAny>,").unwrap();
    writeln!(out, "    name: String,").unwrap();
    writeln!(out, "    supported_languages: Vec<String>,").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // Plugin super-trait impl
    if bridge_cfg.super_trait.is_some() {
        gen_plugin_impl(out, struct_name, core_import);
    }

    // Main trait impl
    writeln!(out, "#[async_trait]").unwrap();
    writeln!(out, "impl {trait_path} for {struct_name} {{").unwrap();
    for method in &trait_type.methods {
        if method.trait_source.is_some() {
            continue;
        }
        gen_method(out, method, core_import, type_paths);
    }
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // Registration function — only generated when register_fn is configured
    if bridge_cfg.register_fn.is_some() {
        gen_registration_fn(out, bridge_cfg, struct_name, trait_path);
    }
}

fn gen_plugin_impl(out: &mut String, struct_name: &str, ci: &str) {
    writeln!(out, "impl {ci}::plugins::Plugin for {struct_name} {{").unwrap();
    writeln!(out, "    fn name(&self) -> &str {{ &self.name }}").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "    fn version(&self) -> String {{").unwrap();
    writeln!(out, "        Python::attach(|py| {{").unwrap();
    writeln!(out, "            self.python_obj.bind(py)").unwrap();
    writeln!(out, "                .call_method0(\"version\")").unwrap();
    writeln!(out, "                .and_then(|v| v.extract::<String>())").unwrap();
    writeln!(out, "                .unwrap_or_else(|_| \"1.0.0\".to_string())").unwrap();
    writeln!(out, "        }})").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "    fn initialize(&self) -> {ci}::Result<()> {{").unwrap();
    writeln!(out, "        Python::attach(|py| {{").unwrap();
    writeln!(out, "            let obj = self.python_obj.bind(py);").unwrap();
    writeln!(out, "            if obj.hasattr(\"initialize\").unwrap_or(false) {{").unwrap();
    writeln!(
        out,
        "                obj.call_method0(\"initialize\").map_err(|e| {ci}::KreuzbergError::Plugin {{"
    )
    .unwrap();
    writeln!(
        out,
        "                    message: format!(\"Plugin '{{}}' initialize failed: {{}}\", self.name, e),"
    )
    .unwrap();
    writeln!(out, "                    plugin_name: self.name.clone(),").unwrap();
    writeln!(out, "                }})?;").unwrap();
    writeln!(out, "            }}").unwrap();
    writeln!(out, "            Ok(())").unwrap();
    writeln!(out, "        }})").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "    fn shutdown(&self) -> {ci}::Result<()> {{").unwrap();
    writeln!(out, "        Python::attach(|py| {{").unwrap();
    writeln!(out, "            let obj = self.python_obj.bind(py);").unwrap();
    writeln!(out, "            if obj.hasattr(\"shutdown\").unwrap_or(false) {{").unwrap();
    writeln!(
        out,
        "                obj.call_method0(\"shutdown\").map_err(|e| {ci}::KreuzbergError::Plugin {{"
    )
    .unwrap();
    writeln!(
        out,
        "                    message: format!(\"Plugin '{{}}' shutdown failed: {{}}\", self.name, e),"
    )
    .unwrap();
    writeln!(out, "                    plugin_name: self.name.clone(),").unwrap();
    writeln!(out, "                }})?;").unwrap();
    writeln!(out, "            }}").unwrap();
    writeln!(out, "            Ok(())").unwrap();
    writeln!(out, "        }})").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}

/// Map a visitor method parameter type to the correct Rust type string, handling IR quirks:
/// - `ty=String, optional=true, is_ref=true` → `Option<&str>` (the IR collapses `Option<&str>`)
/// - `ty=Vec<T>, is_ref=true` → `&[T]` (the IR collapses `&[T]`)
/// - Everything else uses the standard `param_type` helper.
fn visitor_param_type(
    ty: &TypeRef,
    is_ref: bool,
    optional: bool,
    tp: &std::collections::HashMap<&str, &str>,
) -> String {
    // `Option<&str>` case: IR collapses it to String + optional + is_ref
    if optional && matches!(ty, TypeRef::String) && is_ref {
        return "Option<&str>".to_string();
    }
    // `&[String]` case: IR collapses it to Vec<String> + is_ref
    if is_ref {
        if let TypeRef::Vec(inner) = ty {
            let inner_str = param_type(inner, "", false, tp);
            return format!("&[{inner_str}]");
        }
    }
    param_type(ty, "", is_ref, tp)
}

/// Generate a single visitor-style trait method that tries Python dispatch, falls back to default.
///
/// For each method the generated code:
/// 1. Checks if the Python object has an attribute with this method's name.
/// 2. If yes, calls the method with converted arguments and converts the Python return value
///    to the appropriate Rust return type.
/// 3. If no (attribute absent), returns the trait default (typically `VisitResult::Continue`).
fn gen_visitor_method(
    out: &mut String,
    method: &MethodDef,
    _trait_path: &str,
    type_paths: &std::collections::HashMap<&str, &str>,
) {
    use alef_core::ir::TypeRef;

    let name = &method.name;

    // Build the &mut self signature using the same helper used for plugin methods.
    // For visitor methods the IR may encode `Option<&str>` as `ty=String, optional=true, is_ref=true`
    // and `&[String]` as `ty=Vec<String>, is_ref=true`.
    let mut sig_parts = vec!["&mut self".to_string()];
    for p in &method.params {
        let ty_str = visitor_param_type(&p.ty, p.is_ref, p.optional, type_paths);
        sig_parts.push(format!("{}: {}", p.name, ty_str));
    }
    let sig = sig_parts.join(", ");

    // Determine the return type for this visitor method.
    // All HtmlVisitor methods return VisitResult (a Named type from the core crate).
    // Use the fully-qualified path from type_paths when available.
    let ret_ty = match &method.return_type {
        TypeRef::Named(n) => type_paths
            .get(n.as_str())
            .map(|p| p.replace('-', "_"))
            .unwrap_or_else(|| n.clone()),
        other => param_type(other, "", false, type_paths),
    };

    writeln!(out, "    fn {name}({sig}) -> {ret_ty} {{").unwrap();
    writeln!(out, "        Python::attach(|py| {{").unwrap();
    writeln!(out, "            let obj = self.python_obj.bind(py);").unwrap();
    writeln!(out, "            if !obj.hasattr(\"{name}\").unwrap_or(false) {{").unwrap();
    writeln!(out, "                return {ret_ty}::Continue;").unwrap();
    writeln!(out, "            }}").unwrap();

    // Build argument expressions for the Python call
    let py_args = build_visitor_py_args(method);

    let call = if py_args.is_empty() {
        format!("obj.call_method0(\"{name}\")")
    } else {
        format!("obj.call_method1(\"{name}\", ({py_args}))")
    };

    // Call the Python method and convert the result
    writeln!(out, "            match {call} {{").unwrap();
    writeln!(out, "                Err(_) => {ret_ty}::Continue,").unwrap();
    writeln!(out, "                Ok(result) => {{").unwrap();
    // Try to extract as string first
    writeln!(out, "                    if let Ok(s) = result.extract::<String>() {{").unwrap();
    writeln!(out, "                        match s.to_lowercase().as_str() {{").unwrap();
    writeln!(out, "                            \"continue\" => {ret_ty}::Continue,").unwrap();
    writeln!(out, "                            \"skip\" => {ret_ty}::Skip,").unwrap();
    writeln!(
        out,
        "                            \"preserve_html\" | \"preservehtml\" => {ret_ty}::PreserveHtml,"
    )
    .unwrap();
    writeln!(out, "                            other => {ret_ty}::Custom(other.to_string()),").unwrap();
    writeln!(out, "                        }}").unwrap();
    writeln!(out, "                    }} else if result.is_none() {{").unwrap();
    writeln!(out, "                        {ret_ty}::Continue").unwrap();
    writeln!(out, "                    }} else {{").unwrap();
    // Try dict protocol: {"custom": "..."} or {"error": "..."}
    writeln!(out, "                        let py_dict = result.downcast::<pyo3::types::PyDict>();").unwrap();
    writeln!(out, "                        if let Ok(d) = py_dict {{").unwrap();
    writeln!(
        out,
        "                            if let Some(v) = d.get_item(\"custom\").ok().flatten() {{"
    )
    .unwrap();
    writeln!(
        out,
        "                                {ret_ty}::Custom(v.extract::<String>().unwrap_or_default())"
    )
    .unwrap();
    writeln!(
        out,
        "                            }} else if let Some(v) = d.get_item(\"error\").ok().flatten() {{"
    )
    .unwrap();
    writeln!(
        out,
        "                                {ret_ty}::Error(v.extract::<String>().unwrap_or_default())"
    )
    .unwrap();
    writeln!(out, "                            }} else {{").unwrap();
    writeln!(out, "                                {ret_ty}::Continue").unwrap();
    writeln!(out, "                            }}").unwrap();
    writeln!(out, "                        }} else {{").unwrap();
    writeln!(out, "                            {ret_ty}::Continue").unwrap();
    writeln!(out, "                        }}").unwrap();
    writeln!(out, "                    }}").unwrap();
    writeln!(out, "                }}").unwrap();
    writeln!(out, "            }}").unwrap();
    writeln!(out, "        }})").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out).unwrap();
}

/// Build Python call argument expressions for a visitor method.
///
/// - `NodeContext` params: converted to a Python dict via `nodecontext_to_py_dict`
/// - `&str` params: passed directly (PyO3 handles `&str` → Python str coercion)
/// - `Option<&str>` params: passed as `Option<&str>` (PyO3 maps `None` → Python `None`)
/// - `bool` and integer params: passed directly
/// - `&[String]` / `Vec<String>` params: passed as Python lists
fn build_visitor_py_args(method: &MethodDef) -> String {
    use alef_core::ir::TypeRef;
    let args: Vec<String> = method
        .params
        .iter()
        .map(|p| {
            // NodeContext: convert to Python dict
            if let TypeRef::Named(n) = &p.ty {
                if n == "NodeContext" {
                    return if p.is_ref {
                        format!("nodecontext_to_py_dict(py, {})", p.name)
                    } else {
                        format!("nodecontext_to_py_dict(py, &{})", p.name)
                    };
                }
            }
            // `Option<&str>`: IR collapses to String + optional + is_ref — pass directly
            if p.optional && matches!(&p.ty, TypeRef::String) && p.is_ref {
                return p.name.clone();
            }
            // `&[String]`: IR collapses to Vec<String> + is_ref — pass directly (slice → PyList)
            if p.is_ref {
                if let TypeRef::Vec(inner) = &p.ty {
                    if matches!(inner.as_ref(), TypeRef::String) {
                        return p.name.clone();
                    }
                }
            }
            // Owned Vec<String>: convert to list
            if let TypeRef::Vec(inner) = &p.ty {
                if matches!(inner.as_ref(), TypeRef::String) {
                    return format!("{}.to_vec()", p.name);
                }
            }
            // Option<&str> encoded as Optional<String>
            if let TypeRef::Optional(inner) = &p.ty {
                if matches!(inner.as_ref(), TypeRef::String) {
                    return p.name.clone();
                }
            }
            // &str: pass directly
            if matches!(&p.ty, TypeRef::String) && p.is_ref {
                return p.name.clone();
            }
            if matches!(&p.ty, TypeRef::String) {
                return format!("{}.as_str()", p.name);
            }
            // Primitives and everything else: pass directly
            p.name.clone()
        })
        .collect();
    if args.len() == 1 {
        format!("{},", args[0])
    } else {
        args.join(", ")
    }
}

fn gen_method(out: &mut String, method: &MethodDef, ci: &str, tp: &std::collections::HashMap<&str, &str>) {
    let name = &method.name;
    let sig = build_signature(method, ci, tp);
    let ret = build_return(method, ci, tp);

    if method.is_async {
        writeln!(out, "    async fn {name}({sig}) -> {ret} {{").unwrap();
        gen_async_body(out, method, ci);
    } else {
        writeln!(out, "    fn {name}({sig}) -> {ret} {{").unwrap();
        gen_sync_body(out, method, ci);
    }
    writeln!(out, "    }}").unwrap();
    writeln!(out).unwrap();
}

/// Build correct trait method signature with proper reference types.
fn build_signature(method: &MethodDef, ci: &str, tp: &std::collections::HashMap<&str, &str>) -> String {
    let mut parts = Vec::new();
    if method.receiver.is_some() {
        parts.push("&self".to_string());
    }
    for p in &method.params {
        parts.push(format!("{}: {}", p.name, param_type(&p.ty, ci, p.is_ref, tp)));
    }
    parts.join(", ")
}

/// Map TypeRef to correct Rust type for trait signatures.
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

fn build_return(method: &MethodDef, ci: &str, tp: &std::collections::HashMap<&str, &str>) -> String {
    let inner = param_type(&method.return_type, ci, false, tp);
    if method.error_type.is_some() {
        format!("{ci}::Result<{inner}>")
    } else {
        inner
    }
}

fn gen_sync_body(out: &mut String, method: &MethodDef, ci: &str) {
    let name = &method.name;
    let has_error = method.error_type.is_some();

    // Use cached fields for known methods
    if name == "supported_languages" {
        writeln!(out, "        self.supported_languages.clone()").unwrap();
        return;
    }
    if name == "supports_language" {
        writeln!(
            out,
            "        self.supported_languages.iter().any(|l| l == {0})",
            method.params.first().map(|p| p.name.as_str()).unwrap_or("lang")
        )
        .unwrap();
        return;
    }
    if name == "backend_type" {
        // Use the fully-qualified core enum path
        if let TypeRef::Named(n) = &method.return_type {
            writeln!(out, "        {ci}::plugins::{n}::Custom").unwrap();
        } else {
            writeln!(out, "        Default::default()").unwrap();
        }
        return;
    }

    writeln!(out, "        Python::attach(|py| {{").unwrap();

    let py_args = sync_py_args(method);
    let call = if py_args.is_empty() {
        format!("self.python_obj.bind(py).call_method0(\"{name}\")")
    } else {
        format!("self.python_obj.bind(py).call_method1(\"{name}\", ({py_args}))")
    };

    if matches!(method.return_type, TypeRef::Unit) {
        writeln!(out, "            {call}").unwrap();
        if has_error {
            writeln!(out, "                .map(|_| ())").unwrap();
            write_error_map(out, name, ci, "self.name");
        } else {
            writeln!(out, "                .map(|_| ()).unwrap_or(())").unwrap();
        }
    } else {
        let ext = extract_ty(&method.return_type);
        writeln!(out, "            {call}").unwrap();
        writeln!(out, "                .and_then(|v| v.extract::<{ext}>())").unwrap();
        if has_error {
            write_error_map(out, name, ci, "self.name");
        } else {
            writeln!(out, "                .unwrap_or_default()").unwrap();
        }
    }

    writeln!(out, "        }})").unwrap();
}

fn gen_async_body(out: &mut String, method: &MethodDef, ci: &str) {
    let name = &method.name;

    writeln!(
        out,
        "        let python_obj = Python::attach(|py| self.python_obj.clone_ref(py));"
    )
    .unwrap();
    writeln!(out, "        let cached_name = self.name.clone();").unwrap();

    // Clone/convert params for the blocking closure
    for p in &method.params {
        match (&p.ty, p.is_ref) {
            (TypeRef::Bytes, true) => {
                writeln!(out, "        let {0} = {0}.to_vec();", p.name).unwrap();
            }
            (TypeRef::Path, true) => {
                writeln!(out, "        let {0}_str = {0}.to_string_lossy().to_string();", p.name).unwrap();
            }
            (TypeRef::Named(n), true) if n == "OcrConfig" => {
                writeln!(out, "        let language = {}.language.clone();", p.name).unwrap();
            }
            (TypeRef::Named(_), true) => {
                writeln!(
                    out,
                    "        let {0}_json = serde_json::to_string({0}).unwrap_or_default();",
                    p.name
                )
                .unwrap();
            }
            (_, true) => {
                writeln!(out, "        let {0} = {0}.to_owned();", p.name).unwrap();
            }
            _ => {
                writeln!(out, "        let {0} = {0}.clone();", p.name).unwrap();
            }
        }
    }

    writeln!(out).unwrap();
    writeln!(out, "        tokio::task::spawn_blocking(move || {{").unwrap();
    writeln!(out, "            Python::attach(|py| {{").unwrap();
    writeln!(out, "                let obj = python_obj.bind(py);").unwrap();

    let py_args = async_py_args(method);
    let call = if py_args.is_empty() {
        format!("obj.call_method0(\"{name}\")")
    } else {
        format!("obj.call_method1(\"{name}\", ({py_args}))")
    };

    if is_named(&method.return_type) {
        // Complex return: Python returns dict, convert via serde JSON
        writeln!(out, "                let py_result = {call}").unwrap();
        writeln!(out, "                    .map_err(|e| {ci}::KreuzbergError::Plugin {{").unwrap();
        writeln!(
            out,
            "                        message: format!(\"Plugin '{{}}' method '{name}' failed: {{}}\", cached_name, e),"
        )
        .unwrap();
        writeln!(out, "                        plugin_name: cached_name.clone(),").unwrap();
        writeln!(out, "                    }})?;").unwrap();
        writeln!(out, "                let json_val: String = py").unwrap();
        writeln!(out, "                    .import(\"json\")").unwrap();
        writeln!(
            out,
            "                    .and_then(|m| m.call_method1(\"dumps\", (py_result,)))"
        )
        .unwrap();
        writeln!(out, "                    .and_then(|v| v.extract())").unwrap();
        writeln!(out, "                    .map_err(|e| {ci}::KreuzbergError::Plugin {{").unwrap();
        writeln!(out, "                        message: format!(\"Plugin '{{}}': JSON serialization failed: {{}}\", cached_name, e),").unwrap();
        writeln!(out, "                        plugin_name: cached_name.clone(),").unwrap();
        writeln!(out, "                    }})?;").unwrap();
        writeln!(
            out,
            "                serde_json::from_str(&json_val).map_err(|e| {ci}::KreuzbergError::Plugin {{"
        )
        .unwrap();
        writeln!(
            out,
            "                    message: format!(\"Plugin '{{}}': deserialization failed: {{}}\", cached_name, e),"
        )
        .unwrap();
        writeln!(out, "                    plugin_name: cached_name.clone(),").unwrap();
        writeln!(out, "                }})").unwrap();
    } else {
        let ext = extract_ty(&method.return_type);
        if matches!(method.return_type, TypeRef::Unit) {
            writeln!(out, "                {call}").unwrap();
            writeln!(out, "                    .map(|_| ())").unwrap();
            writeln!(out, "                    .map_err(|e| {ci}::KreuzbergError::Plugin {{").unwrap();
            writeln!(out, "                        message: format!(\"Plugin '{{}}' method '{name}' failed: {{}}\", cached_name, e),").unwrap();
            writeln!(out, "                        plugin_name: cached_name.clone(),").unwrap();
            writeln!(out, "                    }})").unwrap();
        } else {
            writeln!(out, "                {call}").unwrap();
            writeln!(out, "                    .and_then(|v| v.extract::<{ext}>())").unwrap();
            writeln!(out, "                    .map_err(|e| {ci}::KreuzbergError::Plugin {{").unwrap();
            writeln!(out, "                        message: format!(\"Plugin '{{}}' method '{name}' failed: {{}}\", cached_name, e),").unwrap();
            writeln!(out, "                        plugin_name: cached_name.clone(),").unwrap();
            writeln!(out, "                    }})").unwrap();
        }
    }

    writeln!(out, "            }})").unwrap();
    writeln!(out, "        }})").unwrap();
    writeln!(out, "        .await").unwrap();
    writeln!(out, "        .map_err(|e| {ci}::KreuzbergError::Plugin {{").unwrap();
    writeln!(out, "            message: format!(\"spawn_blocking failed: {{}}\", e),").unwrap();
    writeln!(out, "            plugin_name: self.name.clone(),").unwrap();
    writeln!(out, "        }})?").unwrap();
}

fn write_error_map(out: &mut String, method_name: &str, ci: &str, name_expr: &str) {
    writeln!(out, "                .map_err(|e| {ci}::KreuzbergError::Plugin {{").unwrap();
    writeln!(
        out,
        "                    message: format!(\"Plugin '{{}}' method '{method_name}' failed: {{}}\", {name_expr}, e),"
    )
    .unwrap();
    writeln!(out, "                    plugin_name: {name_expr}.clone(),").unwrap();
    writeln!(out, "                }})").unwrap();
}

fn sync_py_args(method: &MethodDef) -> String {
    let args: Vec<String> = method
        .params
        .iter()
        .map(|p| match (&p.ty, p.is_ref) {
            (TypeRef::Bytes, true) => format!("pyo3::types::PyBytes::new(py, {})", p.name),
            (TypeRef::Path, true) => format!("{}.to_str().unwrap_or_default()", p.name),
            (TypeRef::Named(n), true) if n == "OcrConfig" => {
                format!("{}.language.as_str()", p.name)
            }
            (TypeRef::Named(_), true) => {
                format!("serde_json::to_string({}).unwrap_or_default()", p.name)
            }
            _ => p.name.clone(),
        })
        .collect();
    if args.len() == 1 {
        format!("{},", args[0])
    } else {
        args.join(", ")
    }
}

fn async_py_args(method: &MethodDef) -> String {
    let args: Vec<String> = method
        .params
        .iter()
        .map(|p| match (&p.ty, p.is_ref) {
            (TypeRef::Bytes, true) => format!("pyo3::types::PyBytes::new(py, &{})", p.name),
            (TypeRef::Path, true) => format!("{}_str.as_str()", p.name),
            (TypeRef::Named(n), true) if n == "OcrConfig" => "language.as_str()".into(),
            (TypeRef::Named(_), true) => format!("{}_json.as_str()", p.name),
            _ => p.name.clone(),
        })
        .collect();
    if args.len() == 1 {
        format!("{},", args[0])
    } else {
        args.join(", ")
    }
}

fn extract_ty(ty: &TypeRef) -> String {
    match ty {
        TypeRef::Primitive(p) => prim(p).to_string(),
        TypeRef::String | TypeRef::Path | TypeRef::Char => "String".into(),
        TypeRef::Bytes => "Vec<u8>".into(),
        TypeRef::Vec(inner) => format!("Vec<{}>", extract_ty(inner)),
        TypeRef::Optional(inner) => format!("Option<{}>", extract_ty(inner)),
        TypeRef::Named(name) => name.clone(),
        TypeRef::Unit => "()".into(),
        TypeRef::Map(k, v) => format!("std::collections::HashMap<{}, {}>", extract_ty(k), extract_ty(v)),
        TypeRef::Json => "String".into(),
        TypeRef::Duration => "u64".into(),
    }
}

fn is_named(ty: &TypeRef) -> bool {
    matches!(ty, TypeRef::Named(_))
}

fn gen_registration_fn(out: &mut String, cfg: &TraitBridgeConfig, struct_name: &str, trait_path: &str) {
    let register_fn = cfg
        .register_fn
        .as_deref()
        .expect("gen_registration_fn called without register_fn");
    let registry_getter = cfg
        .registry_getter
        .as_deref()
        .expect("gen_registration_fn called without registry_getter");
    let trait_name = &cfg.trait_name;

    writeln!(out, "#[pyfunction]").unwrap();
    writeln!(
        out,
        "pub fn {register_fn}(py: Python<'_>, backend: Py<PyAny>) -> PyResult<()> {{"
    )
    .unwrap();
    writeln!(out, "    let obj = backend.bind(py);").unwrap();
    writeln!(
        out,
        "    for method in &[\"name\", \"supported_languages\", \"process_image\"] {{"
    )
    .unwrap();
    writeln!(out, "        if !obj.hasattr(*method)? {{").unwrap();
    writeln!(
        out,
        "            return Err(pyo3::exceptions::PyAttributeError::new_err("
    )
    .unwrap();
    writeln!(
        out,
        "                format!(\"{trait_name} missing required method: {{}}\", method)"
    )
    .unwrap();
    writeln!(out, "            ));").unwrap();
    writeln!(out, "        }}").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "    let name: String = obj.call_method0(\"name\")?.extract()?;").unwrap();
    writeln!(
        out,
        "    let supported_languages: Vec<String> = obj.call_method0(\"supported_languages\")?.extract()?;"
    )
    .unwrap();
    writeln!(out).unwrap();
    writeln!(
        out,
        "    let wrapper = {struct_name} {{ python_obj: backend, name, supported_languages }};"
    )
    .unwrap();
    writeln!(out, "    let arc: Arc<dyn {trait_path}> = Arc::new(wrapper);").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "    py.detach(|| {{").unwrap();
    writeln!(out, "        let registry = {registry_getter}();").unwrap();
    writeln!(out, "        let mut registry = registry.write();").unwrap();
    writeln!(
        out,
        "        registry.register(arc).map_err(|e| pyo3::exceptions::PyRuntimeError::new_err("
    )
    .unwrap();
    writeln!(out, "            format!(\"Failed to register {trait_name}: {{}}\", e)").unwrap();
    writeln!(out, "        ))").unwrap();
    writeln!(out, "    }})?;").unwrap();
    writeln!(out, "    Ok(())").unwrap();
    writeln!(out, "}}").unwrap();
}

/// Collect registration function names for module init.
///
/// Bridges without a `register_fn` (per-call visitor pattern) are skipped.
pub fn collect_bridge_register_fns(configs: &[TraitBridgeConfig]) -> Vec<String> {
    configs.iter().filter_map(|c| c.register_fn.clone()).collect()
}

/// Imports needed by trait bridge generated code.
pub fn trait_bridge_imports(configs: &[TraitBridgeConfig]) -> Vec<&'static str> {
    if configs.is_empty() {
        return vec![];
    }
    vec![
        "use async_trait::async_trait;",
        "use pyo3::prelude::*;",
        "use std::sync::Arc;",
    ]
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
        // Try matching by the IR type name (for non-sanitized params).
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
            // Match by type alias (non-sanitized path).
            if let Some(type_name) = named {
                if bridge.type_alias.as_deref() == Some(type_name) {
                    return Some((idx, bridge));
                }
            }
            // Match by param name (sanitized path: the extractor collapsed the type to String
            // because it couldn't represent e.g. `Rc<RefCell<dyn Trait>>`).
            if bridge.param_name.as_deref() == Some(param.name.as_str()) {
                return Some((idx, bridge));
            }
        }
    }
    None
}

/// Generate a PyO3 free function that has one parameter replaced by `Py<PyAny>` (a trait bridge).
///
/// The bridge param becomes `Option<Py<PyAny>>` (or `Py<PyAny>` if not optional).
/// Before calling the core function the bridge is constructed:
/// ```rust
/// let visitor = visitor.map(|v| {
///     let bridge = PyHtmlVisitorBridge::new(v);
///     std::rc::Rc::new(std::cell::RefCell::new(bridge)) as html_to_markdown_rs::visitor::VisitorHandle
/// });
/// ```
#[allow(clippy::too_many_arguments)]
pub fn gen_bridge_function(
    func: &alef_core::ir::FunctionDef,
    bridge_param_idx: usize,
    bridge_cfg: &TraitBridgeConfig,
    mapper: &dyn alef_codegen::type_mapper::TypeMapper,
    cfg: &alef_codegen::generators::RustBindingConfig<'_>,
    adapter_bodies: &alef_codegen::generators::AdapterBodies,
    opaque_types: &ahash::AHashSet<String>,
    core_import: &str,
) -> String {
    use alef_codegen::generators::AsyncPattern;
    use alef_core::ir::TypeRef;

    let struct_name = format!("Py{}Bridge", bridge_cfg.trait_name);

    // Determine the VisitorHandle type alias path in the core crate.
    // Convention: the visitor module is always `{core_import}::visitor::VisitorHandle`.
    let handle_path = format!("{core_import}::visitor::VisitorHandle");

    // Build the param name for the bridge param
    let param_name = &func.params[bridge_param_idx].name;
    let bridge_param = &func.params[bridge_param_idx];
    // A param is optional either when its IR type is wrapped in Optional, OR when the
    // param's `optional` field is set (e.g. sanitized params where the extractor collapsed
    // `Rc<RefCell<dyn Trait>>` to `String` but preserved the optional metadata).
    let is_optional = bridge_param.optional
        || matches!(&bridge_param.ty, TypeRef::Optional(_));

    // Use gen_function to produce the "base" function, then intercept:
    // We generate a modified version manually because we need to replace the
    // signature type and inject pre-call wrapping code.

    // Build parameter list for the generated signature, replacing the bridge param
    let mut sig_parts = Vec::new();
    // For async Pyo3, first param is `py: Python<'py>`
    let func_needs_py =
        func.is_async && cfg.async_pattern == AsyncPattern::Pyo3FutureIntoPy;
    if func_needs_py {
        sig_parts.push("py: Python<'py>".to_string());
    }

    for (idx, p) in func.params.iter().enumerate() {
        if idx == bridge_param_idx {
            // Replace with Py<PyAny>
            if is_optional {
                sig_parts.push(format!("{}: Option<Py<PyAny>>", p.name));
            } else {
                sig_parts.push(format!("{}: Py<PyAny>", p.name));
            }
        } else {
            // Use the standard type mapping with optional promotion
            let promoted = idx > bridge_param_idx
                || func.params[..idx].iter().any(|pp| pp.optional);
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

    // Build the call args for the core function call
    // Reuse gen_function's body but via adapter injection: construct the adapter body manually.
    // The bridge wrapping code goes before the regular body.

    // Build the pre-call wrapping let-binding
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

    // Temporarily inject an adapter body that starts with the bridge wrap,
    // then delegates normally. We compose the full body here.

    // For the regular call args (non-bridge params), reuse the standard logic.
    // We need to also handle the serde-based options conversion.
    // The simplest correct approach: inject the bridge wrap as a preamble, then
    // call gen_function with an adapter body that includes this preamble plus
    // the standard serde-based conversion code.

    // Build standard call args the same way gen_function does via serde path
    // (since ConversionOptions has serde, and has_named_params will be true)
    let serde_err_conv = ".map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))";

    // Generate serde let-bindings for non-bridge Named params
    let serde_bindings: String = func.params.iter().enumerate().filter(|(idx, p)| {
        // Skip the bridge param — it's handled separately
        if *idx == bridge_param_idx {
            return false;
        }
        // Only process Named or Optional<Named> types that are not opaque
        let named = match &p.ty {
            TypeRef::Named(n) => Some(n.as_str()),
            TypeRef::Optional(inner) => {
                if let TypeRef::Named(n) = inner.as_ref() { Some(n.as_str()) } else { None }
            }
            _ => None,
        };
        named.is_some_and(|n| !opaque_types.contains(n))
    }).map(|(_, p)| {
        let name = &p.name;
        let core_path = format!("{core_import}::{}", match &p.ty {
            TypeRef::Named(n) => n.clone(),
            TypeRef::Optional(inner) => if let TypeRef::Named(n) = inner.as_ref() { n.clone() } else { String::new() },
            _ => String::new(),
        });
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
    }).collect();

    // Build the core function call args
    let call_args: Vec<String> = func.params.iter().enumerate().map(|(idx, p)| {
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
            // Non-opaque Named or Optional<Named>: use the _core let-binding
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
    }).collect();
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

    // Build the return expression
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
            format!("{bridge_wrap}\n    {serde_bindings}{core_call}{serde_err_conv}")
        } else {
            format!("{bridge_wrap}\n    {serde_bindings}{core_call}.map(|val| {return_wrap}){serde_err_conv}")
        }
    } else {
        format!("{bridge_wrap}\n    {serde_bindings}{core_call}")
    };

    // Build signature with pyo3 attributes
    let attr_inner = cfg
        .function_attr
        .trim_start_matches('#')
        .trim_start_matches('[')
        .trim_end_matches(']');

    let mut out = String::with_capacity(1024);
    if func.error_type.is_some() {
        writeln!(out, "#[allow(clippy::missing_errors_doc)]").ok();
    }
    writeln!(out, "#[{attr_inner}]").ok();
    if cfg.needs_signature {
        // Build PyO3 signature listing ALL params in order.
        // Required params appear by name, optional params appear with =None.
        // Once any param is optional, all subsequent params must also use =None.
        let mut seen_optional = false;
        let sig_parts: Vec<String> = func.params.iter().enumerate().map(|(idx, p)| {
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
        }).collect();
        let sig_str = sig_parts.join(", ");
        writeln!(out, "{}{}{}", cfg.signature_prefix, sig_str, cfg.signature_suffix).ok();
    }
    let func_name = &func.name;
    writeln!(out, "pub fn {func_name}{lifetime}({params_str}) -> {ret} {{").ok();
    writeln!(out, "    {body}").ok();
    writeln!(out, "}}").ok();

    // Suppress unused adapter_bodies warning
    let _ = adapter_bodies;

    out
}
