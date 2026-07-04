//! Plugin trait-bridge generation for the JNI backend.
//!
//! Kotlin-Android registration passes a generated `<Trait>JniDispatcher` (which
//! wraps the user's `I<Trait>` — the interface methods are `suspend`, so they
//! cannot be invoked over raw JNI). The generated Rust bridge holds a global
//! reference to that dispatcher and routes every trait method call through its
//! `dispatch(method, argsJson)` entry point as JSON, with bytes crossing as
//! base64 (the backend's existing convention). Built on the shared
//! [`TraitBridgeGenerator`] infrastructure, so Rust-defaulted methods and the
//! `Plugin` lifecycle hooks get presence-guarded forwarding with the Rust
//! default as fallback.

use std::collections::{HashMap, HashSet};

use crate::backends::jni::template_env;
use crate::codegen::generators::trait_bridge::{
    BridgeOutput, TraitBridgeGenerator, TraitBridgeSpec, format_type_ref, gen_bridge_all,
};
use crate::core::config::TraitBridgeConfig;
use crate::core::ir::{ApiSurface, MethodDef, TypeDef, TypeRef};

pub struct JniBridgeGenerator {
    /// Fully-qualified JNI symbol for the registration shim
    /// (e.g. `Java_dev_sample_1crate_DemoBridge_nativeRegisterTextBackend`).
    pub register_symbol: String,
    /// Rust-defaulted trait methods the bridge forwards to the host when the
    /// dispatcher reports them implemented. Methods absent here keep the trait's
    /// Rust default unconditionally.
    pub forwardable_defaulted: HashSet<String>,
    /// Map of type name → fully-qualified Rust path for return-type deserialization.
    pub type_paths: HashMap<String, String>,
}

impl JniBridgeGenerator {
    /// serde_json expression for one dispatched argument.
    fn arg_expr(param: &crate::core::ir::ParamDef) -> String {
        match (&param.ty, param.is_ref) {
            // Bytes cross as base64 strings; Jackson decodes base64 into ByteArray natively.
            (TypeRef::Bytes, _) => format!(
                "base64::Engine::encode(&base64::engine::general_purpose::STANDARD, {})",
                param.name
            ),
            (TypeRef::Path, _) => format!("{}.to_string_lossy()", param.name),
            // Everything else (strings, primitives, serde structs, enums, options,
            // collections) serializes directly — `json!` accepts references.
            _ => param.name.clone(),
        }
    }

    fn method_body(&self, method: &MethodDef, spec: &TraitBridgeSpec) -> String {
        let name = &method.name;
        let has_error = method.error_type.is_some();
        let is_unit = matches!(method.return_type, TypeRef::Unit);

        let args: Vec<minijinja::Value> = method
            .params
            .iter()
            .map(|p| {
                minijinja::context! {
                    name => &p.name,
                    expr => Self::arg_expr(p),
                }
            })
            .collect();

        let dispatch_error_expr = spec.make_error(&format!(
            "format!(\"Plugin '{{}}' method '{name}' failed: {{}}\", self.cached_name, e)"
        ));
        let parse_error_expr = spec.make_error(&format!(
            "format!(\"Plugin '{{}}' method '{name}' returned invalid JSON: {{}}\", self.cached_name, e)"
        ));
        let host_error_expr = spec.make_error(&format!(
            "format!(\"Plugin '{{}}' method '{name}' failed: {{}}\", self.cached_name, e)"
        ));
        // The serde error names the offending field so a host returning a
        // mismatched shape can fix it.
        let deser_error_expr = spec.make_error(&format!(
            "format!(\"Plugin '{{}}' method '{name}' returned a value that does not match the expected return type: {{}}\", self.cached_name, e)"
        ));

        template_env::render(
            "trait_bridge_method_body.rs.jinja",
            minijinja::context! {
                method_name => name,
                wrapper => spec.wrapper_name(),
                args => args,
                has_error => has_error,
                is_unit => is_unit,
                return_ty => format_type_ref(&method.return_type, &self.type_paths),
                dispatch_error_expr => dispatch_error_expr,
                parse_error_expr => parse_error_expr,
                host_error_expr => host_error_expr,
                deser_error_expr => deser_error_expr,
            },
        )
    }
}

impl TraitBridgeGenerator for JniBridgeGenerator {
    fn foreign_object_type(&self) -> &str {
        "jni::refs::Global<jni::objects::JObject<'static>>"
    }

