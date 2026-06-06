//! PyO3-specific trait bridge code generation.
//!
//! Generates Rust wrapper structs that implement Rust traits by delegating
//! to Python objects via PyO3.

pub use crate::codegen::generators::trait_bridge::find_bridge_param;
use crate::codegen::generators::trait_bridge::{
    BridgeOutput, TraitBridgeGenerator, TraitBridgeSpec, bridge_param_type as param_type, gen_bridge_all,
    host_function_path, visitor_param_type,
};
use crate::core::config::TraitBridgeConfig;
use crate::core::ir::{ApiSurface, MethodDef, TypeDef, TypeRef};
use std::collections::{HashMap, HashSet};

/// Compute the Python-visible symbol name for a generated `#[pyfunction]`.
///
/// We prefix the Rust function with `_alef_` to avoid colliding with the
/// host crate's own re-exports of `register_*`/`unregister_*` symbols when
/// the binding crate `use`s them via `*`. The Python-side name (set with
/// `#[pyo3(name = "...")]`) keeps the bare function name so callers see
/// `module.unregister_text_backend(name)`.
fn exported_pyfunction_symbol(fn_name: &str) -> String {
    fn_name.to_string()
}

/// PyO3-specific trait bridge generator.
/// Implements code generation for bridging Python objects to Rust traits.
pub struct Pyo3BridgeGenerator {
    /// Core crate import path (e.g., `"sample_core"`).
    pub core_import: String,
    /// Map of type name → fully-qualified Rust path for type references.
    pub type_paths: HashMap<String, String>,
    /// Error type name (e.g., `"SampleCrateError"`).
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

        let py_args = self.sync_py_args(method);
        let call = if py_args.is_empty() {
            format!("self.inner.bind(py).call_method0(\"{name}\")")
        } else {
            format!("self.inner.bind(py).call_method1(\"{name}\", ({py_args}))")
        };
        let error_expr = spec.make_error(&format!(
            "format!(\"Plugin '{{}}' method '{name}' failed: {{}}\", self.cached_name, e)"
        ));

