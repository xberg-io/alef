//! R (extendr) specific trait bridge code generation.
//!
//! Generates Rust wrapper structs that implement Rust traits by delegating
//! to R objects (named lists of functions) via extendr.

use alef_codegen::generators::trait_bridge::{TraitBridgeGenerator, TraitBridgeSpec, gen_bridge_all};
use alef_core::config::TraitBridgeConfig;
use alef_core::ir::{MethodDef, TypeDef, TypeRef};
use std::collections::HashMap;
use std::fmt::Write;

/// Extendr-specific trait bridge generator.
/// Implements code generation for bridging R objects to Rust traits.
pub struct ExtendrBridgeGenerator {
    /// Core crate import path (e.g., `"kreuzberg"`).
    pub core_import: String,
    /// Map of type name → fully-qualified Rust path for type references.
    pub type_paths: HashMap<String, String>,
    error_type: error_type.to_string(),
}

impl TraitBridgeGenerator for ExtendrBridgeGenerator {
    fn foreign_object_type(&self) -> &str {
        "extendr_api::Robj"
    }

    fn bridge_imports(&self) -> Vec<String> {
        vec![
            "use extendr_api::prelude::*;".to_string(),
            "use std::sync::Arc;".to_string(),
        ]
    }

    fn gen_sync_method_body(&self, method: &MethodDef, spec: &TraitBridgeSpec) -> String {
        let name = &method.name;
        let has_error = method.error_type.is_some();
        let mut out = String::with_capacity(512);

        // Check if the R object has this method
        writeln!(out, "let maybe_fn = self.inner.dollar(\"{name}\");").ok();
        writeln!(out, "let fn_robj = match maybe_fn {{").ok();
        writeln!(out, "    Ok(v) if !v.is_null() && !v.is_na() => v,").ok();
        if has_error {
            writeln!(
                out,
                "    _ => return Err({}::KreuzbergError::Plugin {{",
                spec.core_import
            )
            .ok();
            writeln!(
                out,
                "        message: format!(\"Plugin '{{}}' missing method '{}'\", self.cached_name),",
                name
            )
            .ok();
            writeln!(out, "        plugin_name: self.cached_name.clone(),").ok();
            writeln!(out, "    }}),").ok();
        } else {
            writeln!(out, "    _ => return Default::default(),").ok();
        }
        writeln!(out, "}};").ok();

        // Build argument list for the R function call
        if method.params.is_empty() {
            writeln!(out, "let result = fn_robj.call(extendr_api::Pairlist::new());").ok();
        } else {
            let args: Vec<String> = method.params.iter().map(build_extendr_arg).collect();
            let pairs: Vec<String> = method
                .params
                .iter()
                .zip(args.iter())
                .map(|(p, expr)| format!("(\"{}\", {})", p.name, expr))
                .collect();
            let pairs_str = pairs.join(", ");
            writeln!(out, "let args = extendr_api::Pairlist::from_pairs(&[{pairs_str}]);").ok();
            writeln!(out, "let result = fn_robj.call(args);").ok();
        }

        // Handle the result
        let ret_ty = match &method.return_type {
            TypeRef::Named(n) => self
                .type_paths
                .get(n.as_str())
                .map(|p| p.replace('-', "_"))
                .unwrap_or_else(|| n.clone()),
            other => format_type_ref(other, &self.type_paths),
        };

        if matches!(method.return_type, TypeRef::Unit) {
            writeln!(out, "match result {{").ok();
            writeln!(out, "    Err(_) if {} => {{", if has_error { "true" } else { "false" }).ok();
            if has_error {
                writeln!(out, "        Err({}::KreuzbergError::Plugin {{", spec.core_import).ok();
                writeln!(
                    out,
                    "            message: format!(\"Plugin '{{}}' method '{}' failed\", self.cached_name),",
                    name
                )
                .ok();
                writeln!(out, "            plugin_name: self.cached_name.clone(),").ok();
                writeln!(out, "        }})").ok();
            } else {
                writeln!(out, "        Ok(())").ok();
            }
            writeln!(out, "    }}").ok();
            writeln!(out, "    _ => {}", if has_error { "Ok(())" } else { "();" }).ok();
            writeln!(out, "}}").ok();
        } else {
            writeln!(out, "match result {{").ok();
            writeln!(out, "    Err(_) => {{").ok();
            if has_error {
                writeln!(out, "        Err({}::KreuzbergError::Plugin {{", spec.core_import).ok();
                writeln!(
                    out,
                    "            message: format!(\"Plugin '{{}}' method '{}' failed\", self.cached_name),",
                    name
                )
                .ok();
                writeln!(out, "            plugin_name: self.cached_name.clone(),").ok();
                writeln!(out, "        }})").ok();
            } else {
                writeln!(out, "        Default::default()").ok();
            }
            writeln!(out, "    }}").ok();
            writeln!(out, "    Ok(val) => {{").ok();

            // Try to extract as the expected type
            match &method.return_type {
                TypeRef::String | TypeRef::Char => {
                    writeln!(out, "        if let Some(s) = val.as_str() {{").ok();
                    writeln!(out, "            Ok(s.to_string())").ok();
                    writeln!(out, "        }} else {{").ok();
                    if has_error {
                        writeln!(out, "            Err({}::KreuzbergError::Plugin {{", spec.core_import).ok();
                        writeln!(out, "                message: format!(\"Plugin '{{}}' method '{}' returned invalid type\", self.cached_name),", name).ok();
                        writeln!(out, "                plugin_name: self.cached_name.clone(),").ok();
                        writeln!(out, "            }})").ok();
                    } else {
                        writeln!(out, "            Default::default()").ok();
                    }
                    writeln!(out, "        }}").ok();
                }
                _ => {
                    // For complex types, try to serialize/deserialize via JSON
                    writeln!(out, "        match serde_json::to_string(&val) {{").ok();
                    writeln!(
                        out,
                        "            Ok(json_str) => serde_json::from_str::<{ret_ty}>(&json_str)"
                    )
                    .ok();
                    writeln!(out, "                .map_err(|_| {{").ok();
                    if has_error {
                        writeln!(
                            out,
                            "                    {}::KreuzbergError::Plugin {{",
                            spec.core_import
                        )
                        .ok();
                        writeln!(out, "                        message: format!(\"Plugin '{{}}' method '{}' deserialization failed\", self.cached_name),", name).ok();
                        writeln!(out, "                        plugin_name: self.cached_name.clone(),").ok();
                        writeln!(out, "                    }}").ok();
                    }
                    writeln!(out, "                }})").ok();
                    writeln!(out, "            Err(_) => {{").ok();
                    if has_error {
                        writeln!(
                            out,
                            "                Err({}::KreuzbergError::Plugin {{",
                            spec.core_import
                        )
                        .ok();
                        writeln!(out, "                    message: format!(\"Plugin '{{}}' method '{}' serialization failed\", self.cached_name),", name).ok();
                        writeln!(out, "                    plugin_name: self.cached_name.clone(),").ok();
                        writeln!(out, "                }})").ok();
                    } else {
                        writeln!(out, "                Default::default()").ok();
                    }
                    writeln!(out, "            }}").ok();
                    writeln!(out, "        }}").ok();
                }
            }

            writeln!(out, "    }}").ok();
            writeln!(out, "}}").ok();
        }

        out
    }