    fn bridge_imports(&self) -> Vec<String> {
        Vec::new()
    }

    fn gen_method_presence_check(&self, method: &MethodDef, _spec: &TraitBridgeSpec) -> Option<String> {
        // The dispatcher reports the host's implemented methods at registration.
        self.forwardable_defaulted
            .contains(&method.name)
            .then(|| format!("self.implemented_methods.contains(\"{}\")", method.name))
    }

    fn gen_lifecycle_presence_check(&self, method: &MethodDef, _spec: &TraitBridgeSpec) -> Option<String> {
        Some(format!("self.implemented_methods.contains(\"{}\")", method.name))
    }

    fn extra_bridge_fields(&self, _spec: &TraitBridgeSpec) -> Vec<(String, String)> {
        vec![
            ("jvm".to_string(), "jni::JavaVM".to_string()),
            (
                "implemented_methods".to_string(),
                "std::collections::HashSet<String>".to_string(),
            ),
        ]
    }

    fn gen_sync_method_body(&self, method: &MethodDef, spec: &TraitBridgeSpec) -> String {
        self.method_body(method, spec)
    }

    fn gen_async_method_body(&self, method: &MethodDef, spec: &TraitBridgeSpec) -> String {
        // The dispatch call is blocking (JVM attach + synchronous Java call), the
        // same execution model the generated service handler bridges use.
        self.method_body(method, spec)
    }

    fn gen_constructor(&self, spec: &TraitBridgeSpec) -> String {
        template_env::render(
            "trait_bridge_constructor.rs.jinja",
            minijinja::context! {
                wrapper_name => spec.wrapper_name(),
                trait_name => &spec.trait_def.name,
                has_super_trait => spec.bridge_config.super_trait.is_some(),
            },
        )
    }

    fn gen_registration_fn(&self, spec: &TraitBridgeSpec) -> String {
        let register_fn = spec.bridge_config.register_fn.as_deref().unwrap_or_default();
        let register_extra_args = spec.bridge_config.register_extra_args.as_deref().unwrap_or_default();
        template_env::render(
            "trait_bridge_register_shim.rs.jinja",
            minijinja::context! {
                symbol => &self.register_symbol,
                wrapper_name => spec.wrapper_name(),
                trait_name => &spec.trait_def.name,
                trait_path => spec.trait_path(),
                has_super_trait => spec.bridge_config.super_trait.is_some(),
                register_call => format!("{}::{}", spec.core_import, register_fn),
                register_extra_args => register_extra_args,
            },
        )
    }
}

/// Generate the full plugin bridge (wrapper struct, trait impl, default
/// delegates, registration shim) for one configured trait bridge.
pub fn gen_plugin_trait_bridge(
    trait_def: &TypeDef,
    bridge_cfg: &TraitBridgeConfig,
    register_symbol: &str,
    core_import: &str,
    error_type: &str,
    error_constructor: &str,
    api: &ApiSurface,
) -> BridgeOutput {
    let type_paths: HashMap<String, String> = api
        .types
        .iter()
        .map(|t| (t.name.clone(), t.rust_path.replace('-', "_")))
        .chain(
            api.enums
                .iter()
                .map(|e| (e.name.clone(), e.rust_path.replace('-', "_"))),
        )
        .chain(
            api.excluded_type_paths
                .iter()
                .map(|(name, path)| (name.clone(), path.replace('-', "_"))),
        )
        .collect();

    // Rust-defaulted methods the bridge can forward to the host (host-implemented
    // methods win; the Rust default runs otherwise).
    let forwardable_defaulted =
        crate::codegen::generators::trait_bridge::forwardable_defaulted_method_names(trait_def, api);

    let generator = JniBridgeGenerator {
        register_symbol: register_symbol.to_string(),
        forwardable_defaulted,
        type_paths: type_paths.clone(),
    };

    let lifetime_type_names: HashSet<String> = api
        .types
        .iter()
        .filter(|t| t.has_lifetime_params)
        .map(|t| t.name.clone())
        .collect();

    let spec = TraitBridgeSpec {
        trait_def,
        bridge_config: bridge_cfg,
        core_import,
        wrapper_prefix: "Jni",
        type_paths,
        lifetime_type_names,
        error_type: error_type.to_string(),
        error_constructor: error_constructor.to_string(),
    };

    gen_bridge_all(&spec, &generator)
}
