//! PyO3-specific trait bridge code generation.
//!
//! Generates Rust wrapper structs that implement Rust traits by delegating
//! to Python objects via PyO3.

pub use alef_codegen::generators::trait_bridge::find_bridge_field;
pub use alef_codegen::generators::trait_bridge::find_bridge_param;
use alef_codegen::generators::trait_bridge::{
    BridgeFieldMatch, BridgeOutput, TraitBridgeGenerator, TraitBridgeSpec,
    bridge_param_type as param_type, gen_bridge_all, visitor_param_type,
};
use alef_core::config::TraitBridgeConfig;
use alef_core::ir::{ApiSurface, MethodDef, TypeDef, TypeRef};
use std::collections::HashMap;
use std::fmt::Write;

/// PyO3-specific trait bridge generator.
/// Implements code generation for bridging Python objects to Rust traits.
pub struct Pyo3BridgeGenerator {
    /// Core crate import path (e.g., `"kreuzberg"`).
    pub core_import: String,
    /// Map of type name → fully-qualified Rust path for type references.
    pub type_paths: HashMap<String, String>,
    /// Error type name (e.g., `"KreuzbergError"`).
    pub error_type: String,
}

impl TraitBridgeGenerator for Pyo3BridgeGenerator {
    fn foreign_object_type(&self) -> &str {
        "Py<PyAny>"
    }

    fn bridge_imports(&self) -> Vec<String> {
        vec!["pyo3::prelude::*".to_string(), "std::sync::Arc".to_string()]
    }

    fn gen_sync_method_body(&self, method: &MethodDef, spec: &TraitBridgeSpec) -> String {
        let name = &method.name;
        let has_error = method.error_type.is_some();
        let mut out = String::with_capacity(512);

        writeln!(out, "Python::attach(|py| {{").ok();

        let py_args = self.sync_py_args(method);
        let call = if py_args.is_empty() {
            format!("self.inner.bind(py).call_method0(\"{name}\")")
        } else {
            format!("self.inner.bind(py).call_method1(\"{name}\", ({py_args}))")
        };

        if matches!(method.return_type, TypeRef::Unit) {
            writeln!(out, "            {call}").ok();
            if has_error {
                writeln!(out, "                .map(|_| ())").ok();
                self.write_error_map(&mut out, name, spec.core_import);
            } else {
                writeln!(out, "                .map(|_| ()).unwrap_or(())").ok();
            }
        } else {
            let ext = self.extract_ty(&method.return_type);
            writeln!(out, "            {call}").ok();
            // For Named types, extract as String and deserialize
            if matches!(method.return_type, TypeRef::Named(_)) {
                writeln!(out, "                .and_then(|v| v.extract::<String>())").ok();
                writeln!(out, "                .and_then(|s| serde_json::from_str::<{ext}>(&s).map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string())))").ok();
            } else {
                writeln!(out, "                .and_then(|v| v.extract::<{ext}>())").ok();
            }
            if has_error {
                self.write_error_map(&mut out, name, spec.core_import);
            } else {
                writeln!(out, "                .unwrap_or_default()").ok();
            }
        }

        writeln!(out, "        }})").ok();
        out
    }