    fn gen_async_method_body(&self, method: &MethodDef, spec: &TraitBridgeSpec) -> String {
        // R is single-threaded. For async methods, we spawn_blocking to avoid
        // blocking the main event loop, then call the sync method.
        let name = &method.name;
        let mut out = String::with_capacity(1024);

        writeln!(out, "let cached_name = self.cached_name.clone();").ok();
        writeln!(out, "let r_obj = self.inner.clone();").ok();

        // Clone params for the blocking closure
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

        // Call the R method synchronously within the blocking task
        writeln!(out, "    let maybe_fn = r_obj.dollar(\"{name}\");").ok();
        writeln!(out, "    let fn_robj = match maybe_fn {{").ok();
        writeln!(out, "        Ok(v) if !v.is_null() && !v.is_na() => v,").ok();
        writeln!(
            out,
            "        _ => return Err({}::KreuzbergError::Plugin {{",
            spec.core_import
        )
        .ok();
        writeln!(
            out,
            "            message: format!(\"Plugin '{{}}' missing method '{}'\", cached_name),",
            name
        )
        .ok();
        writeln!(out, "            plugin_name: cached_name.clone(),").ok();
        writeln!(out, "        }}),").ok();
        writeln!(out, "    }};").ok();

        // Build call arguments
        if method.params.is_empty() {
            writeln!(out, "    let result = fn_robj.call(extendr_api::Pairlist::new());").ok();
        } else {
            let args: Vec<String> = method
                .params
                .iter()
                .map(|p| match (&p.ty, p.is_ref) {
                    (TypeRef::Bytes, true) => format!("extendr_api::Robj::from(&{0}[..])", p.name),
                    (TypeRef::Path, true) => format!("extendr_api::Robj::from({0}_str.as_str())", p.name),
                    (TypeRef::Named(_), true) => format!("extendr_api::Robj::from({0}_json.as_str())", p.name),
                    _ => format!("extendr_api::Robj::from({})", p.name),
                })
                .collect();
            let pairs: Vec<String> = method
                .params
                .iter()
                .zip(args.iter())
                .map(|(p, expr)| format!("(\"{}\", {})", p.name, expr))
                .collect();
            let pairs_str = pairs.join(", ");
            writeln!(out, "    let args = extendr_api::Pairlist::from_pairs(&[{pairs_str}]);").ok();
            writeln!(out, "    let result = fn_robj.call(args);").ok();
        }

        let _ret_ty = match &method.return_type {
            TypeRef::Named(n) => self
                .type_paths
                .get(n.as_str())
                .map(|p| p.replace('-', "_"))
                .unwrap_or_else(|| n.clone()),
            other => format_type_ref(other, &self.type_paths),
        };

        // Parse the result (similar to sync case but in a blocking context)
        writeln!(out, "    match result {{").ok();
        writeln!(out, "        Err(_) => {{").ok();
        writeln!(out, "            Err({}::KreuzbergError::Plugin {{", spec.core_import).ok();
        writeln!(
            out,
            "                message: format!(\"Plugin '{{}}' method '{}' failed\", cached_name),",
            name
        )
        .ok();
        writeln!(out, "                plugin_name: cached_name.clone(),").ok();
        writeln!(out, "            }})").ok();
        writeln!(out, "        }}").ok();
        writeln!(out, "        Ok(val) => {{").ok();

        if matches!(method.return_type, TypeRef::Unit) {
            writeln!(out, "            Ok(())").ok();
        } else {
            writeln!(out, "            if let Some(s) = val.as_str() {{").ok();
            writeln!(out, "                Ok(s.to_string())").ok();
            writeln!(out, "            }} else {{").ok();
            writeln!(
                out,
                "                Err({}::KreuzbergError::Plugin {{",
                spec.core_import
            )
            .ok();
            writeln!(out, "                    message: format!(\"Plugin '{{}}' method '{}' returned invalid type\", cached_name),", name).ok();
            writeln!(out, "                    plugin_name: cached_name.clone(),").ok();
            writeln!(out, "                }})").ok();
            writeln!(out, "            }}").ok();
        }

        writeln!(out, "        }}").ok();
        writeln!(out, "    }}").ok();
        writeln!(out, "}})").ok();
        writeln!(out, ".await").ok();
        writeln!(out, ".map_err(|e| {}::KreuzbergError::Plugin {{", spec.core_import).ok();
        writeln!(out, "    message: format!(\"spawn_blocking failed: {{}}\", e),").ok();
        writeln!(out, "    plugin_name: cached_name.clone(),").ok();
        writeln!(out, "}})?").ok();

        out
    }

