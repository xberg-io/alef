//! NAPI-RS-specific trait bridge code generation.
//!
//! Generates Rust wrapper structs that implement Rust traits by delegating
//! to JavaScript objects via NAPI-RS.

use alef_codegen::generators::trait_bridge::{
    BridgeOutput, TraitBridgeGenerator, TraitBridgeSpec, bridge_param_type as param_type, gen_bridge_all,
    host_function_path, to_camel_case, visitor_param_type,
};
use alef_core::config::TraitBridgeConfig;
use alef_core::ir::{ApiSurface, MethodDef, TypeDef, TypeRef};
use std::collections::HashMap;

/// Find the first parameter index and bridge config where the parameter's named type
/// matches a trait bridge's `type_alias`.
///
/// Returns `None` when no bridge applies.
pub use alef_codegen::generators::trait_bridge::find_bridge_param;

/// Find a bridge config that uses options_field binding and a parameter of the options_type.
/// This complements find_bridge_param which only handles FunctionParam bindings.
pub fn find_options_field_binding<'a>(
    func: &alef_core::ir::FunctionDef,
    bridges: &'a [TraitBridgeConfig],
) -> Option<(usize, &'a TraitBridgeConfig)> {
    for bridge in bridges {
        if bridge.bind_via != alef_core::config::BridgeBinding::OptionsField {
            continue;
        }
        if let Some(options_type) = &bridge.options_type {
            for (idx, param) in func.params.iter().enumerate() {
                // Check if param type is Named(options_type) or Optional(Named(options_type))
                let matches = match &param.ty {
                    alef_core::ir::TypeRef::Named(n) => n == options_type,
                    alef_core::ir::TypeRef::Optional(inner) => {
                        if let alef_core::ir::TypeRef::Named(n) = inner.as_ref() {
                            n == options_type
                        } else {
                            false
                        }
                    }
                    _ => false,
                };
                if matches {
                    return Some((idx, bridge));
                }
            }
        }
    }
    None
}

/// NAPI-specific trait bridge generator.
/// Implements code generation for bridging JavaScript objects to Rust traits.
pub struct NapiBridgeGenerator {
    /// Core crate import path (e.g., `"kreuzberg"`).
    pub core_import: String,
    /// Map of type name → fully-qualified Rust path for type references.
    pub type_paths: HashMap<String, String>,
    /// Error type name (e.g., `"KreuzbergError"`).
    pub error_type: String,
}

impl TraitBridgeGenerator for NapiBridgeGenerator {
    fn foreign_object_type(&self) -> &str {
        "napi::bindgen_prelude::Object<'static>"
    }

    fn bridge_imports(&self) -> Vec<String> {
        vec![
            "napi::bindgen_prelude::{JsObjectValue, ToNapiValue, Unknown, Object}".to_string(),
            "napi::JsValue".to_string(),
            "std::sync::Arc".to_string(),
        ]
    }

    fn gen_sync_method_body(&self, method: &MethodDef, spec: &TraitBridgeSpec) -> String {
        let name = &method.name;
        let has_error = method.error_type.is_some();

        // Get the JS function from the object
        let js_args_exprs = build_napi_args(method);
        let inner_tuple_ty = unknown_tuple_type(js_args_exprs.len());
        let args_tuple_ty = if js_args_exprs.is_empty() {
            inner_tuple_ty.clone()
        } else {
            format!("napi::bindgen_prelude::FnArgs<{inner_tuple_ty}>")
        };

        let empty_args = js_args_exprs.is_empty();
        let tuple_args = if empty_args {
            String::new()
        } else if js_args_exprs.len() == 1 {
            format!("({},)", js_args_exprs[0])
        } else {
            format!("({})", js_args_exprs.join(", "))
        };

        let error_lookup =
            spec.make_error("format!(\"Method '{}' not found on bridge object: {}\", self.cached_name, e)");
        let error_call = spec.make_error(&format!(
            "format!(\"Plugin '{{}}' method '{}' failed: {{}}\", self.cached_name, e)",
            name
        ));
        let error_coercion = spec.make_error(&format!(
            "format!(\"Failed to extract return value from method '{}': {{}}\", e)",
            name
        ));
        let error_parse = spec.make_error(&format!(
            "format!(\"Failed to parse return value for method '{}'\", self.cached_name)",
            name
        ));

        if matches!(method.return_type, TypeRef::Unit) {
            crate::template_env::render(
                "sync_method_unit_return.jinja",
                minijinja::context! {
                    method_name => name,
                    args_tuple_ty => args_tuple_ty,
                    has_error => has_error,
                    empty_args => empty_args,
                    tuple_args => tuple_args,
                    error_lookup => error_lookup,
                    error_call => error_call,
                },
            )
        } else {
            crate::template_env::render(
                "sync_method_non_unit_return.jinja",
                minijinja::context! {
                    method_name => name,
                    args_tuple_ty => args_tuple_ty,
                    has_error => has_error,
                    empty_args => empty_args,
                    tuple_args => tuple_args,
                    error_lookup => error_lookup,
                    error_call => error_call,
                    error_coercion => error_coercion,
                    error_parse => error_parse,
                },
            )
        }
    }

