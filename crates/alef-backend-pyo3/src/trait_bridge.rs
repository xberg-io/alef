//! PyO3-specific trait bridge code generation.
//!
//! Generates Rust wrapper structs that implement Rust traits by delegating
//! to Python objects via PyO3. Also generates `#[pyfunction]` registration
//! functions that validate and register Python plugin objects.

use alef_core::config::TraitBridgeConfig;
use alef_core::ir::{MethodDef, TypeDef, TypeRef};
use std::fmt::Write;

/// Generate all trait bridge code for a given trait type and bridge config.
///
/// Returns a String containing:
/// 1. The wrapper struct definition
/// 2. Trait impl(s) for the wrapper (including super-trait impls)
/// 3. A `#[pyfunction]` registration function
pub fn gen_trait_bridge(trait_type: &TypeDef, bridge_cfg: &TraitBridgeConfig, core_import: &str) -> String {
    let mut out = String::with_capacity(4096);

    let trait_name = &bridge_cfg.trait_name;
    let struct_name = format!("Py{trait_name}Bridge");

    // --- Wrapper struct ---
    gen_bridge_struct(&mut out, &struct_name);

    // --- Super-trait impl (e.g., `impl Plugin for PyOcrBackendBridge`) ---
    if let Some(super_trait) = &bridge_cfg.super_trait {
        if let Some(super_type) = find_super_trait_type(trait_type, super_trait) {
            gen_super_trait_impl(&mut out, &struct_name, &super_type, core_import);
        }
    }

    // --- Main trait impl ---
    gen_trait_impl(&mut out, &struct_name, trait_type, core_import);

    // --- Registration function ---
    gen_registration_fn(&mut out, trait_type, bridge_cfg, &struct_name, core_import);

    out
}

/// Generate the bridge wrapper struct holding a `Py<PyAny>` and cached name.
fn gen_bridge_struct(out: &mut String, struct_name: &str) {
    writeln!(out, "pub struct {struct_name} {{").unwrap();
    writeln!(out, "    python_obj: Py<PyAny>,").unwrap();
    writeln!(out, "    name: String,").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}

/// Generate `impl SuperTrait for Bridge` block for super-trait methods.
///
/// Super-trait methods are looked up from the trait type's `super_traits` list
/// and the corresponding TypeDef (which should also be in the API surface).
/// Since we don't have the full API surface here, we generate based on the
/// super-trait methods stored on the trait_type itself (methods with trait_source
/// matching the super-trait name).
fn gen_super_trait_impl(out: &mut String, struct_name: &str, super_type: &TypeDef, core_import: &str) {
    let super_trait_name = &super_type.name;
    writeln!(out, "#[async_trait]").unwrap();
    writeln!(
        out,
        "impl {core_import}::{super_trait_name} for {struct_name} {{"
    )
    .unwrap();

    for method in &super_type.methods {
        gen_method_impl(out, method, core_import, struct_name);
    }

    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}

/// Generate `impl Trait for Bridge` block for the main trait.
fn gen_trait_impl(out: &mut String, struct_name: &str, trait_type: &TypeDef, core_import: &str) {
    let trait_name = &trait_type.name;
    writeln!(out, "#[async_trait]").unwrap();
    writeln!(out, "impl {core_import}::{trait_name} for {struct_name} {{").unwrap();

    for method in &trait_type.methods {
        // Skip methods that belong to a super-trait (already generated above)
        if method.trait_source.is_some() {
            continue;
        }
        gen_method_impl(out, method, core_import, struct_name);
    }

    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}