    fn gen_constructor(&self, spec: &TraitBridgeSpec) -> String {
        let wrapper = spec.wrapper_name();
        let mut out = String::with_capacity(512);

        writeln!(out, "impl {wrapper} {{").ok();
        writeln!(
            out,
            "    /// Create a new bridge wrapping an R object (named list of functions)."
        )
        .ok();
        writeln!(out, "    ///").ok();
        writeln!(
            out,
            "    /// Validates that the R object provides all required methods."
        )
        .ok();
        writeln!(
            out,
            "    pub fn new(r_obj: extendr_api::Robj) -> Result<Self, String> {{"
        )
        .ok();

        // Validate all required methods exist
        for req_method in spec.required_methods() {
            writeln!(out, "        match r_obj.dollar(\"{}\") {{", req_method.name).ok();
            writeln!(out, "            Ok(v) if !v.is_null() && !v.is_na() => {{}},").ok();
            writeln!(
                out,
                "            _ => return Err(\"R object missing required method: {}\".to_string()),",
                req_method.name
            )
            .ok();
            writeln!(out, "        }}").ok();
        }

        // Extract and cache name (call a name() method if available, else "unknown")
        writeln!(out, "        let cached_name: String = match r_obj.dollar(\"name\") {{").ok();
        writeln!(out, "            Ok(v) if !v.is_null() && !v.is_na() => {{").ok();
        writeln!(out, "                if let Some(s) = v.as_str() {{").ok();
        writeln!(out, "                    s.to_string()").ok();
        writeln!(out, "                }} else {{").ok();
        writeln!(out, "                    \"unknown\".to_string()").ok();
        writeln!(out, "                }}").ok();
        writeln!(out, "            }}").ok();
        writeln!(out, "            _ => \"unknown\".to_string(),").ok();
        writeln!(out, "        }};").ok();

        writeln!(out).ok();
        writeln!(out, "        Ok(Self {{").ok();
        writeln!(out, "            inner: r_obj,").ok();
        writeln!(out, "            cached_name,").ok();
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

        writeln!(out, "#[extendr]").ok();
        writeln!(
            out,
            "pub fn {register_fn}(r_backend: extendr_api::Robj) -> Result<(), String> {{"
        )
        .ok();

        // Validate required methods
        let req_methods = spec.required_methods();
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
            writeln!(out, "    for method in &required_methods {{").ok();
            writeln!(out, "        match r_backend.dollar(method) {{").ok();
            writeln!(out, "            Ok(v) if !v.is_null() && !v.is_na() => {{}},").ok();
            writeln!(
                out,
                "            _ => return Err(format!(\"Backend missing required method: {{}}\", method)),"
            )
            .ok();
            writeln!(out, "        }}").ok();
            writeln!(out, "    }}").ok();
        }