    fn gen_async_method_body(&self, method: &MethodDef, spec: &TraitBridgeSpec) -> String {
        let name = &method.name;
        let mut out = String::with_capacity(1024);

        writeln!(out, "let python_obj = Python::attach(|py| self.inner.clone_ref(py));").ok();
        writeln!(out, "let cached_name = self.cached_name.clone();").ok();

        // Clone/convert params for the blocking closure
        for p in &method.params {
            match (&p.ty, p.is_ref) {
                (TypeRef::Bytes, true) => {
                    writeln!(out, "let {0} = {0}.to_vec();", p.name).ok();
                }
                (TypeRef::Path, true) => {
                    writeln!(out, "let {0}_str = {0}.to_string_lossy().to_string();", p.name).ok();
                }
                (TypeRef::Named(_), true) => {
                    writeln!(
                        out,
                        "let {0}_json = serde_json::to_string({0}).unwrap_or_default();",
                        p.name
                    )
                    .ok();
                }
                (_, true) => {
                    writeln!(out, "let {0} = {0}.to_owned();", p.name).ok();
                }
                _ => {
                    writeln!(out, "let {0} = {0}.clone();", p.name).ok();
                }
            }
        }

        writeln!(out).ok();
        writeln!(out, "tokio::task::spawn_blocking(move || {{").ok();
        writeln!(out, "    Python::attach(|py| {{").ok();
        writeln!(out, "        let obj = python_obj.bind(py);").ok();

        let py_args = self.async_py_args(method);
        let call = if py_args.is_empty() {
            format!("obj.call_method0(\"{name}\")")
        } else {
            format!("obj.call_method1(\"{name}\", ({py_args}))")
        };

        if self.is_named(&method.return_type) {
            // Complex return: Python returns dict, convert via serde JSON
            writeln!(out, "        let py_result = {call}").ok();
            writeln!(
                out,
                "            .map_err(|e| {}::KreuzbergError::Plugin {{",
                spec.core_import
            )
            .ok();
            writeln!(
                out,
                "                message: format!(\"Plugin '{{}}' method '{name}' failed: {{}}\", cached_name, e),"
            )
            .ok();
            writeln!(out, "                plugin_name: cached_name.clone(),").ok();
            writeln!(out, "            }})?;").ok();
            writeln!(out, "        let json_val: String = py").ok();
            writeln!(out, "            .import(\"json\")").ok();
            writeln!(
                out,
                "            .and_then(|m| m.call_method1(\"dumps\", (py_result,)))"
            )
            .ok();
            writeln!(out, "            .and_then(|v| v.extract())").ok();
            writeln!(
                out,
                "            .map_err(|e| {}::KreuzbergError::Plugin {{",
                spec.core_import
            )
            .ok();
            writeln!(
                out,
                "                message: format!(\"Plugin '{{}}': JSON serialization failed: {{}}\", cached_name, e),"
            )
            .ok();
            writeln!(out, "                plugin_name: cached_name.clone(),").ok();
            writeln!(out, "            }})?;").ok();
            let return_type =
                alef_codegen::generators::trait_bridge::format_type_ref(&method.return_type, &spec.type_paths);
            writeln!(
                out,
                "        serde_json::from_str::<{}>(&json_val).map_err(|e| {}::KreuzbergError::Plugin {{",
                return_type, spec.core_import
            )
            .ok();
            writeln!(
                out,
                "            message: format!(\"Plugin '{{}}': deserialization failed: {{}}\", cached_name, e),"
            )
            .ok();
            writeln!(out, "            plugin_name: cached_name.clone(),").ok();
            writeln!(out, "        }})").ok();
        } else {
            let ext = self.extract_ty(&method.return_type);
            if matches!(method.return_type, TypeRef::Unit) {
                writeln!(out, "        {call}").ok();
                writeln!(out, "            .map(|_| ())").ok();
                writeln!(
                    out,
                    "            .map_err(|e| {}::KreuzbergError::Plugin {{",
                    spec.core_import
                )
                .ok();
                writeln!(
                    out,
                    "                message: format!(\"Plugin '{{}}' method '{name}' failed: {{}}\", cached_name, e),"
                )
                .ok();
                writeln!(out, "                plugin_name: cached_name.clone(),").ok();
                writeln!(out, "            }})").ok();
            } else {
                writeln!(out, "        {call}").ok();
                writeln!(out, "            .and_then(|v| v.extract::<{ext}>())").ok();
                writeln!(
                    out,
                    "            .map_err(|e| {}::KreuzbergError::Plugin {{",
                    spec.core_import
                )
                .ok();
                writeln!(
                    out,
                    "                message: format!(\"Plugin '{{}}' method '{name}' failed: {{}}\", cached_name, e),"
                )
                .ok();
                writeln!(out, "                plugin_name: cached_name.clone(),").ok();
                writeln!(out, "            }})").ok();
            }
        }

        writeln!(out, "    }})").ok();
        writeln!(out, "}})").ok();
        writeln!(out, ".await").ok();
        writeln!(out, ".map_err(|e| {}::KreuzbergError::Plugin {{", spec.core_import).ok();
        writeln!(out, "    message: format!(\"spawn_blocking failed: {{}}\", e),").ok();
        writeln!(out, "    plugin_name: self.cached_name.clone(),").ok();
        writeln!(out, "}})").ok();
        writeln!(out, ".flatten()").ok();
        out
    }

