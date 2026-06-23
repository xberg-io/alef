use super::visitor_bridge::{build_napi_args, unknown_tuple_type};
use crate::codegen::generators::trait_bridge::{
    TraitBridgeGenerator, TraitBridgeSpec, host_function_path, to_camel_case,
};
use crate::core::ir::{MethodDef, TypeRef};
use std::collections::HashMap;

pub struct NapiBridgeGenerator {
    /// Core crate import path (e.g., `"sample_core"`).
    pub core_import: String,
    /// Map of type name → fully-qualified Rust path for type references.
    pub type_paths: HashMap<String, String>,
    /// Error type name (e.g., `"SampleCrateError"`).
    pub error_type: String,
    /// Callback-param type names that get NATIVE-object marshalling — known serde structs per
    /// the shared [`crate::codegen::generators::trait_bridge::is_native_marshalled_struct`] rule.
    /// For such a param the bridge constructs the binding's native JS object (the
    /// `#[napi(object)]` DTO wrapper, via the same `From<core::T>` conversion used for return
    /// values) and hands THAT to the host method, instead of serializing the param to a string.
    /// Enums, opaque/handle types, and excluded/unknown `Named` params are absent and keep their
    /// prior representation.
    pub struct_param_types: std::collections::HashSet<String>,
    /// Node type-name prefix (e.g. `"Js"`) used to name the native DTO wrapper for a struct param
    /// (`Js{TypeName}`), matching the binding's emitted `#[napi(object)]` struct.
    pub type_prefix: String,
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
            "tokio_util::sync::CancellationToken".to_string(),
        ]
    }

    fn gen_sync_method_body(&self, method: &MethodDef, spec: &TraitBridgeSpec) -> String {
        let name = &method.name;
        let js_method_name = to_camel_case(name);
        let snake_method_name = name.clone();
        let has_error = method.error_type.is_some();

        // Get the JS function from the object
        let js_args_exprs = build_napi_args(method, spec.bridge_config, &self.struct_param_types, &self.type_prefix);
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
            "format!(\"Plugin '{{}}' failed to parse return value for method '{}'\", self.cached_name)",
            name
        ));

        let has_default_impl = method.has_default_impl;
        if matches!(method.return_type, TypeRef::Unit) {
            crate::backends::napi::template_env::render(
                "sync_method_unit_return.jinja",
                minijinja::context! {
                    method_name => &js_method_name,
                    snake_case_method_name => &snake_method_name,
                    args_tuple_ty => args_tuple_ty,
                    has_error => has_error,
                    has_default_impl => has_default_impl,
                    empty_args => empty_args,
                    tuple_args => tuple_args,
                    error_lookup => error_lookup,
                    error_call => error_call,
                },
            )
        } else {
            crate::backends::napi::template_env::render(
                "sync_method_non_unit_return.jinja",
                minijinja::context! {
                    method_name => &js_method_name,
                    snake_case_method_name => &snake_method_name,
                    args_tuple_ty => args_tuple_ty,
                    has_error => has_error,
                    has_default_impl => has_default_impl,
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
        let js_method_name = to_camel_case(name);
        let snake_method_name = name.clone();

        // Build the JS function call
        let js_args_exprs = build_napi_args(method, spec.bridge_config, &self.struct_param_types, &self.type_prefix);
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
            crate::backends::napi::template_env::render(
                "async_method_unit_return.jinja",
                minijinja::context! {
                    method_name => &js_method_name,
                    snake_case_method_name => &snake_method_name,
                    args_tuple_ty => args_tuple_ty,
                    empty_args => empty_args,
                    tuple_args => tuple_args,
                    error_lookup => error_lookup,
                    error_call => error_call,
                },
            )
        } else {
            crate::backends::napi::template_env::render(
                "async_method_non_unit_return.jinja",
                minijinja::context! {
                    method_name => &js_method_name,
                    snake_case_method_name => &snake_method_name,
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
                let js_name = to_camel_case(&m.name);
                let snake_name = m.name.clone();
                minijinja::context! {
                    name => js_name,
                    snake_case_name => snake_name,
                }
            })
            .collect::<Vec<_>>();

        crate::backends::napi::template_env::render(
            "trait_bridge_constructor.jinja",
            minijinja::context! {
                wrapper_name => wrapper,
                required_methods => required_methods,
                requires_plugin_name => spec.bridge_config.super_trait.is_some(),
            },
        )
    }

    fn gen_unregistration_fn(&self, spec: &TraitBridgeSpec) -> String {
        let Some(unregister_fn) = spec.bridge_config.unregister_fn.as_deref() else {
            return String::new();
        };
        let host_path = host_function_path(spec, unregister_fn);
        let camel = to_camel_case(unregister_fn);
        crate::backends::napi::template_env::render(
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
        crate::backends::napi::template_env::render(
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

        crate::backends::napi::template_env::render(
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