/// Generate a single method implementation inside a trait impl block.
fn gen_method_impl(out: &mut String, method: &MethodDef, core_import: &str, _struct_name: &str) {
    let method_name = &method.name;

    // Build method signature
    let params_sig = build_method_signature(method, core_import);
    let return_type_str = rust_type_str(&method.return_type, core_import);

    let has_error = method.error_type.is_some();
    let full_return = if has_error {
        format!("Result<{return_type_str}, {core_import}::KreuzbergError>")
    } else {
        return_type_str.clone()
    };

    if method.is_async {
        writeln!(
            out,
            "    async fn {method_name}({params_sig}) -> {full_return} {{"
        )
        .unwrap();
        gen_async_method_body(out, method, core_import);
    } else {
        writeln!(out, "    fn {method_name}({params_sig}) -> {full_return} {{").unwrap();
        gen_sync_method_body(out, method, core_import);
    }

    writeln!(out, "    }}").unwrap();
    writeln!(out).unwrap();
}

/// Build the method signature parameters string (including &self).
fn build_method_signature(method: &MethodDef, core_import: &str) -> String {
    let mut parts = Vec::new();

    // Receiver
    if method.receiver.is_some() {
        parts.push("&self".to_string());
    }

    // Regular params
    for param in &method.params {
        let ty = rust_type_str(&param.ty, core_import);
        if param.is_ref {
            parts.push(format!("{}: &{ty}", param.name));
        } else {
            parts.push(format!("{}: {ty}", param.name));
        }
    }

    parts.join(", ")
}

/// Generate a sync method body that calls into Python.
fn gen_sync_method_body(out: &mut String, method: &MethodDef, core_import: &str) {
    let method_name = &method.name;
    let has_error = method.error_type.is_some();
    let return_type = &method.return_type;

    writeln!(out, "        Python::attach(|py| {{").unwrap();

    if method.params.is_empty() {
        // No params — use call_method0
        let extract = extract_expression(return_type, core_import);
        if is_unit_return(return_type) {
            writeln!(
                out,
                "            self.python_obj.bind(py)"
            )
            .unwrap();
            writeln!(
                out,
                "                .call_method0(\"{method_name}\")"
            )
            .unwrap();
            if has_error {
                writeln!(
                    out,
                    "                .map(|_| ())"
                )
                .unwrap();
                writeln!(
                    out,
                    "                .map_err(|e| {core_import}::KreuzbergError::Plugin {{"
                )
                .unwrap();
                writeln!(
                    out,
                    "                    message: format!(\"Python plugin '{{}}' method '{method_name}' failed: {{}}\", self.name, e),"
                )
                .unwrap();
                writeln!(
                    out,
                    "                    plugin_name: self.name.clone(),"
                )
                .unwrap();
                writeln!(out, "                }})").unwrap();
            } else {
                writeln!(out, "                .map(|_| ())").unwrap();
                writeln!(
                    out,
                    "                .expect(\"Python plugin method '{method_name}' failed\")"
                )
                .unwrap();
            }
        } else {
            writeln!(
                out,
                "            self.python_obj.bind(py)"
            )
            .unwrap();
            writeln!(
                out,
                "                .call_method0(\"{method_name}\")"
            )
            .unwrap();
            writeln!(
                out,
                "                .and_then(|v| v.extract::<{extract}>())"
            )
            .unwrap();
            if has_error {
                writeln!(
                    out,
                    "                .map_err(|e| {core_import}::KreuzbergError::Plugin {{"
                )
                .unwrap();
                writeln!(
                    out,
                    "                    message: format!(\"Python plugin '{{}}' method '{method_name}' failed: {{}}\", self.name, e),"
                )
                .unwrap();
                writeln!(
                    out,
                    "                    plugin_name: self.name.clone(),"
                )
                .unwrap();
                writeln!(out, "                }})").unwrap();
            } else {
                writeln!(
                    out,
                    "                .expect(\"Python plugin method '{method_name}' failed\")"
                )
                .unwrap();
            }
        }
    } else {
        // Has params — use call_method1
        let py_args = method
            .params
            .iter()
            .map(python_convert_param)
            .collect::<Vec<_>>()
            .join(", ");
        let py_tuple = if method.params.len() == 1 {
            format!("({py_args},)")
        } else {
            format!("({py_args})")
        };

        let extract = extract_expression(return_type, core_import);
        if is_unit_return(return_type) {
            writeln!(
                out,
                "            self.python_obj.bind(py)"
            )
            .unwrap();
            writeln!(
                out,
                "                .call_method1(\"{method_name}\", {py_tuple})"
            )
            .unwrap();
            if has_error {
                writeln!(
                    out,
                    "                .map(|_| ())"
                )
                .unwrap();
                writeln!(
                    out,
                    "                .map_err(|e| {core_import}::KreuzbergError::Plugin {{"
                )
                .unwrap();
                writeln!(
                    out,
                    "                    message: format!(\"Python plugin '{{}}' method '{method_name}' failed: {{}}\", self.name, e),"
                )
                .unwrap();
                writeln!(
                    out,
                    "                    plugin_name: self.name.clone(),"
                )
                .unwrap();
                writeln!(out, "                }})").unwrap();
            } else {
                writeln!(out, "                .map(|_| ())").unwrap();
                writeln!(
                    out,
                    "                .expect(\"Python plugin method '{method_name}' failed\")"
                )
                .unwrap();
            }
        } else {
            writeln!(
                out,
                "            self.python_obj.bind(py)"
            )
            .unwrap();
            writeln!(
                out,
                "                .call_method1(\"{method_name}\", {py_tuple})"
            )
            .unwrap();
            writeln!(
                out,
                "                .and_then(|v| v.extract::<{extract}>())"
            )
            .unwrap();
            if has_error {
                writeln!(
                    out,
                    "                .map_err(|e| {core_import}::KreuzbergError::Plugin {{"
                )
                .unwrap();
                writeln!(
                    out,
                    "                    message: format!(\"Python plugin '{{}}' method '{method_name}' failed: {{}}\", self.name, e),"
                )
                .unwrap();
                writeln!(
                    out,
                    "                    plugin_name: self.name.clone(),"
                )
                .unwrap();
                writeln!(out, "                }})").unwrap();
            } else {
                writeln!(
                    out,
                    "                .expect(\"Python plugin method '{method_name}' failed\")"
                )
                .unwrap();
            }
        }
    }

    writeln!(out, "        }})").unwrap();
}