    fn gen_constructor(&self, spec: &TraitBridgeSpec) -> String {
        let wrapper = spec.wrapper_name();
        let mut out = String::with_capacity(512);

        writeln!(out, "impl {wrapper} {{").ok();
        writeln!(out, "    /// Create a new bridge wrapping a Python object.").ok();
        writeln!(out, "    ///").ok();
        writeln!(
            out,
            "    /// Validates that the Python object provides all required methods."
        )
        .ok();
        writeln!(out, "    pub fn new(python_obj: Py<PyAny>) -> PyResult<Self> {{").ok();
        writeln!(out, "        Python::attach(|py| {{").ok();
        writeln!(out, "            let obj = python_obj.bind(py);").ok();

        // Validate all required methods exist
        for req_method in spec.required_methods() {
            writeln!(
                out,
                "            if !obj.hasattr(\"{}\").unwrap_or(false) {{",
                req_method.name
            )
            .ok();
            writeln!(
                out,
                "                return Err(pyo3::exceptions::PyAttributeError::new_err("
            )
            .ok();
            writeln!(
                out,
                "                    \"Python object missing required method: {}\",",
                req_method.name
            )
            .ok();
            writeln!(out, "                ));").ok();
            writeln!(out, "            }}").ok();
        }

        // Extract and cache name
        writeln!(out, "            let cached_name: String = obj").ok();
        writeln!(out, "                .call_method0(\"name\")").ok();
        writeln!(out, "                .and_then(|v| v.extract())").ok();
        writeln!(out, "                .unwrap_or_else(|_| \"unknown\".to_string());").ok();

        writeln!(out).ok();
        writeln!(out, "            Ok(Self {{").ok();
        writeln!(out, "                inner: python_obj,").ok();
        writeln!(out, "                cached_name,").ok();
        writeln!(out, "            }})").ok();
        writeln!(out, "        }})").ok();
        writeln!(out, "    }}").ok();
        writeln!(out, "}}").ok();
        out
    }

    fn gen_registration_fn(&self, spec: &TraitBridgeSpec) -> String {
        let Some(register_fn) = spec.bridge_config.register_fn.as_deref() else {
            return String::new();
        };
        let Some(registry_getter) = spec.bridge_config.registry_getter.as_deref() else {
            return String::new();
        };
        let wrapper = spec.wrapper_name();
        let trait_path = spec.trait_path();

        let mut out = String::with_capacity(1024);

        writeln!(out, "#[pyfunction]").ok();
        writeln!(
            out,
            "pub fn {register_fn}(py: Python<'_>, backend: Py<PyAny>) -> PyResult<()> {{"
        )
        .ok();

        // Validate required methods
        let req_methods: Vec<&MethodDef> = spec.required_methods();
        if !req_methods.is_empty() {
            writeln!(
                out,
                "    let required_methods = [{}];",
                req_methods
                    .iter()
                    .map(|m| format!("\"{}\"", m.name))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
            .ok();
            writeln!(out, "    let obj = backend.bind(py);").ok();
            writeln!(out, "    for method in &required_methods {{").ok();
            writeln!(out, "        if !obj.hasattr(*method)? {{").ok();
            writeln!(
                out,
                "            return Err(pyo3::exceptions::PyAttributeError::new_err("
            )
            .ok();
            writeln!(
                out,
                "                format!(\"Backend missing required method: {{}}\", method)"
            )
            .ok();
            writeln!(out, "            ));").ok();
            writeln!(out, "        }}").ok();
            writeln!(out, "    }}").ok();
        }

        // Create the wrapper using the constructor
        writeln!(out).ok();
        writeln!(out, "    let wrapper = {wrapper}::new(backend)?;").ok();
        writeln!(out, "    let arc: Arc<dyn {trait_path}> = Arc::new(wrapper);").ok();
        writeln!(out).ok();

        // Register in the plugin registry
        let extra = spec
            .bridge_config
            .register_extra_args
            .as_deref()
            .map(|a| format!(", {a}"))
            .unwrap_or_default();
        writeln!(out, "    py.detach(|| {{").ok();
        writeln!(out, "        let registry = {registry_getter}();").ok();
        writeln!(out, "        let mut registry = registry.write();").ok();
        writeln!(
            out,
            "        registry.register(arc{extra}).map_err(|e| pyo3::exceptions::PyRuntimeError::new_err("
        )
        .ok();
        writeln!(out, "            format!(\"Failed to register backend: {{}}\", e)").ok();
        writeln!(out, "        ))").ok();
        writeln!(out, "    }})?;").ok();
        writeln!(out, "    Ok(())").ok();
        writeln!(out, "}}").ok();
        out
    }
}

impl Pyo3BridgeGenerator {
    /// Extract the Python type that corresponds to a Rust TypeRef.
    fn extract_ty(&self, ty: &TypeRef) -> String {
        match ty {
            TypeRef::Primitive(p) => self.prim(p).to_string(),
            TypeRef::String | TypeRef::Path | TypeRef::Char => "String".into(),
            TypeRef::Bytes => "Vec<u8>".into(),
            TypeRef::Vec(inner) => format!("Vec<{}>", self.extract_ty(inner)),
            TypeRef::Optional(inner) => format!("Option<{}>", self.extract_ty(inner)),
            TypeRef::Named(name) => {
                // Qualify Named types with core crate path if available in type_paths
                self.type_paths
                    .get(name.as_str())
                    .map(|p| p.replace('-', "_"))
                    .unwrap_or_else(|| format!("{}::{}", self.core_import, name))
            }
            TypeRef::Unit => "()".into(),
            TypeRef::Map(k, v) => format!(
                "std::collections::HashMap<{}, {}>",
                self.extract_ty(k),
                self.extract_ty(v)
            ),
            TypeRef::Json => "String".into(),
            TypeRef::Duration => "u64".into(),
        }
    }