        if matches!(method.return_type, TypeRef::Unit) {
            crate::backends::pyo3::template_env::render(
                "trait_bridge/sync_method_unit_return.jinja",
                minijinja::context! {
                    method_name => name,
                    call => call,
                    has_error => has_error,
                    error_expr => error_expr,
                },
            )
        } else {
            let ext = self.extract_ty(&method.return_type);
            let is_named = matches!(method.return_type, TypeRef::Named(_));
            crate::backends::pyo3::template_env::render(
                "trait_bridge/sync_method_non_unit_return.jinja",
                minijinja::context! {
                    method_name => name,
                    call => call,
                    is_named => is_named,
                    extract_ty => ext,
                    has_error => has_error,
                    error_expr => error_expr,
                },
            )
        }
    }

    fn gen_async_method_body(&self, method: &MethodDef, spec: &TraitBridgeSpec) -> String {
        let name = &method.name;

        // Build param cloning code using template
        let params: Vec<minijinja::Value> = method
            .params
            .iter()
            .map(|p| {
                minijinja::context! {
                    name => &p.name,
                    ty => match &p.ty {
                        TypeRef::Bytes => "Bytes",
                        TypeRef::Path => "Path",
                        TypeRef::Named(_) => {

                            match &p.ty {
                                TypeRef::Named(n) => n.as_str(),
                                _ => "",
                            }
                        },
                        _ => "",
                    }.to_string(),
                    ty_is_named => matches!(&p.ty, TypeRef::Named(_)),
                    is_ref => p.is_ref,
                }
            })
            .collect();

        let param_cloning = crate::backends::pyo3::template_env::render(
            "trait_bridge/async_param_cloning.jinja",
            minijinja::context! {
                params => params,
            },
        );

        let py_args = self.async_py_args(method);
        let call = if py_args.is_empty() {
            format!("obj.call_method0(\"{name}\")")
        } else {
            format!("obj.call_method1(\"{name}\", ({py_args}))")
        };
        let error_expr = spec.make_error(&format!(
            "format!(\"Plugin '{{}}' method '{name}' failed: {{}}\", cached_name, e)"
        ));
        let json_error_expr =
            spec.make_error("format!(\"Plugin '{}': JSON serialization failed: {}\", cached_name, e)");
        let deserialize_error_expr =
            spec.make_error("format!(\"Plugin '{}': deserialization failed: {}\", cached_name, e)");
        let spawn_error_expr = spec.make_error("format!(\"spawn_blocking failed: {}\", e)");

        if self.is_named(&method.return_type) {
            let return_type =
                crate::codegen::generators::trait_bridge::format_type_ref(&method.return_type, &spec.type_paths);
            crate::backends::pyo3::template_env::render(
                "trait_bridge/async_method_named_return.jinja",
                minijinja::context! {
                    method_name => name,
                    call => call,
                    param_cloning => param_cloning,
                    return_type => return_type,
                    error_expr => error_expr,
                    json_error_expr => json_error_expr,
                    deserialize_error_expr => deserialize_error_expr,
                    spawn_error_expr => spawn_error_expr,
                },
            )
        } else if matches!(method.return_type, TypeRef::Unit) {
            crate::backends::pyo3::template_env::render(
                "trait_bridge/async_method_unit_return.jinja",
                minijinja::context! {
                    method_name => name,
                    call => call,
                    param_cloning => param_cloning,
                    error_expr => error_expr,
                    spawn_error_expr => spawn_error_expr,
                },
            )
        } else {
            let ext = self.extract_ty(&method.return_type);
            crate::backends::pyo3::template_env::render(
                "trait_bridge/async_method_non_unit_return.jinja",
                minijinja::context! {
                    method_name => name,
                    call => call,
                    extract_ty => ext,
                    param_cloning => param_cloning,
                    error_expr => error_expr,
                    spawn_error_expr => spawn_error_expr,
                },
            )
        }
    }

    fn gen_constructor(&self, spec: &TraitBridgeSpec) -> String {
        let wrapper = spec.wrapper_name();
        let required_methods = spec.required_methods();
        crate::backends::pyo3::template_env::render(
            "trait_bridge/constructor.jinja",
            minijinja::context! {
                wrapper => wrapper,
                required_methods => required_methods,
            },
        )
    }

    fn gen_unregistration_fn(&self, spec: &TraitBridgeSpec) -> String {
        let Some(unregister_fn) = spec.bridge_config.unregister_fn.as_deref() else {
            return String::new();
        };
        // Derive the FQN of the host crate's `unregister_*` function from the
        // bridge's `registry_getter` path: `sample_core::plugins::registry::get_*`
        // → `sample_core::plugins::*::unregister_*`. When `registry_getter` is not
        // set we fall back to `{core}::plugins::{unregister_fn}` and trust the
        // caller's wiring.
        let host_path = host_function_path(spec, unregister_fn);
        let host_symbol = exported_pyfunction_symbol(unregister_fn);
        crate::backends::pyo3::template_env::render(
            "trait_bridge/unregistration_fn.jinja",
            minijinja::context! {
                unregister_fn => unregister_fn,
                host_symbol => host_symbol,
                host_path => host_path,
            },
        )
    }

    fn gen_clear_fn(&self, spec: &TraitBridgeSpec) -> String {
        let Some(clear_fn) = spec.bridge_config.clear_fn.as_deref() else {
            return String::new();
        };
        let host_path = host_function_path(spec, clear_fn);
        let host_symbol = exported_pyfunction_symbol(clear_fn);
        crate::backends::pyo3::template_env::render(
            "trait_bridge/clear_fn.jinja",
            minijinja::context! {
                clear_fn => clear_fn,
                host_symbol => host_symbol,
                host_path => host_path,
            },
        )
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

        let req_methods: Vec<&MethodDef> = spec.required_methods();
        let required_methods_str = req_methods
            .iter()
            .map(|m| format!("\"{}\"", m.name))
            .collect::<Vec<_>>()
            .join(", ");

        let register_extra_args = spec
            .bridge_config
            .register_extra_args
            .as_deref()
            .map(|a| format!(", {a}"))
            .unwrap_or_default();

        crate::backends::pyo3::template_env::render(
            "trait_bridge/registration_fn.jinja",
            minijinja::context! {
                register_fn => register_fn,
                wrapper => wrapper,
                trait_path => trait_path,
                registry_getter => registry_getter,
                register_extra_args => register_extra_args,
                has_required_methods => !req_methods.is_empty(),
                required_methods_str => required_methods_str,
            },
        )
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
    fn prim(&self, p: &crate::core::ir::PrimitiveType) -> &'static str {
        use crate::core::ir::PrimitiveType::*;
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
}