/// Generate an async method body that spawns a blocking task to call Python.
fn gen_async_method_body(out: &mut String, method: &MethodDef, core_import: &str) {
    let method_name = &method.name;
    let has_error = method.error_type.is_some();

    // Clone python_obj and name for the blocking closure
    writeln!(
        out,
        "        let python_obj = Python::attach(|py| self.python_obj.clone_ref(py));"
    )
    .unwrap();
    writeln!(out, "        let cached_name = self.name.clone();").unwrap();

    // Clone any params that need to cross the thread boundary
    for param in &method.params {
        writeln!(out, "        let {name} = {name}.clone();", name = param.name).unwrap();
    }

    writeln!(out).unwrap();
    writeln!(out, "        tokio::task::spawn_blocking(move || {{").unwrap();
    writeln!(out, "            Python::attach(|py| {{").unwrap();
    writeln!(out, "                let obj = python_obj.bind(py);").unwrap();

    let return_type = &method.return_type;

    if method.params.is_empty() {
        if is_unit_return(return_type) {
            writeln!(
                out,
                "                obj.call_method0(\"{method_name}\")"
            )
            .unwrap();
            writeln!(
                out,
                "                    .map(|_| ())"
            )
            .unwrap();
            writeln!(
                out,
                "                    .map_err(|e| {core_import}::KreuzbergError::Plugin {{"
            )
            .unwrap();
            writeln!(
                out,
                "                        message: format!(\"Python plugin '{{}}' method '{method_name}' failed: {{}}\", cached_name, e),"
            )
            .unwrap();
            writeln!(
                out,
                "                        plugin_name: cached_name.clone(),"
            )
            .unwrap();
            writeln!(out, "                    }})").unwrap();
        } else {
            let extract = extract_expression(return_type, core_import);
            writeln!(
                out,
                "                let result = obj.call_method0(\"{method_name}\")"
            )
            .unwrap();
            writeln!(
                out,
                "                    .and_then(|v| v.extract::<{extract}>())"
            )
            .unwrap();
            writeln!(
                out,
                "                    .map_err(|e| {core_import}::KreuzbergError::Plugin {{"
            )
            .unwrap();
            writeln!(
                out,
                "                        message: format!(\"Python plugin '{{}}' method '{method_name}' failed: {{}}\", cached_name, e),"
            )
            .unwrap();
            writeln!(
                out,
                "                        plugin_name: cached_name.clone(),"
            )
            .unwrap();
            writeln!(out, "                    }})?;").unwrap();
            writeln!(out, "                Ok(result)").unwrap();
        }
    } else {
        let py_args = method
            .params
            .iter()
            .map(python_convert_param)
            .collect::<Vec<_>>()
            .join(", ");
        let py_tuple = if method.params.len() == 1 {
            format!("({py_args},)")
        } else {
            format!("({py_args})")
        };

        if is_unit_return(return_type) {
            writeln!(
                out,
                "                obj.call_method1(\"{method_name}\", {py_tuple})"
            )
            .unwrap();
            writeln!(
                out,
                "                    .map(|_| ())"
            )
            .unwrap();
            writeln!(
                out,
                "                    .map_err(|e| {core_import}::KreuzbergError::Plugin {{"
            )
            .unwrap();
            writeln!(
                out,
                "                        message: format!(\"Python plugin '{{}}' method '{method_name}' failed: {{}}\", cached_name, e),"
            )
            .unwrap();
            writeln!(
                out,
                "                        plugin_name: cached_name.clone(),"
            )
            .unwrap();
            writeln!(out, "                    }})").unwrap();
        } else {
            let extract = extract_expression(return_type, core_import);
            writeln!(
                out,
                "                let result = obj.call_method1(\"{method_name}\", {py_tuple})"
            )
            .unwrap();
            writeln!(
                out,
                "                    .and_then(|v| v.extract::<{extract}>())"
            )
            .unwrap();
            writeln!(
                out,
                "                    .map_err(|e| {core_import}::KreuzbergError::Plugin {{"
            )
            .unwrap();
            writeln!(
                out,
                "                        message: format!(\"Python plugin '{{}}' method '{method_name}' failed: {{}}\", cached_name, e),"
            )
            .unwrap();
            writeln!(
                out,
                "                        plugin_name: cached_name.clone(),"
            )
            .unwrap();
            writeln!(out, "                    }})?;").unwrap();
            writeln!(out, "                Ok(result)").unwrap();
        }
    }

    writeln!(out, "            }})").unwrap();
    writeln!(out, "        }})").unwrap();
    writeln!(out, "        .await").unwrap();

    if has_error {
        writeln!(
            out,
            "        .map_err(|e| {core_import}::KreuzbergError::Plugin {{"
        )
        .unwrap();
        writeln!(
            out,
            "            message: format!(\"Failed to spawn blocking task: {{}}\", e),"
        )
        .unwrap();
        writeln!(out, "            plugin_name: self.name.clone(),").unwrap();
        writeln!(out, "        }})?").unwrap();
    } else {
        writeln!(
            out,
            "        .expect(\"Failed to spawn blocking task\")"
        )
        .unwrap();
    }
}

