//! R (extendr) specific trait bridge code generation.
//!
//! Generates Rust wrapper structs that implement Rust traits by delegating
//! to R objects (named lists of functions) via extendr.

pub use alef_codegen::generators::trait_bridge::find_bridge_param;
use alef_codegen::generators::trait_bridge::{
    BridgeOutput, TraitBridgeGenerator, TraitBridgeSpec, bridge_param_type as param_type, format_type_ref,
    gen_bridge_all, visitor_param_type,
};
use alef_core::config::TraitBridgeConfig;
use alef_core::ir::{MethodDef, TypeDef, TypeRef};
use std::collections::HashMap;

/// Extendr-specific trait bridge generator.
/// Implements code generation for bridging R objects to Rust traits.
pub struct ExtendrBridgeGenerator {
    /// Core crate import path (e.g., `"kreuzberg"`).
    pub core_import: String,
    /// Map of type name → fully-qualified Rust path for type references.
    pub type_paths: HashMap<String, String>,
    pub error_type: String,
}

impl TraitBridgeGenerator for ExtendrBridgeGenerator {
    fn foreign_object_type(&self) -> &str {
        "extendr_api::Robj"
    }

    fn bridge_imports(&self) -> Vec<String> {
        // Return bare import paths (no `use` keyword, no trailing `;`).
        // RustFileBuilder::add_import() wraps them as `use {path};`.
        vec!["extendr_api::prelude::*".to_string(), "std::sync::Arc".to_string()]
    }

    fn gen_sync_method_body(&self, method: &MethodDef, spec: &TraitBridgeSpec) -> String {
        let name = &method.name;
        let has_error = method.error_type.is_some();

        // Build argument list for the R function call
        let (empty_args, args_pairs) = if method.params.is_empty() {
            (true, String::new())
        } else {
            let args: Vec<String> = method.params.iter().map(build_extendr_arg).collect();
            let pairs: Vec<String> = method
                .params
                .iter()
                .zip(args.iter())
                .map(|(p, expr)| format!("(\"{}\", {})", p.name.trim_start_matches('_'), expr))
                .collect();
            (false, pairs.join(", "))
        };

        // Determine which template to use based on return type
        let template_name = match &method.return_type {
            TypeRef::Unit => "sync_method_unit_return.jinja",
            TypeRef::String | TypeRef::Char => "sync_method_string_return.jinja",
            _ => "sync_method_complex_return.jinja",
        };

        let ret_ty = match &method.return_type {
            TypeRef::Named(n) => self
                .type_paths
                .get(n.as_str())
                .map(|p| p.replace('-', "_"))
                .unwrap_or_else(|| n.clone()),
            other => format_type_ref(other, &self.type_paths),
        };

        crate::template_env::render(
            template_name,
            minijinja::context! {
                method_name => name,
                has_error => has_error,
                has_error_check => if has_error { "true" } else { "false" },
                core_import => &spec.core_import,
                empty_args => empty_args,
                args_pairs => args_pairs,
                return_type => ret_ty,
            },
        )
    }

    fn gen_async_method_body(&self, method: &MethodDef, spec: &TraitBridgeSpec) -> String {
        let name = &method.name;

        // Generate param cloning statements
        let mut params_to_clone = Vec::new();
        for p in &method.params {
            let clone_stmt = match (&p.ty, p.is_ref) {
                (TypeRef::Bytes, true) => format!("let {} = {}.to_vec();", p.name, p.name),
                (TypeRef::Path, true) => format!("let {}_str = {}.to_string_lossy().to_string();", p.name, p.name),
                (TypeRef::Named(_), true) => format!(
                    "let {}_json = serde_json::to_string({}).unwrap_or_default();",
                    p.name, p.name
                ),
                (_, true) => format!("let {} = {}.to_owned();", p.name, p.name),
                _ => format!("let {} = {}.clone();", p.name, p.name),
            };
            params_to_clone.push(clone_stmt);
        }

        // Build argument list for the R function call
        let (empty_args, args_pairs) = if method.params.is_empty() {
            (true, String::new())
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
                .map(|(p, expr)| format!("(\"{}\", {})", p.name.trim_start_matches('_'), expr))
                .collect();
            (false, pairs.join(", "))
        };

        // Choose template based on return type
        let template_name = if matches!(method.return_type, TypeRef::Unit) {
            "async_method_unit_return.jinja"
        } else {
            "async_method_non_unit_return.jinja"
        };

        crate::template_env::render(
            template_name,
            minijinja::context! {
                method_name => name,
                core_import => &spec.core_import,
                params_to_clone => params_to_clone,
                empty_args => empty_args,
                args_pairs => args_pairs,
            },
        )
    }