    /// Get the Rust string representation of a primitive type.
    fn prim(&self, p: &alef_core::ir::PrimitiveType) -> &'static str {
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

    /// Build Python call argument expressions for a sync method.
    fn sync_py_args(&self, method: &MethodDef) -> String {
        let args: Vec<String> = method
            .params
            .iter()
            .map(|p| match (&p.ty, p.is_ref) {
                (TypeRef::Bytes, true) => format!("pyo3::types::PyBytes::new(py, {})", p.name),
                (TypeRef::Path, true) => format!("{}.to_str().unwrap_or_default()", p.name),
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

    /// Build Python call argument expressions for an async method.
    fn async_py_args(&self, method: &MethodDef) -> String {
        let args: Vec<String> = method
            .params
            .iter()
            .map(|p| match (&p.ty, p.is_ref) {
                (TypeRef::Bytes, true) => format!("pyo3::types::PyBytes::new(py, &{})", p.name),
                (TypeRef::Path, true) => format!("{}_str.as_str()", p.name),
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

    /// Check if a TypeRef is a Named type.
    fn is_named(&self, ty: &TypeRef) -> bool {
        matches!(ty, TypeRef::Named(_))
    }

    /// Write error mapping code to the output.
    fn write_error_map(&self, out: &mut String, method_name: &str, core_import: &str) {
        writeln!(
            out,
            "                .map_err(|e| {core_import}::KreuzbergError::Plugin {{"
        )
        .ok();
        writeln!(
            out,
            "                    message: format!(\"Plugin '{{}}' method '{method_name}' failed: {{}}\", self.cached_name, e),"
        )
        .ok();
        writeln!(out, "                    plugin_name: self.cached_name.clone(),").ok();
        writeln!(out, "                }})").ok();
    }
}

/// Generate all trait bridge code for a given trait type and bridge config.
pub fn gen_trait_bridge(
    trait_type: &TypeDef,
    bridge_cfg: &TraitBridgeConfig,
    core_import: &str,
    error_type: &str,
    error_constructor: &str,
    api: &ApiSurface,
) -> BridgeOutput {
    // Build type name → rust_path lookup for qualifying Named types in signatures
    let type_paths: HashMap<String, String> = api
        .types
        .iter()
        .map(|t| (t.name.clone(), t.rust_path.replace('-', "_")))
        .chain(
            api.enums
                .iter()
                .map(|e| (e.name.clone(), e.rust_path.replace('-', "_"))),
        )
        .collect();

    // Determine bridge pattern: visitor-style (all methods have defaults, no registry) vs
    // plugin-style (cached fields, registry, super-trait).
    let is_visitor_bridge = bridge_cfg.type_alias.is_some()
        && bridge_cfg.register_fn.is_none()
        && bridge_cfg.super_trait.is_none()
        && trait_type.methods.iter().all(|m| m.has_default_impl);

    if is_visitor_bridge {
        let trait_path = trait_type.rust_path.replace('-', "_");
        let struct_name = format!("Py{}Bridge", bridge_cfg.trait_name);
        let code = gen_visitor_bridge(trait_type, bridge_cfg, &struct_name, &trait_path, &type_paths);
        BridgeOutput { imports: vec![], code }
    } else {
        // Use the IR-driven TraitBridgeGenerator infrastructure
        let generator = Pyo3BridgeGenerator {
            core_import: core_import.to_string(),
            type_paths: type_paths.clone(),
            error_type: error_type.to_string(),
        };
        let spec = TraitBridgeSpec {
            trait_def: trait_type,
            bridge_config: bridge_cfg,
            core_import,
            wrapper_prefix: "Py",
            type_paths,
            error_type: error_type.to_string(),
            error_constructor: error_constructor.to_string(),
        };
        gen_bridge_all(&spec, &generator)
    }
}

/// Generate a visitor-style bridge: thin wrapper over `Py<PyAny>` where every trait method
/// tries to call the corresponding Python method, falling back to the default if absent.
///
/// This pattern is used for traits where:
/// - All methods have default implementations (e.g., `HtmlVisitor`)
/// - No registration function is needed (per-call construction via `type_alias`)
/// - No super-trait forwarding
fn gen_visitor_bridge(
    trait_type: &TypeDef,
    bridge_cfg: &TraitBridgeConfig,
    struct_name: &str,
    trait_path: &str,
    type_paths: &HashMap<String, String>,
) -> String {
    let mut out = String::with_capacity(4096);
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
    writeln!(
        out,
        "    d.set_item(\"node_type\", format!(\"{{:?}}\", ctx.node_type)).unwrap_or(());"
    )
    .unwrap();
    writeln!(out, "    d.set_item(\"tag_name\", &ctx.tag_name).unwrap_or(());").unwrap();
    writeln!(out, "    d.set_item(\"depth\", ctx.depth).unwrap_or(());").unwrap();
    writeln!(
        out,
        "    d.set_item(\"index_in_parent\", ctx.index_in_parent).unwrap_or(());"
    )
    .unwrap();
    writeln!(out, "    d.set_item(\"is_inline\", ctx.is_inline).unwrap_or(());").unwrap();
    writeln!(
        out,
        "    d.set_item(\"parent_tag\", ctx.parent_tag.as_deref()).unwrap_or(());"
    )
    .unwrap();
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
        gen_visitor_method(&mut out, method, trait_path, type_paths);
    }
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
    out
}

/// Generate a single visitor-style trait method that tries Python dispatch, falls back to default.
///
/// For each method the generated code:
/// 1. Checks if the Python object has an attribute with this method's name.
/// 2. If yes, calls the method with converted arguments and converts the Python return value
///    to the appropriate Rust return type.
/// 3. If no (attribute absent), returns the trait default (typically `VisitResult::Continue`).
fn gen_visitor_method(out: &mut String, method: &MethodDef, _trait_path: &str, type_paths: &HashMap<String, String>) {
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
        TypeRef::Named(n) => type_paths.get(n).cloned().unwrap_or_else(|| n.clone()),
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
    writeln!(
        out,
        "                            other => {ret_ty}::Custom(other.to_string()),"
    )
    .unwrap();
    writeln!(out, "                        }}").unwrap();
    writeln!(out, "                    }} else if result.is_none() {{").unwrap();
    writeln!(out, "                        {ret_ty}::Continue").unwrap();
    writeln!(out, "                    }} else {{").unwrap();
    // Try dict protocol: {"custom": "..."} or {"error": "..."}
    writeln!(
        out,
        "                        let py_dict = result.downcast::<pyo3::types::PyDict>();"
    )
    .unwrap();
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

/// Generate a PyO3 free function that has one parameter replaced by `Py<PyAny>` (a trait bridge).
///
/// The bridge param becomes `Option<Py<PyAny>>` (or `Py<PyAny>` if not optional).
/// Before calling the core function the bridge is constructed:
/// ```rust,ignore
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
    let is_optional = bridge_param.optional || matches!(&bridge_param.ty, TypeRef::Optional(_));

    // Use gen_function to produce the "base" function, then intercept:
    // We generate a modified version manually because we need to replace the
    // signature type and inject pre-call wrapping code.

    // Build parameter list for the generated signature, replacing the bridge param
    let mut sig_parts = Vec::new();
    // For async Pyo3, first param is `py: Python<'py>`
    let func_needs_py = func.is_async && cfg.async_pattern == AsyncPattern::Pyo3FutureIntoPy;
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
    let serde_bindings: String = func
        .params
        .iter()
        .enumerate()
        .filter(|(idx, p)| {
            // Skip the bridge param — it's handled separately
            if *idx == bridge_param_idx {
                return false;
            }
            // Only process Named or Optional<Named> types that are not opaque
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
        })
        .collect();

    // Build the core function call args
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

    // Build the return expression
    let return_wrap = match &func.return_type {
        TypeRef::Named(name) if opaque_types.contains(name.as_str()) => {
            format!("{name} {{ inner: std::sync::Arc::new(val) }}")
        }
        TypeRef::Named(_) => "val.into()".to_string(),
        TypeRef::String | TypeRef::Bytes => "val.into()".to_string(),
        _ => "val".to_string(),
    };

    let body = if let Some(ref error_type) = func.error_type {
        // Build the error conversion. For known error types, use the dedicated converter
        // function (e.g. `conversion_error_to_py_err`). For generic/unknown error types
        // (anyhow::Error, etc.), fall back to PyRuntimeError.
        let core_err_conv = if error_type.contains("::") || error_type == "Error" {
            // Generic error type — use PyRuntimeError
            ".map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))".to_string()
        } else {
            // Known error type — convert PascalCase to snake_case for converter function name
            let snake_error = {
                let mut s = String::with_capacity(error_type.len() + 4);
                for (i, c) in error_type.chars().enumerate() {
                    if c.is_uppercase() {
                        if i > 0 {
                            s.push('_');
                        }
                        s.push(c.to_ascii_lowercase());
                    } else {
                        s.push(c);
                    }
                }
                s
            };
            format!(".map_err({snake_error}_to_py_err)")
        };
        if return_wrap == "val" {
            format!("{bridge_wrap}\n    {serde_bindings}{core_call}{core_err_conv}")
        } else {
            format!("{bridge_wrap}\n    {serde_bindings}{core_call}.map(|val| {return_wrap}){core_err_conv}")
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
        let sig_parts: Vec<String> = func
            .params
            .iter()
            .enumerate()
            .map(|(idx, p)| {
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
            })
            .collect();
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

/// Generate a PyO3 free function for a bridge whose handle lives on an options struct field
/// (`bind_via = "options_field"`).
///
/// The generated function:
/// 1. Accepts the binding-side options struct (which carries `visitor: Option<Py<PyAny>>`).
/// 2. Extracts the visitor, constructs the bridge wrapper, sets it on core options.
/// 3. Calls the core function with the prepared core options.
///
/// No separate `convert_with_visitor` export is emitted — this replaces it entirely.
#[allow(clippy::too_many_arguments)]
pub fn gen_bridge_field_function(
    func: &alef_core::ir::FunctionDef,
    bridge_match: &BridgeFieldMatch<'_>,
    mapper: &dyn alef_codegen::type_mapper::TypeMapper,
    cfg: &alef_codegen::generators::RustBindingConfig<'_>,
    adapter_bodies: &alef_codegen::generators::AdapterBodies,
    opaque_types: &ahash::AHashSet<String>,
    core_import: &str,
) -> String {
    use alef_codegen::generators::AsyncPattern;

    let struct_name = format!("Py{}Bridge", bridge_match.bridge.trait_name);
    let handle_path = format!("{core_import}::visitor::VisitorHandle");

    let param_idx = bridge_match.param_index;
    let param_name = &func.params[param_idx].name;
    let param_is_optional = bridge_match.param_is_optional;
    let field_name = &bridge_match.field_name;

    let func_needs_py = func.is_async && cfg.async_pattern == AsyncPattern::Pyo3FutureIntoPy;
    let mut sig_parts = Vec::new();
    if func_needs_py {
        sig_parts.push("py: Python<'py>".to_string());
    }

    let serde_err_conv = ".map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))";

    for (idx, p) in func.params.iter().enumerate() {
        if idx == param_idx {
            let options_type_name = &bridge_match.options_type;
            if param_is_optional {
                sig_parts.push(format!("{param_name}: Option<{options_type_name}>"));
            } else {
                sig_parts.push(format!("{param_name}: {options_type_name}"));
            }
        } else {
            let ty = if p.optional {
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

    // Generate serde let-bindings for non-options-struct Named params
    let serde_bindings: String = func
        .params
        .iter()
        .enumerate()
        .filter(|(idx, p)| {
            if *idx == param_idx {
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
                    TypeRef::Optional(inner) => {
                        if let TypeRef::Named(n) = inner.as_ref() {
                            n.clone()
                        } else {
                            String::new()
                        }
                    }
                    _ => String::new(),
                }
            );
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
        })
        .collect();

    // Build the options extraction + bridge construction + core conversion preamble.
    let options_core_type = format!("{core_import}::{}", bridge_match.options_type);
    let preamble = if param_is_optional {
        format!(
            "// Extract visitor from binding options before serde round-trip (visitor is #[serde(skip)]).\n    \
             // Use clone_ref rather than .clone() — Py<PyAny> requires the `py-clone` feature for\n    \
             // Clone, but clone_ref(py) increments the refcount without that feature.\n    \
             let __visitor = {param_name}.as_ref().and_then(|o| o.{field_name}.as_ref().map(|v| Python::attach(|py| v.clone_ref(py))));\n    \
             let __options_core: Option<{options_core_type}> = {param_name}.map(|v| {{\n        \
             let json = serde_json::to_string(&v){serde_err_conv}?;\n        \
             serde_json::from_str::<{options_core_type}>(&json){serde_err_conv}\n    \
             }}).transpose()?;\n    \
             let {param_name}_core = __options_core.map(|mut opts| {{\n        \
             if let Some(v) = __visitor {{\n            \
             let bridge = {struct_name}::new(v);\n            \
             opts.{field_name} = Some(std::rc::Rc::new(std::cell::RefCell::new(bridge)) as {handle_path});\n        \
             }}\n        \
             opts\n    \
             }});"
        )
    } else {
        format!(
            "// Extract visitor from binding options before serde round-trip (visitor is #[serde(skip)]).\n    \
             // Use clone_ref rather than .clone() — Py<PyAny> requires the `py-clone` feature for\n    \
             // Clone, but clone_ref(py) increments the refcount without that feature.\n    \
             let __visitor = {param_name}.{field_name}.as_ref().map(|v| Python::attach(|py| v.clone_ref(py)));\n    \
             let {param_name}_json = serde_json::to_string(&{param_name}){serde_err_conv}?;\n    \
             let mut {param_name}_core: {options_core_type} = serde_json::from_str(&{param_name}_json){serde_err_conv}?;\n    \
             if let Some(__v) = __visitor {{\n        \
             let bridge = {struct_name}::new(__v);\n        \
             {param_name}_core.{field_name} = Some(std::rc::Rc::new(std::cell::RefCell::new(bridge)) as {handle_path});\n    \
             }}"
        )
    };

    // Build the core function call args
    let call_args: Vec<String> = func
        .params
        .iter()
        .enumerate()
        .map(|(idx, p)| {
            if idx == param_idx {
                return format!("{param_name}_core");
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

    let body = if let Some(ref error_type) = func.error_type {
        let core_err_conv = if error_type.contains("::") || error_type == "Error" {
            ".map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))".to_string()
        } else {
            let snake_error = {
                let mut s = String::with_capacity(error_type.len() + 4);
                for (i, c) in error_type.chars().enumerate() {
                    if c.is_uppercase() {
                        if i > 0 {
                            s.push('_');
                        }
                        s.push(c.to_ascii_lowercase());
                    } else {
                        s.push(c);
                    }
                }
                s
            };
            format!(".map_err({snake_error}_to_py_err)")
        };
        if return_wrap == "val" {
            format!("{preamble}\n    {serde_bindings}{core_call}{core_err_conv}")
        } else {
            format!("{preamble}\n    {serde_bindings}{core_call}.map(|val| {return_wrap}){core_err_conv}")
        }
    } else {
        format!("{preamble}\n    {serde_bindings}{core_call}")
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
        let mut seen_optional = false;
        let sig_parts: Vec<String> = func
            .params
            .iter()
            .enumerate()
            .map(|(idx, p)| {
                let this_optional = if idx == param_idx { param_is_optional } else { p.optional };
                if this_optional {
                    seen_optional = true;
                }
                if this_optional || seen_optional {
                    format!("{}=None", p.name)
                } else {
                    p.name.clone()
                }
            })
            .collect();
        let sig_str = sig_parts.join(", ");
        writeln!(out, "{}{}{}", cfg.signature_prefix, sig_str, cfg.signature_suffix).ok();
    }
    let func_name = &func.name;
    writeln!(out, "pub fn {func_name}{lifetime}({params_str}) -> {ret} {{").ok();
    writeln!(out, "    {body}").ok();
    writeln!(out, "}}").ok();

    let _ = adapter_bodies;
    out
}