        writeln!(out).ok();
        writeln!(out, "    let wrapper = {wrapper}::new(r_backend)?;").ok();
        writeln!(out, "    let arc: Arc<dyn {trait_path}> = Arc::new(wrapper);").ok();
        writeln!(out).ok();

        // Register in the plugin registry
        writeln!(out, "    let registry = {registry_getter}();").ok();
        writeln!(out, "    let mut registry = registry.write().map_err(|e| {{").ok();
        writeln!(out, "        format!(\"Failed to acquire registry lock: {{}}\", e)").ok();
        writeln!(out, "    }})?;").ok();
        writeln!(out, "    registry.register(arc).map_err(|e| {{").ok();
        writeln!(out, "        format!(\"Failed to register backend: {{}}\", e)").ok();
        writeln!(out, "    }})").ok();
        writeln!(out, "}}").ok();
        out
    }
}

/// Generate all trait bridge code for a given trait type and bridge config.
pub fn gen_trait_bridge(
    trait_type: &TypeDef,
    bridge_cfg: &TraitBridgeConfig,
    core_import: &str,
    error_type: &str,
    api: &alef_core::ir::ApiSurface,
) -> String {
    let struct_name = format!("R{}Bridge", bridge_cfg.trait_name);
    let trait_path = trait_type.rust_path.replace('-', "_");

    // Build type name → rust_path lookup (owned HashMap for use with new generator)
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

    // Visitor-style bridge: all methods have defaults, no registry, no super-trait.
    let is_visitor_bridge = bridge_cfg.type_alias.is_some()
        && bridge_cfg.register_fn.is_none()
        && bridge_cfg.super_trait.is_none()
        && trait_type.methods.iter().all(|m| m.has_default_impl);

    if is_visitor_bridge {
        let mut out = String::with_capacity(8192);
        // Convert HashMap back to &str-keyed refs for visitor bridge
        let ref_type_paths: std::collections::HashMap<&str, &str> =
            type_paths.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        gen_visitor_bridge(
            &mut out,
            trait_type,
            &struct_name,
            &trait_path,
            core_import,
            &ref_type_paths,
        );
        out
    } else {
        // Use the IR-driven TraitBridgeGenerator infrastructure
        let generator = ExtendrBridgeGenerator {
            core_import: core_import.to_string(),
            type_paths,
        };
        let spec = TraitBridgeSpec {
            trait_def: trait_type,
            bridge_config: bridge_cfg,
            core_import,
            wrapper_prefix: "R",
            type_paths: generator.type_paths.clone(),
            error_type: error_type.to_string(),
        };
        gen_bridge_all(&spec, &generator)
    }
}

