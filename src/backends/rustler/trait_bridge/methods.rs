use super::generator::RustlerBridgeGenerator;
use super::native_args::build_native_args;
use crate::codegen::generators::trait_bridge::{TraitBridgeGenerator, TraitBridgeSpec};
use crate::core::ir::MethodDef;

impl TraitBridgeGenerator for RustlerBridgeGenerator {
    fn foreign_object_type(&self) -> &str {
        "rustler::LocalPid"
    }

    fn gen_lifecycle_presence_check(&self, method: &MethodDef, _spec: &TraitBridgeSpec) -> Option<String> {
        Some(format!("self.implemented_methods.contains(\"{}\")", method.name))
    }

    fn gen_method_presence_check(&self, method: &MethodDef, _spec: &TraitBridgeSpec) -> Option<String> {
        // The exported-function set is supplied by the Elixir side at registration
        // (the GenServer bridge knows its impl_module) and cached on the wrapper.
        self.forwardable_defaulted
            .contains(&method.name)
            .then(|| format!("self.implemented_methods.contains(\"{}\")", method.name))
    }

    fn extra_bridge_fields(&self, _spec: &TraitBridgeSpec) -> Vec<(String, String)> {
        // Unconditional so every plugin bridge has the same constructor and
        // registration arity, whether or not the trait has defaulted methods.
        vec![(
            "implemented_methods".to_string(),
            "std::collections::HashSet<String>".to_string(),
        )]
    }

    fn bridge_imports(&self) -> Vec<String> {
        // async_trait is needed because the trait impls may have async methods.
        // We import the prelude to ensure the async_trait attribute is available.
        vec!["async_trait::async_trait".to_string()]
    }

    fn gen_sync_method_body(&self, method: &MethodDef, spec: &TraitBridgeSpec) -> String {
        let ctx = self.native_method_ctx(method, spec);
        crate::backends::rustler::template_env::render("trait_sync_method_body.rs.jinja", ctx)
    }

    fn gen_async_method_body(&self, method: &MethodDef, spec: &TraitBridgeSpec) -> String {
        let ctx = self.native_method_ctx(method, spec);
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
    /// Build the shared minijinja context for a trait method body (sync or async).
    ///
    /// Each callback argument is materialised into an OWNED, `Encoder`-able value before the
    /// dispatch closure, then encoded into a NATIVE Erlang term map inside `send_and_clear` — so the
    /// Elixir host receives native terms (structs/maps), not a JSON string. Serde-struct params are
    /// built as the binding `NifStruct` via the shared allowlist (`struct_param_types`); other args
    /// encode as their natural native terms.
    fn native_method_ctx(&self, method: &MethodDef, spec: &TraitBridgeSpec) -> minijinja::Value {
        let has_error = method.error_type.is_some();

        let native_args: Vec<minijinja::Value> = build_native_args(&method.params, &self.struct_param_types)
            .into_iter()
            .map(|a| {
                minijinja::context! {
                    key => a.key,
                    binding => a.binding,
                    owned_expr => a.owned_expr,
                }
            })
            .collect();

        let error_deser = spec
            .error_constructor
            .replace("{msg}", "format!(\"Failed to deserialize response: {}\", _e)");
        let error_msg = spec.error_constructor.replace("{msg}", "msg");
        let error_closed = spec
            .error_constructor
            .replace("{msg}", "\"Channel closed before reply received\".to_string()");

        minijinja::context! {
            wrapper => spec.wrapper_name(),
            native_args => native_args,
            method_name => method.name,
            has_error => has_error,
            error_deser => error_deser,
            error_msg => error_msg,
            error_closed => error_closed,
        }
    }

    /// Generate support NIFs for completing trait calls from Elixir.
    pub fn gen_support_nifs(&self) -> String {
        let ctx = minijinja::context! {};
        crate::backends::rustler::template_env::render("trait_support_nifs.rs.jinja", ctx)
    }
}