/// Generate all trait bridge code for a given trait type and bridge config.
pub fn gen_trait_bridge(
    trait_type: &TypeDef,
    bridge_cfg: &TraitBridgeConfig,
    core_import: &str,
    error_type: &str,
    error_constructor: &str,
    api: &ApiSurface,
) -> anyhow::Result<BridgeOutput> {
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
        // Include excluded types so trait methods referencing them (for example, `&HiddenDoc`)
        // are qualified with the full Rust path rather than emitting the bare type name.
        .chain(
            api.excluded_type_paths
                .iter()
                .map(|(name, path)| (name.clone(), path.replace('-', "_"))),
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
        let struct_name = crate::codegen::generators::trait_bridge::bridge_wrapper_name("Py", bridge_cfg);
        let code = gen_visitor_bridge(
            trait_type,
            bridge_cfg,
            &struct_name,
            &trait_path,
            core_import,
            &type_paths,
            api,
        )?;
        Ok(BridgeOutput { imports: vec![], code })
    } else {
        // Use the IR-driven TraitBridgeGenerator infrastructure
        let generator = Pyo3BridgeGenerator {
            core_import: core_import.to_string(),
            type_paths: type_paths.clone(),
            error_type: error_type.to_string(),
        };
        let lifetime_type_names: HashSet<String> = api
            .types
            .iter()
            .filter(|t| t.has_lifetime_params)
            .map(|t| t.name.clone())
            .collect();
        let spec = TraitBridgeSpec {
            trait_def: trait_type,
            bridge_config: bridge_cfg,
            core_import,
            wrapper_prefix: "Py",
            type_paths,
            lifetime_type_names,
            error_type: error_type.to_string(),
            error_constructor: error_constructor.to_string(),
        };
        Ok(gen_bridge_all(&spec, &generator))
    }
}

/// Generate a visitor-style bridge: thin wrapper over `Py<PyAny>` where every trait method
/// tries to call the corresponding Python method, falling back to the default if absent.
///
/// This pattern is used for traits where:
/// - All methods have default implementations
/// - No registration function is needed (per-call construction via `type_alias`)
/// - No super-trait forwarding
fn gen_visitor_bridge(
    trait_type: &TypeDef,
    bridge_cfg: &TraitBridgeConfig,
    struct_name: &str,
    trait_path: &str,
    core_crate: &str,
    type_paths: &HashMap<String, String>,
    api: &ApiSurface,
) -> anyhow::Result<String> {
    let result_metadata = crate::codegen::visitor_result::required_visitor_result_metadata(api, bridge_cfg)?;
    let context_helper = crate::codegen::visitor_context::visitor_context_helper(
        api,
        bridge_cfg,
        core_crate,
        crate::codegen::visitor_context::VisitorContextBackend::Pyo3,
    )?;

    // Emit a helper function for converting the configured visitor context to a Python dict.
    let helper_fn = crate::backends::pyo3::template_env::render(
        "trait_bridge/nodecontext_to_py_dict.jinja",
        minijinja::context! {
            context_type_path => context_helper.type_path,
            context_field_lines => context_helper.field_lines,
        },
    );

    // Struct with only the Python object — no cached fields needed
    let struct_def = crate::backends::pyo3::template_env::render(
        "trait_bridge/visitor_struct.jinja",
        minijinja::context! {
            struct_name => struct_name,
        },
    );

    // Trait impl — collect all methods
    let mut methods_code = String::new();
    for method in crate::codegen::generators::trait_bridge::visitor_callback_methods(trait_type, bridge_cfg) {
        gen_visitor_method(
            &mut methods_code,
            method,
            trait_path,
            bridge_cfg,
            type_paths,
            &result_metadata,
        );
    }

    let mut out = String::with_capacity(4096);
    out.push_str(&helper_fn);
    out.push_str(&struct_def);
    out.push_str(&crate::backends::pyo3::template_env::render(
        "trait_bridge/impl_header.jinja",
        minijinja::context! { trait_path => trait_path, struct_name => struct_name },
    ));
    out.push_str(&methods_code);
    out.push_str("}\n");
    Ok(out)
}