/// Generate a visitor-style bridge wrapping an `extendr_api::Robj` (a named list of functions).
///
/// Every trait method checks if the list has a function with the snake_case method name,
/// calls it via extendr's `.call()`, and maps the return value to `VisitResult`.
fn gen_visitor_bridge(
    out: &mut String,
    trait_type: &TypeDef,
    struct_name: &str,
    trait_path: &str,
    core_crate: &str,
    type_paths: &std::collections::HashMap<&str, &str>,
) {
    // Helper: convert NodeContext to an R list (Robj)
    writeln!(out, "fn nodecontext_to_robj(").unwrap();
    writeln!(out, "    ctx: &{core_crate}::visitor::NodeContext,").unwrap();
    writeln!(out, ") -> extendr_api::Robj {{").unwrap();
    writeln!(out, "    use extendr_api::prelude::*;").unwrap();
    writeln!(out, "    let attrs: extendr_api::Robj = ctx.attributes.iter()").unwrap();
    writeln!(
        out,
        "        .map(|(k, v)| (k.as_str(), extendr_api::Robj::from(v.as_str())))"
    )
    .unwrap();
    writeln!(out, "        .collect::<List>().into();").unwrap();
    writeln!(out, "    list!(").unwrap();
    writeln!(out, "        node_type = format!(\"{{:?}}\", ctx.node_type),").unwrap();
    writeln!(out, "        tag_name = ctx.tag_name.as_str(),").unwrap();
    writeln!(out, "        depth = ctx.depth as i32,").unwrap();
    writeln!(out, "        index_in_parent = ctx.index_in_parent as i32,").unwrap();
    writeln!(out, "        is_inline = ctx.is_inline,").unwrap();
    writeln!(out, "        parent_tag = ctx.parent_tag.as_deref().unwrap_or(\"\"),").unwrap();
    writeln!(out, "        attributes = attrs,").unwrap();
    writeln!(out, "    ).into()").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // Bridge struct — Robj may not implement Debug, so we derive it manually.
    writeln!(out, "pub struct {struct_name} {{").unwrap();
    writeln!(out, "    r_obj: extendr_api::Robj,").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // Manual Debug impl (Robj does not derive Debug)
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
    writeln!(out, "    pub fn new(r_obj: extendr_api::Robj) -> Self {{").unwrap();
    writeln!(out, "        Self {{ r_obj }}").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // Trait impl — each method checks for a list element and calls it
    writeln!(out, "impl {trait_path} for {struct_name} {{").unwrap();
    for method in &trait_type.methods {
        if method.trait_source.is_some() {
            continue;
        }
        gen_visitor_method_extendr(out, method, type_paths);
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

/// Generate a single visitor method that checks if the R list has an element with this name
/// and calls it as a function.
fn gen_visitor_method_extendr(
    out: &mut String,
    method: &MethodDef,
    type_paths: &std::collections::HashMap<&str, &str>,
) {
    let name = &method.name;
    // R conventions: snake_case method names (same as Rust)

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
    writeln!(out, "        use extendr_api::prelude::*;").unwrap();

    // Check if the list has an element with this name by attempting dollar indexing
    writeln!(out, "        let maybe_fn = self.r_obj.dollar(\"{name}\");").unwrap();
    writeln!(out, "        let fn_robj = match maybe_fn {{").unwrap();
    writeln!(out, "            Ok(v) if !v.is_null() && !v.is_na() => v,").unwrap();
    writeln!(out, "            _ => return {ret_ty}::Continue,").unwrap();
    writeln!(out, "        }};").unwrap();

    // Build argument list for the R function call
    if method.params.is_empty() {
        writeln!(out, "        let result = fn_robj.call(extendr_api::Pairlist::new());").unwrap();
    } else {
        // Build arg expressions
        let args: Vec<String> = method.params.iter().map(build_extendr_arg).collect();
        let pairs: Vec<String> = method
            .params
            .iter()
            .zip(args.iter())
            .map(|(p, expr)| format!("(\"{}\", {})", p.name, expr))
            .collect();
        let pairs_str = pairs.join(", ");
        writeln!(
            out,
            "        let args = extendr_api::Pairlist::from_pairs(&[{pairs_str}]);"
        )
        .unwrap();
        writeln!(out, "        let result = fn_robj.call(args);").unwrap();
    }

    // Parse VisitResult from the R return value
    writeln!(out, "        match result {{").unwrap();
    writeln!(out, "            Err(_) => {ret_ty}::Continue,").unwrap();
    writeln!(out, "            Ok(val) => {{").unwrap();
    // Try string first ("skip", "continue", "preserve_html")
    writeln!(out, "                if let Some(s) = val.as_str() {{").unwrap();
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
    writeln!(out, "                }} else if val.is_null() || val.is_na() {{").unwrap();
    writeln!(out, "                    {ret_ty}::Continue").unwrap();
    writeln!(out, "                }} else {{").unwrap();
    // Try named list: list(custom = "text") or list(error = "text")
    writeln!(
        out,
        "                    if let Ok(custom_val) = val.dollar(\"custom\") {{"
    )
    .unwrap();
    writeln!(out, "                        if let Some(s) = custom_val.as_str() {{").unwrap();
    writeln!(out, "                            {ret_ty}::Custom(s.to_string())").unwrap();
    writeln!(out, "                        }} else {{").unwrap();
    writeln!(out, "                            {ret_ty}::Continue").unwrap();
    writeln!(out, "                        }}").unwrap();
    writeln!(
        out,
        "                    }} else if let Ok(error_val) = val.dollar(\"error\") {{"
    )
    .unwrap();
    writeln!(out, "                        if let Some(s) = error_val.as_str() {{").unwrap();
    writeln!(out, "                            {ret_ty}::Error(s.to_string())").unwrap();
    writeln!(out, "                        }} else {{").unwrap();
    writeln!(out, "                            {ret_ty}::Continue").unwrap();
    writeln!(out, "                        }}").unwrap();
    writeln!(out, "                    }} else {{").unwrap();
    writeln!(out, "                        {ret_ty}::Continue").unwrap();
    writeln!(out, "                    }}").unwrap();
    writeln!(out, "                }}").unwrap();
    writeln!(out, "            }}").unwrap();
    writeln!(out, "        }}").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out).unwrap();
}

/// Build a single extendr `Pairlist` arg expression for a visitor method parameter.
fn build_extendr_arg(p: &alef_core::ir::ParamDef) -> String {
    use alef_core::ir::TypeRef;

    // NodeContext: convert to an R list
    if let TypeRef::Named(n) = &p.ty {
        if n == "NodeContext" {
            let ref_prefix = if p.is_ref { "" } else { "&" };
            return format!("extendr_api::Robj::from(nodecontext_to_robj({}{}))", ref_prefix, p.name);
        }
    }

    // Option<&str>: IR collapses to String + optional + is_ref
    if p.optional && matches!(&p.ty, TypeRef::String) && p.is_ref {
        return format!(
            "match {name} {{ Some(s) => extendr_api::Robj::from(s), None => extendr_api::Robj::from(extendr_api::NULL) }}",
            name = p.name
        );
    }

    // &str: wrap in Robj
    if matches!(&p.ty, TypeRef::String) && p.is_ref {
        return format!("extendr_api::Robj::from({})", p.name);
    }

    // Owned String
    if matches!(&p.ty, TypeRef::String) {
        return format!("extendr_api::Robj::from({}.as_str())", p.name);
    }

    // bool
    if matches!(&p.ty, TypeRef::Primitive(alef_core::ir::PrimitiveType::Bool)) {
        return format!("extendr_api::Robj::from({})", p.name);
    }

    // Integer-like primitives: cast to i32 (R INTEGER)
    if let TypeRef::Primitive(prim) = &p.ty {
        use alef_core::ir::PrimitiveType;
        match prim {
            PrimitiveType::U8
            | PrimitiveType::U16
            | PrimitiveType::U32
            | PrimitiveType::I8
            | PrimitiveType::I16
            | PrimitiveType::I32 => {
                return format!("extendr_api::Robj::from({} as i32)", p.name);
            }
            PrimitiveType::U64 | PrimitiveType::I64 | PrimitiveType::Usize | PrimitiveType::Isize => {
                return format!("extendr_api::Robj::from({} as f64)", p.name);
            }
            PrimitiveType::F32 | PrimitiveType::F64 => {
                return format!("extendr_api::Robj::from({} as f64)", p.name);
            }
            PrimitiveType::Bool => {
                return format!("extendr_api::Robj::from({})", p.name);
            }
        }
    }

    // Fallback
    format!("extendr_api::Robj::from({})", p.name)
}

/// Format a TypeRef as a Rust type string, using an owned HashMap<String, String> for type paths.
/// This is used by ExtendrBridgeGenerator to format types in bridge method bodies.
fn format_type_ref(ty: &TypeRef, type_paths: &HashMap<String, String>) -> String {
    match ty {
        TypeRef::Primitive(p) => prim(p).to_string(),
        TypeRef::String => "String".to_string(),
        TypeRef::Char => "char".to_string(),
        TypeRef::Bytes => "Vec<u8>".to_string(),
        TypeRef::Optional(inner) => format!("Option<{}>", format_type_ref(inner, type_paths)),
        TypeRef::Vec(inner) => format!("Vec<{}>", format_type_ref(inner, type_paths)),
        TypeRef::Map(k, v) => format!(
            "std::collections::HashMap<{}, {}>",
            format_type_ref(k, type_paths),
            format_type_ref(v, type_paths)
        ),
        TypeRef::Named(name) => type_paths.get(name.as_str()).cloned().unwrap_or_else(|| name.clone()),
        TypeRef::Path => "std::path::PathBuf".to_string(),
        TypeRef::Unit => "()".to_string(),
        TypeRef::Json => "serde_json::Value".to_string(),
        TypeRef::Duration => "std::time::Duration".to_string(),
    }
}

/// Map TypeRef to a Rust type string for trait method signatures.
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
///
/// This is pure IR logic — copied verbatim from the PyO3 backend.
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

/// Generate an extendr free function that has one parameter replaced by `Option<extendr_api::Robj>`
/// (a trait bridge). The bridge is constructed before calling the core function.
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

    let struct_name = format!("R{}Bridge", bridge_cfg.trait_name);
    let handle_path = format!("{core_import}::visitor::VisitorHandle");
    let param_name = &func.params[bridge_param_idx].name;
    let bridge_param = &func.params[bridge_param_idx];
    let is_optional = bridge_param.optional || matches!(&bridge_param.ty, TypeRef::Optional(_));

    // Build parameter list — replace the bridge param with Option<extendr_api::Robj>
    let mut sig_parts = Vec::new();
    for (idx, p) in func.params.iter().enumerate() {
        if idx == bridge_param_idx {
            // The visitor is always optional from R's perspective (NULL means "no visitor")
            sig_parts.push(format!("{}: Option<extendr_api::Robj>", p.name));
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
    let has_error = func.error_type.is_some();
    let ret = mapper.wrap_return(&return_type, has_error);

    let err_conv = ".map_err(|e| extendr_api::Error::Other(e.to_string()))";

    // Bridge wrapping: Option<Robj> → Option<VisitorHandle>
    // We always treat it as optional since R passes NULL for missing visitors.
    let bridge_wrap = if is_optional {
        format!(
            "let {param_name}: Option<{handle_path}> = match {param_name} {{\n        \
             Some(v) if !v.is_null() => {{\n            \
             let bridge = {struct_name}::new(v);\n            \
             Some(std::rc::Rc::new(std::cell::RefCell::new(bridge)) as {handle_path})\n        \
             }},\n        \
             _ => None,\n    \
             }};"
        )
    } else {
        // Non-optional in IR, but we expose it as Option<Robj> regardless and
        // unwrap or construct a bridge from a non-null Robj.
        format!(
            "let {param_name}: Option<{handle_path}> = match {param_name} {{\n        \
             Some(v) if !v.is_null() => {{\n            \
             let bridge = {struct_name}::new(v);\n            \
             Some(std::rc::Rc::new(std::cell::RefCell::new(bridge)) as {handle_path})\n        \
             }},\n        \
             _ => None,\n    \
             }};"
        )
    };

    // Serde let-bindings for non-bridge Named params
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
                    "let {name}_core: Option<{core_path}> = {name}.as_deref()\
                     .filter(|s| *s != \"NULL\")\
                     .map(|s| serde_json::from_str(s){err_conv}).transpose()?;\n    "
                )
            } else {
                format!("let {name}_core: {core_path} = serde_json::from_str(&{name}){err_conv}?;\n    ")
            }
        })
        .collect();

    // Build call args for the core function
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
        TypeRef::Named(_) | TypeRef::String | TypeRef::Bytes => "val.into()".to_string(),
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
    writeln!(out, "#[extendr]").ok();
    writeln!(out, "pub fn {func_name}({params_str}) -> {ret} {{").ok();
    writeln!(out, "    {body}").ok();
    writeln!(out, "}}").ok();

    out
}
