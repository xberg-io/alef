//! PyO3-specific trait bridge code generation.
//!
//! Generates Rust wrapper structs that implement Rust traits by delegating
//! to Python objects via PyO3.

use alef_core::config::TraitBridgeConfig;
use alef_core::ir::{MethodDef, TypeDef, TypeRef};
use std::fmt::Write;

/// Generate all trait bridge code for a given trait type and bridge config.
pub fn gen_trait_bridge(
    trait_type: &TypeDef,
    bridge_cfg: &TraitBridgeConfig,
    core_import: &str,
) -> String {
    let mut out = String::with_capacity(8192);
    let struct_name = format!("Py{}Bridge", bridge_cfg.trait_name);
    let trait_path = trait_type.rust_path.replace('-', "_");

    // Wrapper struct with cached fields
    writeln!(out, "pub struct {struct_name} {{").unwrap();
    writeln!(out, "    python_obj: Py<PyAny>,").unwrap();
    writeln!(out, "    name: String,").unwrap();
    writeln!(out, "    supported_languages: Vec<String>,").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // Plugin super-trait impl
    if bridge_cfg.super_trait.is_some() {
        gen_plugin_impl(&mut out, &struct_name, core_import);
    }

    // Main trait impl
    writeln!(out, "#[async_trait]").unwrap();
    writeln!(out, "impl {trait_path} for {struct_name} {{").unwrap();
    for method in &trait_type.methods {
        if method.trait_source.is_some() {
            continue;
        }
        gen_method(&mut out, method, core_import);
    }
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // Registration function
    gen_registration_fn(&mut out, bridge_cfg, &struct_name, &trait_path);

    out
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
    writeln!(out, "                obj.call_method0(\"initialize\").map_err(|e| {ci}::KreuzbergError::Plugin {{").unwrap();
    writeln!(out, "                    message: format!(\"Plugin '{{}}' initialize failed: {{}}\", self.name, e),").unwrap();
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
    writeln!(out, "                obj.call_method0(\"shutdown\").map_err(|e| {ci}::KreuzbergError::Plugin {{").unwrap();
    writeln!(out, "                    message: format!(\"Plugin '{{}}' shutdown failed: {{}}\", self.name, e),").unwrap();
    writeln!(out, "                    plugin_name: self.name.clone(),").unwrap();
    writeln!(out, "                }})?;").unwrap();
    writeln!(out, "            }}").unwrap();
    writeln!(out, "            Ok(())").unwrap();
    writeln!(out, "        }})").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}

