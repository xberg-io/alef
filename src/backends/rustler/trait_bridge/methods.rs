use super::generator::RustlerBridgeGenerator;
use super::json_args::build_json_arg;
use crate::codegen::generators::trait_bridge::{TraitBridgeGenerator, TraitBridgeSpec};
use crate::core::ir::{MethodDef, TypeRef};

impl TraitBridgeGenerator for RustlerBridgeGenerator {
    fn foreign_object_type(&self) -> &str {
        "rustler::LocalPid"
    }

    fn bridge_imports(&self) -> Vec<String> {
        // async_trait is needed because the trait impls may have async methods.
        // We import the prelude to ensure the async_trait attribute is available.
        vec!["async_trait::async_trait".to_string()]
    }

    fn gen_sync_method_body(&self, method: &MethodDef, spec: &TraitBridgeSpec) -> String {
        let has_error = method.error_type.is_some();

        // Build clone_params array: only parameters that own their data and need to be moved.
        // Skip references to primitives like &[u8] or &str (borrowed slices/strings don't clone meaningfully).
        // Include: String (owned), Vec (owned), custom types passed by reference.
        let clone_params: Vec<minijinja::Value> = method
            .params
            .iter()
            .filter(|p| {
                // Clone String types (owned) and referenced custom types.
                // Skip Bytes (&[u8]) and bare references to primitives.
                match &p.ty {
                    TypeRef::String => !p.is_ref,  // Clone String but not &str
                    TypeRef::Bytes => false,         // Skip &[u8] and Vec<u8> (handled separately)
                    TypeRef::Named(_) => p.is_ref,  // Clone references to custom types for thread safety
                    _ => false,
                }
            })
            .map(|p| {
                minijinja::context! {
                    name => p.name.clone()
                }
            })
            .collect();

        // Build params array with json_expr
        let params: Vec<minijinja::Value> = method
            .params
            .iter()
            .map(|p| {
                let json_expr = build_json_arg(p, spec.bridge_config);
                minijinja::context! {
                    name => p.name.clone(),
                    json_expr => json_expr
                }
            })
            .collect();

        // Build error constructors
        let error_deser = spec
            .error_constructor
            .replace("{msg}", "format!(\"Failed to deserialize response: {}\", _e)");
        let error_msg = spec.error_constructor.replace("{msg}", "msg");
        let error_closed = spec
            .error_constructor
            .replace("{msg}", "\"Channel closed before reply received\".to_string()");

        let ctx = minijinja::context! {
            clone_params => clone_params,
            params => params,
            method_name => method.name,
            has_error => has_error,
            error_deser => error_deser,
            error_msg => error_msg,
            error_closed => error_closed
        };

        crate::backends::rustler::template_env::render("sync_method_body.rs.jinja", ctx)
    }

    fn gen_async_method_body(&self, method: &MethodDef, spec: &TraitBridgeSpec) -> String {
        let has_error = method.error_type.is_some();

        // Build param_clones array: only parameters that own their data and need to be moved.
        // Skip references to primitives like &[u8] or &str (borrowed slices/strings don't clone meaningfully).
        // Include: String (owned), Vec (owned), custom types passed by reference.
        let param_clones: Vec<minijinja::Value> = method
            .params
            .iter()
            .filter(|p| {
                // Clone String types (owned) and referenced custom types.
                // Skip Bytes (&[u8]) and bare references to primitives.
                match &p.ty {
                    TypeRef::String => !p.is_ref,  // Clone String but not &str
                    TypeRef::Bytes => false,         // Skip &[u8] and Vec<u8> (handled separately)
                    TypeRef::Named(_) => p.is_ref,  // Clone references to custom types for thread safety
                    _ => false,
                }
            })
            .map(|p| {
                minijinja::context! {
                    name => p.name.clone()
                }
            })
            .collect();

        // Build args_json array with name and expr
        let args_json: Vec<minijinja::Value> = method
            .params
            .iter()
            .map(|p| {
                let expr = build_json_arg(p, spec.bridge_config);
                minijinja::context! {
                    name => p.name.clone(),
                    expr => expr
                }
            })
            .collect();

        // Build error constructors
        let error_deser = spec
            .error_constructor
            .replace("{msg}", "format!(\"Failed to deserialize response: {}\", _e)");
        let error_msg = spec.error_constructor.replace("{msg}", "msg");
        let error_closed = spec
            .error_constructor
            .replace("{msg}", "\"Channel closed before reply received\".to_string()");

        let ctx = minijinja::context! {
            param_clones => param_clones,
            args_json => args_json,
            method_name => method.name,
            has_error => has_error,
            error_deser => error_deser,
            error_msg => error_msg,
            error_closed => error_closed
        };

        crate::backends::rustler::template_env::render("trait_async_method_body.rs.jinja", ctx)
    }

    fn gen_constructor(&self, spec: &TraitBridgeSpec) -> String {
        let wrapper = spec.wrapper_name();
        let ctx = minijinja::context! {
            wrapper_name => wrapper
        };
        crate::backends::rustler::template_env::render("trait_constructor.rs.jinja", ctx)
    }

    fn gen_unregistration_fn(&self, spec: &TraitBridgeSpec) -> String {
        let Some(unregister_fn) = spec.bridge_config.unregister_fn.as_deref() else {
            return String::new();
        };
        let host_path = crate::codegen::generators::trait_bridge::host_function_path(spec, unregister_fn);
        let ctx = minijinja::context! {
            unregister_fn => unregister_fn,
            host_path => host_path
        };
        crate::backends::rustler::template_env::render("trait_unregistration_fn.rs.jinja", ctx)
    }

    fn gen_clear_fn(&self, spec: &TraitBridgeSpec) -> String {
        let Some(clear_fn) = spec.bridge_config.clear_fn.as_deref() else {
            return String::new();
        };
        let host_path = crate::codegen::generators::trait_bridge::host_function_path(spec, clear_fn);
        let ctx = minijinja::context! {
            clear_fn => clear_fn,
            host_path => host_path
        };
        crate::backends::rustler::template_env::render("trait_clear_fn.rs.jinja", ctx)
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

        // Register in plugin registry, including any extra arguments (e.g., priority for PostProcessor)
        let extra_args = spec.bridge_config.register_extra_args.as_deref().unwrap_or_default();

        let ctx = minijinja::context! {
            register_fn => register_fn,
            wrapper_name => wrapper,
            trait_path => trait_path,
            registry_getter => registry_getter,
            extra_args => extra_args
        };
        crate::backends::rustler::template_env::render("trait_registration_fn.rs.jinja", ctx)
    }
}

impl RustlerBridgeGenerator {
    /// Generate support NIFs for completing trait calls from Elixir.
    pub fn gen_support_nifs(&self) -> String {
        let ctx = minijinja::context! {};
        crate::backends::rustler::template_env::render("trait_support_nifs.rs.jinja", ctx)
    }
}