    fn gen_async_method_body(&self, method: &MethodDef, spec: &TraitBridgeSpec) -> String {
        let name = &method.name;

        // Build the JS function call
        let js_args_exprs = build_napi_args(method);
        let inner_tuple_ty = unknown_tuple_type(js_args_exprs.len());
        let args_tuple_ty = if js_args_exprs.is_empty() {
            inner_tuple_ty.clone()
        } else {
            format!("napi::bindgen_prelude::FnArgs<{inner_tuple_ty}>")
        };

        let empty_args = js_args_exprs.is_empty();
        let tuple_args = if empty_args {
            String::new()
        } else if js_args_exprs.len() == 1 {
            format!("({},)", js_args_exprs[0])
        } else {
            format!("({})", js_args_exprs.join(", "))
        };

        let error_lookup = spec.make_error("format!(\"Method '{}' not found on bridge object: {}\", cached_name, e)");
        let error_call = spec.make_error(&format!(
            "format!(\"Plugin '{{}}' method '{}' failed: {{}}\", cached_name, e)",
            name
        ));
        let error_coercion = spec.make_error(&format!(
            "format!(\"Failed to extract return value from method '{}': {{}}\", e)",
            name
        ));
        let error_parse = spec.make_error(&format!(
            "\"Failed to parse return value for method '{}'\".to_string()",
            name
        ));

        if matches!(method.return_type, TypeRef::Unit) {
            crate::template_env::render(
                "async_method_unit_return.jinja",
                minijinja::context! {
                    method_name => name,
                    args_tuple_ty => args_tuple_ty,
                    empty_args => empty_args,
                    tuple_args => tuple_args,
                    error_lookup => error_lookup,
                    error_call => error_call,
                },
            )
        } else {
            crate::template_env::render(
                "async_method_non_unit_return.jinja",
                minijinja::context! {
                    method_name => name,
                    args_tuple_ty => args_tuple_ty,
                    empty_args => empty_args,
                    tuple_args => tuple_args,
                    error_lookup => error_lookup,
                    error_call => error_call,
                    error_coercion => error_coercion,
                    error_parse => error_parse,
                },
            )
        }
    }

    fn gen_constructor(&self, spec: &TraitBridgeSpec) -> String {
        let wrapper = spec.wrapper_name();
        let required_methods = spec
            .required_methods()
            .iter()
            .map(|m| {
                minijinja::context! {
                    name => &m.name,
                }
            })
            .collect::<Vec<_>>();

        crate::template_env::render(
            "trait_bridge_constructor.jinja",
            minijinja::context! {
                wrapper_name => wrapper,
                required_methods => required_methods,
            },
        )
    }

    fn gen_unregistration_fn(&self, spec: &TraitBridgeSpec) -> String {
        let Some(unregister_fn) = spec.bridge_config.unregister_fn.as_deref() else {
            return String::new();
        };
        let host_path = host_function_path(spec, unregister_fn);
        let camel = to_camel_case(unregister_fn);
        crate::template_env::render(
            "unregistration_fn.jinja",
            minijinja::context! {
                unregister_fn => unregister_fn,
                camel_fn_name => camel,
                host_path => host_path,
            },
        )
    }