fn gen_method(out: &mut String, method: &MethodDef, ci: &str) {
    let name = &method.name;
    let sig = build_signature(method, ci);
    let ret = build_return(method, ci);

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
fn build_signature(method: &MethodDef, ci: &str) -> String {
    let mut parts = Vec::new();
    if method.receiver.is_some() {
        parts.push("&self".to_string());
    }
    for p in &method.params {
        parts.push(format!("{}: {}", p.name, param_type(&p.ty, ci, p.is_ref)));
    }
    parts.join(", ")
}

/// Map TypeRef to correct Rust type for trait signatures.
fn param_type(ty: &TypeRef, ci: &str, is_ref: bool) -> String {
    match ty {
        TypeRef::Bytes if is_ref => "&[u8]".into(),
        TypeRef::Bytes => "Vec<u8>".into(),
        TypeRef::String if is_ref => "&str".into(),
        TypeRef::String => "String".into(),
        TypeRef::Path if is_ref => "&std::path::Path".into(),
        TypeRef::Path => "std::path::PathBuf".into(),
        TypeRef::Named(n) if is_ref => format!("&{ci}::{n}"),
        TypeRef::Named(n) => format!("{ci}::{n}"),
        TypeRef::Vec(inner) => format!("Vec<{}>", param_type(inner, ci, false)),
        TypeRef::Optional(inner) => format!("Option<{}>", param_type(inner, ci, false)),
        TypeRef::Primitive(p) => prim(p).into(),
        TypeRef::Unit => "()".into(),
        TypeRef::Char => "char".into(),
        TypeRef::Map(k, v) => format!(
            "std::collections::HashMap<{}, {}>",
            param_type(k, ci, false),
            param_type(v, ci, false)
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

fn build_return(method: &MethodDef, ci: &str) -> String {
    let inner = param_type(&method.return_type, ci, false);
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
        writeln!(out, "        {ci}::plugins::OcrBackendType::Custom").unwrap();
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
                writeln!(
                    out,
                    "        let {0}_str = {0}.to_string_lossy().to_string();",
                    p.name
                )
                .unwrap();
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
        writeln!(out, "                        message: format!(\"Plugin '{{}}' method '{name}' failed: {{}}\", cached_name, e),").unwrap();
        writeln!(out, "                        plugin_name: cached_name.clone(),").unwrap();
        writeln!(out, "                    }})?;").unwrap();
        writeln!(out, "                let json_val: String = py").unwrap();
        writeln!(out, "                    .import(\"json\")").unwrap();
        writeln!(out, "                    .and_then(|m| m.call_method1(\"dumps\", (py_result,)))").unwrap();
        writeln!(out, "                    .and_then(|v| v.extract())").unwrap();
        writeln!(out, "                    .map_err(|e| {ci}::KreuzbergError::Plugin {{").unwrap();
        writeln!(out, "                        message: format!(\"Plugin '{{}}': JSON serialization failed: {{}}\", cached_name, e),").unwrap();
        writeln!(out, "                        plugin_name: cached_name.clone(),").unwrap();
        writeln!(out, "                    }})?;").unwrap();
        writeln!(out, "                serde_json::from_str(&json_val).map_err(|e| {ci}::KreuzbergError::Plugin {{").unwrap();
        writeln!(out, "                    message: format!(\"Plugin '{{}}': deserialization failed: {{}}\", cached_name, e),").unwrap();
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
    writeln!(
        out,
        "            message: format!(\"spawn_blocking failed: {{}}\", e),"
    )
    .unwrap();
    writeln!(out, "            plugin_name: self.name.clone(),").unwrap();
    writeln!(out, "        }})?").unwrap();
}

fn write_error_map(out: &mut String, method_name: &str, ci: &str, name_expr: &str) {
    writeln!(
        out,
        "                .map_err(|e| {ci}::KreuzbergError::Plugin {{"
    )
    .unwrap();
    writeln!(
        out,
        "                    message: format!(\"Plugin '{{}}' method '{method_name}' failed: {{}}\", {name_expr}, e),"
    )
    .unwrap();
    writeln!(
        out,
        "                    plugin_name: {name_expr}.clone(),"
    )
    .unwrap();
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
        TypeRef::Map(k, v) => format!(
            "std::collections::HashMap<{}, {}>",
            extract_ty(k),
            extract_ty(v)
        ),
        TypeRef::Json => "String".into(),
        TypeRef::Duration => "u64".into(),
    }
}

fn is_named(ty: &TypeRef) -> bool {
    matches!(ty, TypeRef::Named(_))
}

fn gen_registration_fn(
    out: &mut String,
    cfg: &TraitBridgeConfig,
    struct_name: &str,
    trait_path: &str,
) {
    let register_fn = &cfg.register_fn;
    let registry_getter = &cfg.registry_getter;
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
    writeln!(
        out,
        "    let name: String = obj.call_method0(\"name\")?.extract()?;"
    )
    .unwrap();
    writeln!(out, "    let supported_languages: Vec<String> = obj.call_method0(\"supported_languages\")?.extract()?;").unwrap();
    writeln!(out).unwrap();
    writeln!(
        out,
        "    let wrapper = {struct_name} {{ python_obj: backend, name, supported_languages }};"
    )
    .unwrap();
    writeln!(
        out,
        "    let arc: Arc<dyn {trait_path}> = Arc::new(wrapper);"
    )
    .unwrap();
    writeln!(out).unwrap();
    writeln!(out, "    py.allow_threads(|| {{").unwrap();
    writeln!(out, "        let registry = {registry_getter}();").unwrap();
    writeln!(out, "        let mut registry = registry.write();").unwrap();
    writeln!(
        out,
        "        registry.register(arc).map_err(|e| pyo3::exceptions::PyRuntimeError::new_err("
    )
    .unwrap();
    writeln!(
        out,
        "            format!(\"Failed to register {trait_name}: {{}}\", e)"
    )
    .unwrap();
    writeln!(out, "        ))").unwrap();
    writeln!(out, "    }})?;").unwrap();
    writeln!(out, "    Ok(())").unwrap();
    writeln!(out, "}}").unwrap();
}

/// Collect registration function names for module init.
pub fn collect_bridge_register_fns(configs: &[TraitBridgeConfig]) -> Vec<String> {
    configs.iter().map(|c| c.register_fn.clone()).collect()
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