/// Generate a single visitor-style trait method that tries Python dispatch, falls back to default.
///
/// For each method the generated code:
/// 1. Checks if the Python object has an attribute with this method's name.
/// 2. If yes, calls the method with converted arguments and converts the Python return value
///    to the appropriate Rust return type.
/// 3. If no (attribute absent), returns the configured default result variant.
fn gen_visitor_method(
    out: &mut String,
    method: &MethodDef,
    _trait_path: &str,
    bridge_cfg: &TraitBridgeConfig,
    type_paths: &HashMap<String, String>,
    result_metadata: &crate::codegen::visitor_result::VisitorResultMetadata,
) {
    use crate::core::ir::TypeRef;

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
    // Visitor-style methods may return a named type from the core crate.
    // Use the fully-qualified path from type_paths when available.
    let ret_ty = match &method.return_type {
        TypeRef::Named(n) => type_paths.get(n).cloned().unwrap_or_else(|| n.clone()),
        other => param_type(other, "", false, type_paths),
    };

    // Build argument expressions for the Python call
    let py_args = build_visitor_py_args(method, bridge_cfg);

    let py_call = if py_args.is_empty() {
        format!("obj.call_method0(\"{name}\")")
    } else {
        format!("obj.call_method1(\"{name}\", ({py_args}))")
    };

    let method_code = crate::backends::pyo3::template_env::render(
        "trait_bridge/visitor_method.jinja",
        minijinja::context! {
            method_name => name,
            sig => sig,
            ret_ty => ret_ty,
            default_result_expr => crate::codegen::visitor_result::default_result_expr(&ret_ty, result_metadata),
            unknown_string_result_expr => crate::codegen::visitor_result::unknown_string_result_expr(
                &ret_ty,
                result_metadata,
                "s",
            ),
            unit_result_variants => crate::codegen::visitor_result::variant_contexts(&result_metadata.unit_variants),
            payload_result_variants => crate::codegen::visitor_result::variant_contexts(
                &result_metadata.string_payload_variants,
            ),
            py_call => py_call,
        },
    );

    out.push_str(&method_code);
}