    fn gen_clear_fn(&self, spec: &TraitBridgeSpec) -> String {
        let Some(clear_fn) = spec.bridge_config.clear_fn.as_deref() else {
            return String::new();
        };
        let host_path = host_function_path(spec, clear_fn);
        let camel = to_camel_case(clear_fn);
        crate::template_env::render(
            "clear_fn.jinja",
            minijinja::context! {
                clear_fn => clear_fn,
                camel_fn_name => camel,
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

        let extra = spec
            .bridge_config
            .register_extra_args
            .as_deref()
            .map(|a| format!(", {a}"))
            .unwrap_or_default();

        crate::template_env::render(
            "registration_fn.jinja",
            minijinja::context! {
                register_fn => register_fn,
                wrapper => wrapper,
                trait_path => trait_path,
                registry_getter => registry_getter,
                extra_args => extra,
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
    api: &ApiSurface,
) -> BridgeOutput {
    // Build type name → rust_path lookup (converted to String-owned HashMap)
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
        let struct_name = format!("Js{}Bridge", bridge_cfg.trait_name);
        let trait_path = trait_type.rust_path.replace('-', "_");
        let code = gen_visitor_bridge(
            trait_type,
            bridge_cfg,
            &struct_name,
            &trait_path,
            core_import,
            &type_paths,
        );
        BridgeOutput { imports: vec![], code }
    } else {
        // Use the IR-driven TraitBridgeGenerator infrastructure
        let generator = NapiBridgeGenerator {
            core_import: core_import.to_string(),
            type_paths: type_paths.clone(),
            error_type: error_type.to_string(),
        };
        let spec = TraitBridgeSpec {
            trait_def: trait_type,
            bridge_config: bridge_cfg,
            core_import,
            wrapper_prefix: "Js",
            type_paths,
            error_type: error_type.to_string(),
            error_constructor: error_constructor.to_string(),
        };
        gen_bridge_all(&spec, &generator)
    }
}

/// Generate a visitor-style bridge wrapping a `napi::bindgen_prelude::Object`.
///
/// Every trait method checks if the JS object has a matching camelCase property,
/// then calls it with converted arguments and maps the JS return value to `VisitResult`.
fn gen_visitor_bridge(
    trait_type: &TypeDef,
    _bridge_cfg: &TraitBridgeConfig,
    struct_name: &str,
    trait_path: &str,
    core_crate: &str,
    type_paths: &HashMap<String, String>,
) -> String {
    let mut method_impls = String::with_capacity(4096);
    for method in &trait_type.methods {
        if method.trait_source.is_some() {
            continue;
        }
        gen_visitor_method_napi(&mut method_impls, method, trait_path, core_crate, type_paths);
    }

    crate::template_env::render(
        "visitor_bridge.jinja",
        minijinja::context! {
            core_crate => core_crate,
            struct_name => struct_name,
            trait_path => trait_path,
            method_impls => method_impls,
        },
    )
}

/// Build the Function args tuple type string for a given number of Unknown args.
fn unknown_tuple_type(count: usize) -> String {
    if count == 0 {
        return "()".to_string();
    }
    let parts = vec!["napi::bindgen_prelude::Unknown"; count];
    format!("({}{})", parts.join(", "), if count == 1 { "," } else { "" })
}

/// Generate a single visitor method that checks for a camelCase JS property and calls it.
fn gen_visitor_method_napi(
    out: &mut String,
    method: &MethodDef,
    _trait_path: &str,
    _core_crate: &str,
    type_paths: &HashMap<String, String>,
) {
    let name = &method.name;
    let js_method_name = to_camel_case(name);

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

    let arg_count = method.params.len();
    let empty_args = arg_count == 0;
    let inner_tuple_ty = unknown_tuple_type(arg_count);
    let args_tuple_ty = if empty_args {
        inner_tuple_ty
    } else {
        format!("napi::bindgen_prelude::FnArgs<{inner_tuple_ty}>")
    };

    let js_args_exprs = build_napi_args(method);
    let arg_exprs: Vec<String> = js_args_exprs
        .iter()
        .map(|expr| expr.replace("self.env()", "__env"))
        .collect();

    let tuple_args = if arg_count == 1 {
        "(arg_0,)".to_string()
    } else if arg_count > 0 {
        let arg_names: Vec<String> = (0..arg_count).map(|i| format!("arg_{i}")).collect();
        format!("({})", arg_names.join(", "))
    } else {
        String::new()
    };

    out.push_str(&crate::template_env::render(
        "visitor_method.jinja",
        minijinja::context! {
            method_name => name,
            js_method_name => js_method_name,
            signature => signature,
            return_type => return_type,
            empty_args => empty_args,
            arg_exprs => arg_exprs,
            tuple_args => tuple_args,
            args_tuple_ty => args_tuple_ty,
        },
    ));
}

/// Build NAPI argument expressions for a visitor method.
///
/// Returns one expression per parameter, each producing a `napi::bindgen_prelude::Unknown`.
fn build_napi_args(method: &MethodDef) -> Vec<String> {
    method
        .params
        .iter()

        .map(|p| {
            if let TypeRef::Named(n) = &p.ty {
                if n == "NodeContext" {
                    return format!(
                        "match nodecontext_to_js_object(&self.env(), {}{}) {{ Ok(o) => o.to_unknown(), Err(_) => unsafe {{ \
                         let r = napi::bindgen_prelude::ToNapiValue::to_napi_value(self.env().raw(), napi::bindgen_prelude::Null).unwrap_or(std::ptr::null_mut()); \
                         napi::bindgen_prelude::Unknown::from_raw_unchecked(self.env().raw(), r) }} \
                        }}",
                        if p.is_ref { "" } else { "&" },
                        p.name
                    );
                }
            }
            // Option<&str>
            if p.optional && matches!(&p.ty, TypeRef::String) && p.is_ref {
                return format!(
                    "match {name} {{ \
                     Some(s) => match self.env().create_string(s) {{ \
                       Ok(v) => v.to_unknown(), \
                       Err(_) => unsafe {{ \
                       let r = napi::bindgen_prelude::ToNapiValue::to_napi_value(self.env().raw(), napi::bindgen_prelude::Null).unwrap_or(std::ptr::null_mut()); \
                       napi::bindgen_prelude::Unknown::from_raw_unchecked(self.env().raw(), r) }} \
                     }}, \
                     None => unsafe {{ \
                       let r = napi::bindgen_prelude::ToNapiValue::to_napi_value(self.env().raw(), napi::bindgen_prelude::Null).unwrap_or(std::ptr::null_mut()); \
                       napi::bindgen_prelude::Unknown::from_raw_unchecked(self.env().raw(), r) }} \
                    }}",
                    name = p.name
                );
            }
            // &str
            if matches!(&p.ty, TypeRef::String) && p.is_ref {
                return format!(
                    "match self.env().create_string({name}) {{ \
                     Ok(s) => s.to_unknown(), \
                     Err(_) => unsafe {{ \
                     let r = napi::bindgen_prelude::ToNapiValue::to_napi_value(self.env().raw(), napi::bindgen_prelude::Null).unwrap_or(std::ptr::null_mut()); \
                     napi::bindgen_prelude::Unknown::from_raw_unchecked(self.env().raw(), r) }} \
                    }}",
                    name = p.name
                );
            }
            // String (owned)
            if matches!(&p.ty, TypeRef::String) {
                return format!(
                    "match self.env().create_string({name}.as_str()) {{ \
                     Ok(s) => s.to_unknown(), \
                     Err(_) => unsafe {{ \
                     let r = napi::bindgen_prelude::ToNapiValue::to_napi_value(self.env().raw(), napi::bindgen_prelude::Null).unwrap_or(std::ptr::null_mut()); \
                     napi::bindgen_prelude::Unknown::from_raw_unchecked(self.env().raw(), r) }} \
                    }}",
                    name = p.name
                );
            }
            // Bool
            if matches!(&p.ty, TypeRef::Primitive(alef_core::ir::PrimitiveType::Bool)) {
                return format!(
                    "unsafe {{ \
                     let r = napi::bindgen_prelude::ToNapiValue::to_napi_value(self.env().raw(), {name}).unwrap_or(std::ptr::null_mut()); \
                     napi::bindgen_prelude::Unknown::from_raw_unchecked(self.env().raw(), r) }}",
                    name = p.name
                );
            }
            // u32 / usize: create_uint32 needs a u32; usize requires the cast but u32 does not.
            if matches!(&p.ty, TypeRef::Primitive(alef_core::ir::PrimitiveType::U32)) {
                return format!(
                    "match self.env().create_uint32({name}) {{ Ok(n) => n.to_unknown(), Err(_) => unsafe {{ \
                     let r = napi::bindgen_prelude::ToNapiValue::to_napi_value(self.env().raw(), napi::bindgen_prelude::Null).unwrap_or(std::ptr::null_mut()); \
                     napi::bindgen_prelude::Unknown::from_raw_unchecked(self.env().raw(), r) }} \
                    }}",
                    name = p.name
                );
            }
            if matches!(&p.ty, TypeRef::Primitive(alef_core::ir::PrimitiveType::Usize)) {
                return format!(
                    "match self.env().create_uint32({name} as u32) {{ Ok(n) => n.to_unknown(), Err(_) => unsafe {{ \
                     let r = napi::bindgen_prelude::ToNapiValue::to_napi_value(self.env().raw(), napi::bindgen_prelude::Null).unwrap_or(std::ptr::null_mut()); \
                     napi::bindgen_prelude::Unknown::from_raw_unchecked(self.env().raw(), r) }} \
                    }}",
                    name = p.name
                );
            }
            // Vec<String> or &[String] - serialize to JSON string as fallback
            // Default: serialize as debug string
            format!(
                "match self.env().create_string(&format!(\"{{:?}}\", {name})) {{ Ok(s) => s.to_unknown(), Err(_) => unsafe {{ \
                 let r = napi::bindgen_prelude::ToNapiValue::to_napi_value(self.env().raw(), napi::bindgen_prelude::Null).unwrap_or(std::ptr::null_mut()); \
                 napi::bindgen_prelude::Unknown::from_raw_unchecked(self.env().raw(), r) }} \
                }}",
                name = p.name
            )
        })
        .collect()
}

/// Generate a NAPI free function that has one parameter replaced by
/// `Option<napi::bindgen_prelude::Object>` (a trait bridge). The bridge is constructed
/// before calling the core function.
#[allow(clippy::too_many_arguments)]
pub fn gen_bridge_function(
    func: &alef_core::ir::FunctionDef,
    bridge_param_idx: usize,
    bridge_cfg: &TraitBridgeConfig,
    mapper: &dyn alef_codegen::type_mapper::TypeMapper,
    _cfg: &alef_codegen::generators::RustBindingConfig<'_>,
    _adapter_bodies: &alef_codegen::generators::AdapterBodies,
    opaque_types: &ahash::AHashSet<String>,
    core_import: &str,
) -> String {
    use alef_core::ir::TypeRef;

    let struct_name = format!("Js{}Bridge", bridge_cfg.trait_name);
    let handle_path = format!("{core_import}::visitor::VisitorHandle");
    let param_name = &func.params[bridge_param_idx].name;
    let bridge_param = &func.params[bridge_param_idx];
    let is_optional = bridge_param.optional || matches!(&bridge_param.ty, TypeRef::Optional(_));

    // Check if this is an options_field binding pattern (visitor embedded in options struct)
    let is_options_field_binding = matches!(bridge_cfg.bind_via, alef_core::config::BridgeBinding::OptionsField);

    // Find the options parameter when using options_field binding
    let options_param_idx = if is_options_field_binding {
        func.params.iter().enumerate().find(|(_, p)| {
            matches!(&p.ty, TypeRef::Named(n) if bridge_cfg.options_type.as_ref().is_some_and(|opt_type| n == opt_type))
        }).map(|(i, _)| i)
    } else {
        None
    };

    // Build parameter list: bridge param becomes Option<Object>, no explicit env param
    // (napi v3 does not implement FromNapiValue for Env; env is obtained from the Object)
    let mut sig_parts = vec![];
    for (idx, p) in func.params.iter().enumerate() {
        if is_options_field_binding && Some(idx) == options_param_idx {
            // For options_field binding, visitor is extracted from options, not a separate param
            let ty = if p.optional || (idx > 0 && func.params[..idx].iter().any(|pp| pp.optional)) {
                format!("Option<{}>", mapper.map_type(&p.ty))
            } else {
                mapper.map_type(&p.ty)
            };
            sig_parts.push(format!("{}: {}", p.name, ty));
        } else if idx == bridge_param_idx {
            if is_optional {
                sig_parts.push(format!("{}: Option<napi::bindgen_prelude::Object>", p.name));
            } else {
                sig_parts.push(format!("{}: napi::bindgen_prelude::Object", p.name));
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

    let err_conv = ".map_err(|e| napi::Error::new(napi::Status::GenericFailure, e.to_string()))";

    // Bridge wrapping code: constructor is infallible (transmute-based).
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

    // Use From/Into for non-bridge Named params — the generated bindings have From impls.
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
                format!("let {name}_core: Option<{core_path}> = {name}.map(|v| v.into());\n    ")
            } else {
                format!("let {name}_core: {core_path} = {name}.into();\n    ")
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

    let js_name = {
        let mut result = String::with_capacity(func.name.len());
        let mut capitalize_next = false;
        for (i, c) in func.name.chars().enumerate() {
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
    };
    let js_name_attr = if js_name != func.name {
        format!("(js_name = \"{}\")", js_name)
    } else {
        String::new()
    };

    let func_name = &func.name;
    crate::template_env::render(
        "bridge_function.jinja",
        minijinja::context! {
            has_error => func.error_type.is_some(),
            js_name_attr => js_name_attr,
            func_name => func_name,
            params_str => params_str,
            ret => ret,
            body => body,
        },
    )
}

/// Generate a NAPI free function where a trait bridge is embedded in an options struct field.
/// The visitor is extracted from options before the Into conversion, wrapped in a bridge,
/// and manually injected back into the converted core options.
#[allow(clippy::too_many_arguments)]
pub fn gen_options_field_bridge_function(
    func: &alef_core::ir::FunctionDef,
    options_param_idx: usize,
    bridge_cfg: &TraitBridgeConfig,
    mapper: &dyn alef_codegen::type_mapper::TypeMapper,
    _cfg: &alef_codegen::generators::RustBindingConfig<'_>,
    opaque_types: &ahash::AHashSet<String>,
    core_import: &str,
) -> String {
    use alef_core::ir::TypeRef;

    let struct_name = format!("Js{}Bridge", bridge_cfg.trait_name);
    let handle_path = format!("{core_import}::visitor::VisitorHandle");
    let options_param = &func.params[options_param_idx];
    let options_name = &options_param.name;

    // Bridge functions always treat the options param as optional: callers may pass
    // undefined/null (no options) or an options object (with or without visitor).
    // Even if the IR marks the param as non-optional (e.g. because has_default types
    // get their Option<> stripped during IR parsing), we force Option<T> behavior here.
    let is_param_optional = true;

    // Whether the IR already marks the options param as Optional<T>.
    let ir_param_optional = matches!(&options_param.ty, TypeRef::Optional(_));

    // Build parameter list; force the options param to Option<T> if the IR didn't already.
    let params_str = {
        let mut sig_parts = vec![];
        for (i, p) in func.params.iter().enumerate() {
            let ty = mapper.map_type(&p.ty);
            if i == options_param_idx && !ir_param_optional {
                sig_parts.push(format!("{}: Option<{ty}>", p.name));
            } else {
                sig_parts.push(format!("{}: {ty}", p.name));
            }
        }
        sig_parts.join(", ")
    };

    let return_type = mapper.map_type(&func.return_type);
    let ret = mapper.wrap_return(&return_type, func.error_type.is_some());

    let err_conv = ".map_err(|e| napi::Error::new(napi::Status::GenericFailure, e.to_string()))";

    // Generate visitor extraction and bridge creation
    let visitor_extract = if is_param_optional {
        format!(
            "let visitor_handle = {options_name}.as_ref().and_then(|o| o.visitor.clone()).map(|v| {{\n    \
             let bridge = {struct_name}::new(v);\n    \
             std::rc::Rc::new(std::cell::RefCell::new(bridge)) as {handle_path}\n\
             }});"
        )
    } else {
        format!(
            "let visitor_handle = {options_name}.visitor.clone().map(|v| {{\n    \
             let bridge = {struct_name}::new(v);\n    \
             std::rc::Rc::new(std::cell::RefCell::new(bridge)) as {handle_path}\n\
             }});"
        )
    };

    // Generate options conversion with visitor preservation.
    // To avoid the From impl dropping the visitor field (it's marked as Default::default()),
    // we clear it from the cloned options before conversion, then re-inject the extracted handle.
    // This ensures the bridge wrapper survives the conversion.
    let options_convert = if is_param_optional {
        format!(
            "let {options_name}_core: Option<{core_import}::ConversionOptions> = {options_name}.map(|mut o| {{\n    \
             o.visitor = None;\n    \
             let mut result: {core_import}::ConversionOptions = o.into();\n    \
             result.visitor = visitor_handle.clone();\n    \
             result\n    \
             }});"
        )
    } else {
        format!(
            "let {options_name}_core: Option<{core_import}::ConversionOptions> = {{\n    \
             let mut o = {options_name}.clone();\n    \
             o.visitor = None;\n    \
             let mut result: {core_import}::ConversionOptions = o.into();\n    \
             result.visitor = visitor_handle.clone();\n    \
             Some(result)\n    \
             }};"
        )
    };

    // Build call args, replacing options param with the _core version
    let call_args: String = func
        .params
        .iter()
        .enumerate()
        .map(|(idx, p)| {
            if idx == options_param_idx {
                format!("{options_name}_core")
            } else {
                match &p.ty {
                    TypeRef::Named(n) if opaque_types.contains(n.as_str()) => {
                        if p.optional {
                            format!("{}.as_ref().map(|v| &v.inner)", p.name)
                        } else {
                            format!("&{}.inner", p.name)
                        }
                    }
                    TypeRef::Named(_) => format!("{}.into()", p.name),
                    TypeRef::Optional(inner) => {
                        if let TypeRef::Named(n) = inner.as_ref() {
                            if opaque_types.contains(n.as_str()) {
                                format!("{}.as_ref().map(|v| &v.inner)", p.name)
                            } else {
                                format!("{}.map(Into::into)", p.name)
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
            }
        })
        .collect::<Vec<_>>()
        .join(", ");

    let core_fn_path = {
        let path = func.rust_path.replace('-', "_");
        if path.starts_with(core_import) {
            path
        } else {
            format!("{core_import}::{}", func.name)
        }
    };
    let core_call = format!("{core_fn_path}({call_args})");

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
            format!("{visitor_extract}\n    {options_convert}\n    {core_call}{err_conv}")
        } else {
            format!("{visitor_extract}\n    {options_convert}\n    {core_call}.map(|val| {return_wrap}){err_conv}")
        }
    } else {
        format!("{visitor_extract}\n    {options_convert}\n    {core_call}")
    };

    let mut out = String::with_capacity(1024);
    if func.error_type.is_some() {
        out.push_str("#[allow(clippy::missing_errors_doc)]\n");
    }
    out.push_str("#[napi]\n");
    let func_name = &func.name;
    out.push_str(&crate::template_env::render(
        "trait_bridge_fn_wrapper.jinja",
        minijinja::context! {
            func_name => func_name,
            params_str => params_str,
            return_type => ret,
            body => body,
        },
    ));

    out
}