    fn gen_constructor(&self, spec: &TraitBridgeSpec) -> String {
        let wrapper = spec.wrapper_name();
        let required_methods: Vec<String> = spec.required_methods().iter().map(|m| m.name.clone()).collect();

        crate::template_env::render(
            "bridge_constructor.jinja",
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
        let host_path = alef_codegen::generators::trait_bridge::host_function_path(spec, unregister_fn);
        crate::template_env::render(
            "unregistration_fn.jinja",
            minijinja::context! {
                unregister_fn => unregister_fn,
                host_path => host_path,
            },
        )
    }

    fn gen_clear_fn(&self, spec: &TraitBridgeSpec) -> String {
        let Some(clear_fn) = spec.bridge_config.clear_fn.as_deref() else {
            return String::new();
        };
        let host_path = alef_codegen::generators::trait_bridge::host_function_path(spec, clear_fn);
        crate::template_env::render(
            "clear_fn.jinja",
            minijinja::context! {
                clear_fn => clear_fn,
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

        let req_methods = spec.required_methods();
        let has_methods = !req_methods.is_empty();
        let required_methods_list = req_methods
            .iter()
            .map(|m| format!("\"{}\"", m.name))
            .collect::<Vec<_>>()
            .join(", ");

        crate::template_env::render(
            "registration_fn.jinja",
            minijinja::context! {
                register_fn => register_fn,
                wrapper => wrapper,
                trait_path => trait_path,
                registry_getter => registry_getter,
                required_methods => has_methods,
                required_methods_list => required_methods_list,
            },
        )
    }
}

/// Generate all trait bridge code for a given trait type and bridge config.
pub fn gen_trait_bridge(
    trait_type: &TypeDef,
    bridge_cfg: &TraitBridgeConfig,
    core_import: &str,
    error_type: &str,
    error_constructor: &str,
    api: &alef_core::ir::ApiSurface,
) -> BridgeOutput {
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
        // Include excluded types so trait methods referencing them (e.g. `&InternalDocument`)
        // are qualified with the full Rust path rather than emitting the bare type name.
        .chain(
            api.excluded_type_paths
                .iter()
                .map(|(name, path)| (name.clone(), path.replace('-', "_"))),
        )
        .collect();

    // Visitor-style bridge: all methods have defaults, no registry, no super-trait.
    let is_visitor_bridge = bridge_cfg.type_alias.is_some()
        && bridge_cfg.register_fn.is_none()
        && bridge_cfg.super_trait.is_none()
        && trait_type.methods.iter().all(|m| m.has_default_impl);

    if is_visitor_bridge {
        let mut out = String::with_capacity(8192);
        gen_visitor_bridge(
            &mut out,
            trait_type,
            &struct_name,
            &trait_path,
            core_import,
            &type_paths,
        );
        BridgeOutput {
            imports: vec![],
            code: out,
        }
    } else {
        // Use the IR-driven TraitBridgeGenerator infrastructure
        let generator = ExtendrBridgeGenerator {
            core_import: core_import.to_string(),
            type_paths: type_paths.clone(),
            error_type: error_type.to_string(),
        };
        let spec = TraitBridgeSpec {
            trait_def: trait_type,
            bridge_config: bridge_cfg,
            core_import,
            wrapper_prefix: "R",
            type_paths,
            error_type: error_type.to_string(),
            error_constructor: error_constructor.to_string(),
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
    type_paths: &std::collections::HashMap<String, String>,
) {
    let mut method_impls = String::with_capacity(4096);
    for method in &trait_type.methods {
        if method.trait_source.is_some() {
            continue;
        }
        gen_visitor_method_extendr(&mut method_impls, method, type_paths);
    }

    let bridge = crate::template_env::render(
        "visitor_bridge.jinja",
        minijinja::context! {
            core_crate => core_crate,
            struct_name => struct_name,
            trait_path => trait_path,
            method_impls => method_impls,
        },
    );
    out.push_str(&bridge);
}

/// Generate a single visitor method that checks if the R list has an element with this name
/// and calls it as a function.
fn gen_visitor_method_extendr(
    out: &mut String,
    method: &MethodDef,
    type_paths: &std::collections::HashMap<String, String>,
) {
    let name = &method.name;

    let mut sig_parts = vec!["&mut self".to_string()];
    for p in &method.params {
        let ty_str = visitor_param_type(&p.ty, p.is_ref, p.optional, type_paths);
        sig_parts.push(format!("{}: {}", p.name, ty_str));
    }
    let signature = sig_parts.join(", ");

    let return_type = match &method.return_type {
        TypeRef::Named(n) => type_paths
            .get(n.as_str())
            .map(|p| p.replace('-', "_"))
            .unwrap_or_else(|| n.clone()),
        other => param_type(other, "", false, type_paths),
    };

    let empty_args = method.params.is_empty();
    let args: Vec<String> = method.params.iter().map(build_extendr_arg).collect();
    let args_pairs: Vec<String> = method
        .params
        .iter()
        .zip(args.iter())
        .map(|(p, expr)| format!("(\"{}\", {})", p.name.trim_start_matches('_'), expr))
        .collect();
    let args_pairs = args_pairs.join(", ");

    out.push_str(&crate::template_env::render(
        "visitor_method.jinja",
        minijinja::context! {
            method_name => name,
            signature => signature,
            return_type => return_type,
            empty_args => empty_args,
            args_pairs => args_pairs,
        },
    ));
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
    crate::template_env::render(
        "bridge_function.jinja",
        minijinja::context! {
            has_error => func.error_type.is_some(),
            func_name => func_name,
            params_str => params_str,
            ret => ret,
            body => body,
        },
    )
}