/// Build Python call argument expressions for a visitor method.
///
/// - configured context params: converted to a Python dict via `nodecontext_to_py_dict`
/// - `&str` params: passed directly (PyO3 handles `&str` → Python str coercion)
/// - `Option<&str>` params: passed as `Option<&str>` (PyO3 maps `None` → Python `None`)
/// - `bool` and integer params: passed directly
/// - `&[String]` / `Vec<String>` params: passed as Python lists
fn build_visitor_py_args(method: &MethodDef, bridge_cfg: &TraitBridgeConfig) -> String {
    use crate::core::ir::TypeRef;
    let args: Vec<String> = method
        .params
        .iter()
        .map(|p| {
            // context_type param: convert to Python dict
            if let TypeRef::Named(n) = &p.ty {
                if Some(n.as_str()) == bridge_cfg.context_type.as_deref() {
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

/// Collect unregistration function names for api.py pass-through wrappers.
///
/// Only bridges that define an `unregister_fn` are included.
pub fn collect_bridge_unregister_fns(configs: &[TraitBridgeConfig]) -> Vec<String> {
    configs.iter().filter_map(|c| c.unregister_fn.clone()).collect()
}

/// Collect clear function names for api.py pass-through wrappers.
///
/// Only bridges that define a `clear_fn` are included.
pub fn collect_bridge_clear_fns(configs: &[TraitBridgeConfig]) -> Vec<String> {
    configs.iter().filter_map(|c| c.clear_fn.clone()).collect()
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
///     let bridge = Py{TraitName}Bridge::new(v);
///     std::sync::Arc::new(std::sync::Mutex::new(bridge)) as core_crate::callbacks::{ConfiguredHandle}
/// });
/// ```
#[allow(clippy::too_many_arguments)]
pub fn gen_bridge_function(
    api: &ApiSurface,
    func: &crate::core::ir::FunctionDef,
    bridge_param_idx: usize,
    bridge_cfg: &TraitBridgeConfig,
    mapper: &dyn crate::codegen::type_mapper::TypeMapper,
    cfg: &crate::codegen::generators::RustBindingConfig<'_>,
    adapter_bodies: &crate::codegen::generators::AdapterBodies,
    opaque_types: &ahash::AHashSet<String>,
    core_import: &str,
    error_converters: &[String],
) -> String {
    use crate::codegen::generators::AsyncPattern;
    use crate::core::ir::TypeRef;

    let struct_name = crate::codegen::generators::trait_bridge::bridge_wrapper_name("Py", bridge_cfg);
    let handle_path = crate::codegen::generators::trait_bridge::bridge_handle_path(api, bridge_cfg, core_import);

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
             std::sync::Arc::new(std::sync::Mutex::new(bridge)) as {handle_path}\n    \
             }});"
        )
    } else {
        format!(
            "let {param_name} = {{\n        \
             let bridge = {struct_name}::new({param_name});\n        \
             std::sync::Arc::new(std::sync::Mutex::new(bridge)) as {handle_path}\n    \
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
    // (since ParseOptions has serde, and has_named_params will be true)
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
        // (anyhow::Error, etc.), fall back to PyRuntimeError — unless there is exactly one
        // known converter available, in which case use it (handles the `anyhow::Result<T>`
        // alias case where the IR records "anyhow::Error" as the error type).
        let core_err_conv = if error_type.contains("::") || error_type == "Error" {
            if error_converters.len() == 1 {
                // Single known converter — use it instead of the generic PyRuntimeError fallback.
                format!(".map_err({})", error_converters[0])
            } else {
                // Generic error type — use PyRuntimeError
                ".map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))".to_string()
            }
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

    let mut sig_str = String::new();
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
        sig_str = sig_parts.join(", ");
    }

    let func_name = &func.name;

    // Suppress unused adapter_bodies warning
    let _ = adapter_bodies;

    crate::backends::pyo3::template_env::render(
        "trait_bridge/function_wrapper.jinja",
        minijinja::context! {
            has_error => func.error_type.is_some(),
            attr_inner => attr_inner,
            needs_signature => cfg.needs_signature,
            signature_prefix => cfg.signature_prefix,
            sig_str => sig_str,
            signature_suffix => cfg.signature_suffix,
            func_name => func_name,
            lifetime => lifetime,
            params_str => params_str,
            ret => ret,
            body => body,
        },
    )
}

/// Generate a PyO3 function wrapper for a bridge whose handle lives as a field
/// on an options struct (`bind_via = "options_field"`).
///
/// The generated function adds an extra `visitor: Option<Py<PyAny>>` parameter.
/// When the caller supplies `visitor`, it is wrapped in `Py{Trait}Bridge`, boxed
/// into the configured core handle, and injected onto a copy of the options struct
/// before the core function is called.  When `visitor` is `None` the options
/// struct is forwarded unchanged.
///
/// Error dispatch: uses the dedicated `{snake_error}_to_py_err` converter when
/// the IR error type is a known PascalCase name; falls back to `PyRuntimeError`
/// for generic/path-qualified types.
#[allow(clippy::too_many_arguments)]
pub fn gen_bridge_field_function(
    api: &ApiSurface,
    func: &crate::core::ir::FunctionDef,
    bridge_match: &crate::codegen::generators::trait_bridge::BridgeFieldMatch<'_>,
    bridge_cfg: &TraitBridgeConfig,
    mapper: &dyn crate::codegen::type_mapper::TypeMapper,
    cfg: &crate::codegen::generators::RustBindingConfig<'_>,
    opaque_types: &ahash::AHashSet<String>,
    core_import: &str,
    error_converters: &[String],
) -> String {
    use crate::codegen::generators::AsyncPattern;
    use crate::core::ir::TypeRef;

    let struct_name = crate::codegen::generators::trait_bridge::bridge_wrapper_name("Py", bridge_cfg);
    let handle_path = crate::codegen::generators::trait_bridge::bridge_handle_path(api, bridge_cfg, core_import);

    // Name of the visitor kwarg that will be appended to the Rust function signature.
    let visitor_kwarg = bridge_cfg.param_name.as_deref().unwrap_or("visitor");
    // Name of the options parameter.
    let options_param = &bridge_match.param_name;
    // Rust type of the options parameter.
    let options_type = &bridge_match.options_type;
    // The field on the options struct that holds the bridge handle.
    let field_name = &bridge_match.field_name;
    let param_is_optional = bridge_match.param_is_optional;

    let func_needs_py = func.is_async && cfg.async_pattern == AsyncPattern::Pyo3FutureIntoPy;
    let lifetime = if func_needs_py { "<'py>" } else { "" };

    // Build parameter list: same as gen_function but append the extra visitor kwarg.
    let mut sig_parts = Vec::new();
    if func_needs_py {
        sig_parts.push("py: Python<'py>".to_string());
    }
    for p in func.params.iter() {
        let ty = if p.optional || matches!(&p.ty, TypeRef::Optional(_)) {
            format!("Option<{}>", mapper.map_type(&p.ty))
        } else {
            mapper.map_type(&p.ty)
        };
        sig_parts.push(format!("{}: {}", p.name, ty));
    }
    // Extra visitor kwarg — always optional
    sig_parts.push(format!("{visitor_kwarg}: Option<Py<PyAny>>"));

    let params_str = sig_parts.join(", ");
    let return_type = mapper.map_type(&func.return_type);
    let ret = mapper.wrap_return(&return_type, func.error_type.is_some());
    let ret = if func_needs_py {
        "PyResult<Bound<'py, PyAny>>".to_string()
    } else {
        ret
    };

    // --- Build function body ---

    // 1. Wrap the extra bridge kwarg into the configured handle type.
    let visitor_wrap = format!(
        "let {visitor_kwarg}_handle: Option<{handle_path}> = {visitor_kwarg}.map(|v| {{\n        \
         let bridge = {struct_name}::new(v);\n        \
         std::sync::Arc::new(std::sync::Mutex::new(bridge)) as {handle_path}\n    \
         }});"
    );

    // 2. Build serde-based conversion for non-options Named params.
    let serde_err_conv = ".map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))";
    let serde_bindings: String = func
        .params
        .iter()
        .filter(|p| {
            // Only process Named or Optional<Named> types that are not opaque and not the
            // options param (which is handled separately).
            if p.name == *options_param {
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
        .map(|p| {
            let name = &p.name;
            let core_type_name = match &p.ty {
                TypeRef::Named(n) => n.clone(),
                TypeRef::Optional(inner) => {
                    if let TypeRef::Named(n) = inner.as_ref() {
                        n.clone()
                    } else {
                        String::new()
                    }
                }
                _ => String::new(),
            };
            let core_path = format!("{core_import}::{core_type_name}");
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

    // 3. Build the options-core conversion, injecting the visitor handle.
    //
    // The visitor field is sanitized in the IR (its type collapsed to String), so we
    // cannot use the serde round-trip path for it — we inject it directly.
    //
    // Use direct .into() conversion instead of serde round-trip to properly handle
    // PyO3 enums (which don't serialize via serde) like TierStrategy, PreprocessingPreset.
    //
    // If the options param is `Option<OptionsType>`:
    //   - When both options and visitor are provided: convert options, then
    //     override the field.
    //   - When only visitor is provided: construct a default OptionsType and set the field.
    //   - When neither is provided: pass None.
    //
    // If the options param is `OptionsType` (non-optional): convert and inject.
    let core_options_type = format!("{core_import}::{options_type}");
    let options_core_binding = if param_is_optional {
        format!(
            // 3a. Convert the Python options to core via From impl (visitor field excluded via serde skip).
            "let {options_param}_core: Option<{core_options_type}> = {options_param}.map(|v| v.into());\n    \
             // Inject the visitor handle: upgrade existing options or construct defaults.\n    \
             let {options_param}_core: Option<{core_options_type}> = if let Some(handle) = {visitor_kwarg}_handle {{\n        \
             let mut opts = {options_param}_core.unwrap_or_default();\n        \
             opts.{field_name} = Some(handle);\n        \
             Some(opts)\n    \
             }} else {{\n        \
             {options_param}_core\n    \
             }};"
        )
    } else {
        format!(
            "let mut {options_param}_core: {core_options_type} = match &{options_param} {{\n        \
             Some(opts) => opts.clone().into(),\n        \
             None => {core_options_type}::default(),\n    \
             }};\n    \
             if let Some(handle) = {visitor_kwarg}_handle {{\n        \
             {options_param}_core.{field_name} = Some(handle);\n    \
             }}"
        )
    };

    // 4. Build the core function call args.
    let call_args: Vec<String> = func
        .params
        .iter()
        .map(|p| {
            if p.name == *options_param {
                return format!("{options_param}_core");
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

    // 5. Build return expression.
    let return_wrap = match &func.return_type {
        TypeRef::Named(name) if opaque_types.contains(name.as_str()) => {
            format!("{name} {{ inner: std::sync::Arc::new(val) }}")
        }
        TypeRef::Named(_) => "val.into()".to_string(),
        TypeRef::String | TypeRef::Bytes => "val.into()".to_string(),
        _ => "val".to_string(),
    };

    // 6. Build error conversion.
    let body = if let Some(ref error_type) = func.error_type {
        // Same heuristic as gen_bridge_function: path-qualified types (anyhow::Error) are
        // treated as generic unless there is exactly one known error converter available,
        // in which case that converter is used instead of the PyRuntimeError fallback.
        let core_err_conv = if error_type.contains("::") || error_type == "Error" {
            if error_converters.len() == 1 {
                format!(".map_err({})", error_converters[0])
            } else {
                ".map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))".to_string()
            }
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
            format!("{visitor_wrap}\n    {serde_bindings}{options_core_binding}\n    {core_call}{core_err_conv}")
        } else {
            format!(
                "{visitor_wrap}\n    {serde_bindings}{options_core_binding}\n    {core_call}.map(|val| {return_wrap}){core_err_conv}"
            )
        }
    } else {
        format!("{visitor_wrap}\n    {serde_bindings}{options_core_binding}\n    {core_call}")
    };

    // Build PyO3 attributes.
    let attr_inner = cfg
        .function_attr
        .trim_start_matches('#')
        .trim_start_matches('[')
        .trim_end_matches(']');

    let mut sig_str = String::new();
    if cfg.needs_signature {
        // #[pyo3(signature = (...))] — all params from the IR plus the extra visitor kwarg.
        let mut seen_optional = false;
        let mut sig_items: Vec<String> = func
            .params
            .iter()
            .map(|p| {
                if p.optional {
                    seen_optional = true;
                }
                if p.optional || seen_optional {
                    format!("{}=None", p.name)
                } else {
                    p.name.clone()
                }
            })
            .collect();
        // visitor kwarg is always optional
        sig_items.push(format!("{visitor_kwarg}=None"));
        sig_str = sig_items.join(", ");
    }
    let func_name = &func.name;
    crate::backends::pyo3::template_env::render(
        "trait_bridge/function_wrapper.jinja",
        minijinja::context! {
            has_error => func.error_type.is_some(),
            attr_inner => attr_inner,
            needs_signature => cfg.needs_signature,
            signature_prefix => cfg.signature_prefix,
            sig_str => sig_str,
            signature_suffix => cfg.signature_suffix,
            func_name => func_name,
            lifetime => lifetime,
            params_str => params_str,
            ret => ret,
            body => body,
        },
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn visitor_bridge_uses_configured_context_and_result_metadata() {
        let (api, trait_type, bridge) = crate::codegen::visitor_context::test_support::neutral_visitor_fixture();
        let output = super::gen_trait_bridge(
            &trait_type,
            &bridge,
            "sample_core",
            "SampleError",
            "SampleError::Message { message: {msg} }",
            &api,
        )
        .expect("visitor bridge should generate");

        crate::codegen::visitor_context::test_support::assert_neutral_visitor_output(&output.code);
        assert!(output.code.contains("\"display_name\""));
    }
}