/// Generate the `#[pyfunction]` registration function.
fn gen_registration_fn(
    out: &mut String,
    trait_type: &TypeDef,
    bridge_cfg: &TraitBridgeConfig,
    struct_name: &str,
    core_import: &str,
) {
    let register_fn = &bridge_cfg.register_fn;
    let trait_name = &bridge_cfg.trait_name;
    let registry_getter = &bridge_cfg.registry_getter;

    // Collect required methods (those without default impls)
    let required_methods: Vec<&str> = trait_type
        .methods
        .iter()
        .filter(|m| !m.has_default_impl && m.trait_source.is_none())
        .map(|m| m.name.as_str())
        .collect();

    // Also include super-trait required methods
    let super_required: Vec<&str> = trait_type
        .methods
        .iter()
        .filter(|m| !m.has_default_impl && m.trait_source.is_some())
        .map(|m| m.name.as_str())
        .collect();

    let all_required: Vec<&str> = super_required
        .iter()
        .chain(required_methods.iter())
        .copied()
        .collect();

    writeln!(out, "#[pyfunction]").unwrap();
    writeln!(
        out,
        "pub fn {register_fn}(py: Python<'_>, backend: Py<PyAny>) -> PyResult<()> {{"
    )
    .unwrap();

    // Validate required methods
    writeln!(out, "    let obj = backend.bind(py);").unwrap();
    writeln!(out, "    let mut missing = Vec::new();").unwrap();
    for method_name in &all_required {
        writeln!(
            out,
            "    if !obj.hasattr(\"{method_name}\")? {{ missing.push(\"{method_name}\"); }}"
        )
        .unwrap();
    }
    writeln!(out, "    if !missing.is_empty() {{").unwrap();
    writeln!(
        out,
        "        return Err(pyo3::exceptions::PyAttributeError::new_err("
    )
    .unwrap();
    writeln!(
        out,
        "            format!(\"Python {trait_name} plugin missing required methods: {{:?}}\", missing)"
    )
    .unwrap();
    writeln!(out, "        ));").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out).unwrap();

    // Cache name
    writeln!(
        out,
        "    let name: String = obj.call_method0(\"name\")?.extract()?;"
    )
    .unwrap();
    writeln!(out).unwrap();

    // Construct wrapper
    writeln!(
        out,
        "    let wrapper = {struct_name} {{ python_obj: backend, name }};"
    )
    .unwrap();
    writeln!(
        out,
        "    let arc: Arc<dyn {core_import}::{trait_name}> = Arc::new(wrapper);"
    )
    .unwrap();
    writeln!(out).unwrap();

    // Register
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
    writeln!(out).unwrap();
    writeln!(out, "    Ok(())").unwrap();
    writeln!(out, "}}").unwrap();
}

/// Find a super-trait TypeDef by checking methods with matching trait_source.
/// Returns a synthetic TypeDef containing only the super-trait methods.
fn find_super_trait_type(trait_type: &TypeDef, super_trait_name: &str) -> Option<TypeDef> {
    let super_methods: Vec<MethodDef> = trait_type
        .methods
        .iter()
        .filter(|m| {
            m.trait_source
                .as_ref()
                .is_some_and(|s| s.ends_with(super_trait_name))
        })
        .cloned()
        .collect();

    if super_methods.is_empty() {
        return None;
    }

    Some(TypeDef {
        name: super_trait_name.to_string(),
        rust_path: String::new(),
        fields: vec![],
        methods: super_methods,
        is_opaque: false,
        is_clone: false,
        doc: String::new(),
        cfg: None,
        is_trait: true,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
    })
}

/// Map an IR TypeRef to a Rust type string for use in generated code.
fn rust_type_str(ty: &TypeRef, core_import: &str) -> String {
    match ty {
        TypeRef::Primitive(p) => match p {
            alef_core::ir::PrimitiveType::Bool => "bool".to_string(),
            alef_core::ir::PrimitiveType::U8 => "u8".to_string(),
            alef_core::ir::PrimitiveType::U16 => "u16".to_string(),
            alef_core::ir::PrimitiveType::U32 => "u32".to_string(),
            alef_core::ir::PrimitiveType::U64 => "u64".to_string(),
            alef_core::ir::PrimitiveType::I8 => "i8".to_string(),
            alef_core::ir::PrimitiveType::I16 => "i16".to_string(),
            alef_core::ir::PrimitiveType::I32 => "i32".to_string(),
            alef_core::ir::PrimitiveType::I64 => "i64".to_string(),
            alef_core::ir::PrimitiveType::F32 => "f32".to_string(),
            alef_core::ir::PrimitiveType::F64 => "f64".to_string(),
            alef_core::ir::PrimitiveType::Usize => "usize".to_string(),
            alef_core::ir::PrimitiveType::Isize => "isize".to_string(),
        },
        TypeRef::String => "String".to_string(),
        TypeRef::Char => "char".to_string(),
        TypeRef::Bytes => "Vec<u8>".to_string(),
        TypeRef::Optional(inner) => format!("Option<{}>", rust_type_str(inner, core_import)),
        TypeRef::Vec(inner) => format!("Vec<{}>", rust_type_str(inner, core_import)),
        TypeRef::Map(k, v) => format!(
            "std::collections::HashMap<{}, {}>",
            rust_type_str(k, core_import),
            rust_type_str(v, core_import)
        ),
        TypeRef::Named(name) => format!("{core_import}::{name}"),
        TypeRef::Path => "String".to_string(),
        TypeRef::Unit => "()".to_string(),
        TypeRef::Json => "serde_json::Value".to_string(),
        TypeRef::Duration => "std::time::Duration".to_string(),
    }
}

/// Get the Rust extraction type for PyO3's `.extract::<T>()`.
fn extract_expression(ty: &TypeRef, _core_import: &str) -> String {
    match ty {
        TypeRef::Primitive(p) => match p {
            alef_core::ir::PrimitiveType::Bool => "bool".to_string(),
            alef_core::ir::PrimitiveType::U8 => "u8".to_string(),
            alef_core::ir::PrimitiveType::U16 => "u16".to_string(),
            alef_core::ir::PrimitiveType::U32 => "u32".to_string(),
            alef_core::ir::PrimitiveType::U64 => "u64".to_string(),
            alef_core::ir::PrimitiveType::I8 => "i8".to_string(),
            alef_core::ir::PrimitiveType::I16 => "i16".to_string(),
            alef_core::ir::PrimitiveType::I32 => "i32".to_string(),
            alef_core::ir::PrimitiveType::I64 => "i64".to_string(),
            alef_core::ir::PrimitiveType::F32 => "f32".to_string(),
            alef_core::ir::PrimitiveType::F64 => "f64".to_string(),
            alef_core::ir::PrimitiveType::Usize => "usize".to_string(),
            alef_core::ir::PrimitiveType::Isize => "isize".to_string(),
        },
        TypeRef::String | TypeRef::Path | TypeRef::Char => "String".to_string(),
        TypeRef::Bytes => "Vec<u8>".to_string(),
        TypeRef::Vec(inner) => format!("Vec<{}>", extract_expression(inner, _core_import)),
        TypeRef::Optional(inner) => {
            format!("Option<{}>", extract_expression(inner, _core_import))
        }
        TypeRef::Map(k, v) => format!(
            "std::collections::HashMap<{}, {}>",
            extract_expression(k, _core_import),
            extract_expression(v, _core_import)
        ),
        // Named types: extract as the binding type string
        TypeRef::Named(name) => name.clone(),
        TypeRef::Unit => "()".to_string(),
        TypeRef::Json => "String".to_string(),
        TypeRef::Duration => "u64".to_string(),
    }
}

/// Convert a parameter for passing to a Python method call.
/// Most types pass through directly; references need cloning.
fn python_convert_param(param: &alef_core::ir::ParamDef) -> String {
    if param.is_ref {
        format!("{}.clone()", param.name)
    } else {
        param.name.clone()
    }
}

/// Check if a return type is unit (void).
fn is_unit_return(ty: &TypeRef) -> bool {
    matches!(ty, TypeRef::Unit)
}

/// Collect all trait bridge registration function names for module init.
pub fn collect_bridge_register_fns(configs: &[TraitBridgeConfig]) -> Vec<String> {
    configs.iter().map(|c| c.register_fn.clone()).collect()
}

/// Check if any trait bridges are configured and add required imports.
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
