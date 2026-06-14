//! Kotlin JVM trait-bridge helper function generator.
//!
//! Implements [`TraitBridgeGenerator`] for the JVM Kotlin target.  The JVM
//! backend delegates all Panama FFM upcall-stub work to the generated Java
//! bridge class (`{TraitPascal}Bridge`); the methods in this module emit
//! thin Kotlin wrapper functions that Kotlin callers can use from the same
//! package without reaching through the Java facade directly.
//!
//! # Generated shape
//!
//! Given a trait bridge configured as:
//!
//! ```toml
//! [[trait_bridges]]
//! trait_name      = "TextBackend"
//! register_fn     = "register_text_backend"
//! unregister_fn   = "unregister_text_backend"
//! clear_fn        = "clear_text_backends"
//! ```
//!
//! …and a Java package `dev.sample_core`, the generator emits:
//!
//! ```kotlin
//! fun registerTextBackend(impl: dev.sample_core.ITextBackend) {
//!     dev.sample_core.TextBackendBridge.registerTextBackend(impl)
//! }
//!
//! fun unregisterTextBackend(name: String) {
//!     dev.sample_core.TextBackendBridge.unregisterTextBackend(name)
//! }
//!
//! fun clearTextBackends() {
//!     dev.sample_core.TextBackendBridge.clearAllTextBackend()
//! }
//! ```
//!
//! The methods live as top-level functions inside the generated Kotlin object
//! block, alongside the regular function wrappers.

use crate::codegen::generators::trait_bridge::{TraitBridgeGenerator, TraitBridgeSpec};
use crate::core::ir::MethodDef;

use super::{to_lower_camel, to_pascal_case};

/// Kotlin JVM trait-bridge generator.
///
/// Emits thin Kotlin wrapper functions that delegate registration, unregistration,
/// and clear operations to the generated Java bridge class for a trait.
pub struct KotlinJvmBridgeGenerator {
    /// Java package (e.g. `"dev.sample_core"`) — used to qualify the bridge class.
    pub java_package: String,
}

impl TraitBridgeGenerator for KotlinJvmBridgeGenerator {
    // -----------------------------------------------------------------------
    // Rust-side bridge infrastructure — not used by the JVM Kotlin backend.
    // The Java facade handles all Panama FFM upcall stubs; these methods
    // intentionally return empty strings so callers that gate on emptiness
    // skip Rust-side struct/impl emission for this target.
    // -----------------------------------------------------------------------

    fn foreign_object_type(&self) -> &str {
        ""
    }

    fn bridge_imports(&self) -> Vec<String> {
        vec![]
    }

    fn gen_sync_method_body(&self, _method: &MethodDef, _spec: &TraitBridgeSpec) -> String {
        String::new()
    }

    fn gen_async_method_body(&self, _method: &MethodDef, _spec: &TraitBridgeSpec) -> String {
        String::new()
    }

    fn gen_constructor(&self, _spec: &TraitBridgeSpec) -> String {
        String::new()
    }

    // -----------------------------------------------------------------------
    // Kotlin-side helper functions
    // -----------------------------------------------------------------------

    /// Emit a Kotlin `fun register{Trait}(impl: I{Trait})` wrapper that
    /// delegates to the Java bridge class's static `register{Trait}` method.
    ///
    /// Returns an empty string when `bridge_config.register_fn` is `None`.
    fn gen_registration_fn(&self, spec: &TraitBridgeSpec) -> String {
        let Some(register_fn) = spec.bridge_config.register_fn.as_deref() else {
            return String::new();
        };
        let trait_pascal = to_pascal_case(&spec.trait_def.name);
        let kotlin_fn = to_lower_camel(register_fn);
        let bridge_class = format!("{}.{}Bridge", self.java_package, trait_pascal);
        let iface = format!("{}.I{}", self.java_package, trait_pascal);
        let java_method = format!("register{trait_pascal}");
        format!("    fun {kotlin_fn}(impl: {iface}) {{\n        {bridge_class}.{java_method}(impl)\n    }}\n")
    }

    /// Emit a Kotlin `fun unregister{Trait}(name: String)` wrapper that
    /// delegates to the Java bridge class's static `unregister{Trait}` method.
    ///
    /// Returns an empty string when `bridge_config.unregister_fn` is `None`.
    fn gen_unregistration_fn(&self, spec: &TraitBridgeSpec) -> String {
        let Some(unregister_fn) = spec.bridge_config.unregister_fn.as_deref() else {
            return String::new();
        };
        let trait_pascal = to_pascal_case(&spec.trait_def.name);
        let kotlin_fn = to_lower_camel(unregister_fn);
        let bridge_class = format!("{}.{}Bridge", self.java_package, trait_pascal);
        let java_method = format!("unregister{trait_pascal}");
        format!("    fun {kotlin_fn}(name: String) {{\n        {bridge_class}.{java_method}(name)\n    }}\n")
    }

    /// Emit a Kotlin `fun clear{Trait}s()` wrapper that delegates to the Java
    /// bridge class's static `clearAll{Trait}` method.
    ///
    /// Returns an empty string when `bridge_config.clear_fn` is `None`.
    fn gen_clear_fn(&self, spec: &TraitBridgeSpec) -> String {
        let Some(clear_fn) = spec.bridge_config.clear_fn.as_deref() else {
            return String::new();
        };
        let trait_pascal = to_pascal_case(&spec.trait_def.name);
        let kotlin_fn = to_lower_camel(clear_fn);
        let bridge_class = format!("{}.{}Bridge", self.java_package, trait_pascal);
        let java_method = format!("clearAll{trait_pascal}");
        format!("    fun {kotlin_fn}() {{\n        {bridge_class}.{java_method}()\n    }}\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::TraitBridgeConfig;
    use crate::core::ir::TypeDef;

    fn make_bridge_config(
        trait_name: &str,
        register_fn: Option<&str>,
        unregister_fn: Option<&str>,
        clear_fn: Option<&str>,
    ) -> TraitBridgeConfig {
        TraitBridgeConfig {
            trait_name: trait_name.to_string(),
            super_trait: None,
            registry_getter: Some("demo::get_registry".to_string()),
            register_fn: register_fn.map(|s| s.to_string()),
            unregister_fn: unregister_fn.map(|s| s.to_string()),
            clear_fn: clear_fn.map(|s| s.to_string()),
            type_alias: None,
            param_name: None,
            register_extra_args: None,
            exclude_languages: vec![],
            ffi_skip_methods: Vec::new(),
            bind_via: crate::core::config::BridgeBinding::FunctionParam,
            options_type: None,
            options_field: None,
            context_type: None,
            result_type: None,
        }
    }

    fn make_trait_def(name: &str) -> TypeDef {
        TypeDef {
            name: name.to_string(),
            rust_path: format!("demo::{name}"),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![],
            is_opaque: false,
            is_clone: false,
            is_copy: false,
            is_trait: true,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: false,
            super_traits: vec![],
            doc: String::new(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            version: Default::default(),
        }
    }

    fn make_spec<'a>(trait_def: &'a TypeDef, bridge_cfg: &'a TraitBridgeConfig) -> TraitBridgeSpec<'a> {
        TraitBridgeSpec {
            trait_def,
            bridge_config: bridge_cfg,
            core_import: "demo",
            wrapper_prefix: "Kotlin",
            type_paths: std::collections::HashMap::new(),
            lifetime_type_names: std::collections::HashSet::new(),
            error_type: "DemoError".to_string(),
            error_constructor: "DemoError::from({msg})".to_string(),
        }
    }

    fn make_generator() -> KotlinJvmBridgeGenerator {
        KotlinJvmBridgeGenerator {
            java_package: "dev.sample_crate".to_string(),
        }
    }

    // --- gen_registration_fn -----------------------------------------------

    #[test]
    fn registration_fn_emits_kotlin_fun_when_set() {
        let cfg = make_bridge_config("TextBackend", Some("register_text_backend"), None, None);
        let trait_def = make_trait_def("TextBackend");
        let spec = make_spec(&trait_def, &cfg);
        let generator = make_generator();

        let out = generator.gen_registration_fn(&spec);
        assert!(!out.is_empty(), "should emit non-empty string when register_fn is set");
        assert!(
            out.contains("fun registerTextBackend(impl: dev.sample_crate.ITextBackend)"),
            "must have correct signature: {out}"
        );
        assert!(
            out.contains("dev.sample_crate.TextBackendBridge.registerTextBackend(impl)"),
            "must delegate to Java bridge: {out}"
        );
    }

    #[test]
    fn registration_fn_returns_empty_when_none() {
        let cfg = make_bridge_config("TextBackend", None, None, None);
        let trait_def = make_trait_def("TextBackend");
        let spec = make_spec(&trait_def, &cfg);
        let generator = make_generator();

        assert!(generator.gen_registration_fn(&spec).is_empty());
    }

    // --- gen_unregistration_fn ---------------------------------------------

    #[test]
    fn unregistration_fn_emits_kotlin_fun_when_set() {
        let cfg = make_bridge_config(
            "TextBackend",
            Some("register_text_backend"),
            Some("unregister_text_backend"),
            None,
        );
        let trait_def = make_trait_def("TextBackend");
        let spec = make_spec(&trait_def, &cfg);
        let generator = make_generator();

        let out = generator.gen_unregistration_fn(&spec);
        assert!(
            !out.is_empty(),
            "should emit non-empty string when unregister_fn is set"
        );
        assert!(
            out.contains("fun unregisterTextBackend(name: String)"),
            "must have correct signature: {out}"
        );
        assert!(
            out.contains("dev.sample_crate.TextBackendBridge.unregisterTextBackend(name)"),
            "must delegate to Java bridge: {out}"
        );
    }

    #[test]
    fn unregistration_fn_returns_empty_when_none() {
        let cfg = make_bridge_config("TextBackend", Some("register_text_backend"), None, None);
        let trait_def = make_trait_def("TextBackend");
        let spec = make_spec(&trait_def, &cfg);
        let generator = make_generator();

        assert!(generator.gen_unregistration_fn(&spec).is_empty());
    }

    // --- gen_clear_fn ------------------------------------------------------

    #[test]
    fn clear_fn_emits_kotlin_fun_when_set() {
        let cfg = make_bridge_config(
            "TextBackend",
            Some("register_text_backend"),
            None,
            Some("clear_text_backends"),
        );
        let trait_def = make_trait_def("TextBackend");
        let spec = make_spec(&trait_def, &cfg);
        let generator = make_generator();

        let out = generator.gen_clear_fn(&spec);
        assert!(!out.is_empty(), "should emit non-empty string when clear_fn is set");
        assert!(
            out.contains("fun clearTextBackends()"),
            "must have correct no-arg signature: {out}"
        );
        assert!(
            out.contains("dev.sample_crate.TextBackendBridge.clearAllTextBackend()"),
            "must delegate to Java bridge: {out}"
        );
    }

    #[test]
    fn clear_fn_returns_empty_when_none() {
        let cfg = make_bridge_config("TextBackend", Some("register_text_backend"), None, None);
        let trait_def = make_trait_def("TextBackend");
        let spec = make_spec(&trait_def, &cfg);
        let generator = make_generator();

        assert!(generator.gen_clear_fn(&spec).is_empty());
    }

    // --- None-config short-circuit (all three at once) ---------------------

    #[test]
    fn all_fns_return_empty_when_all_config_fields_none() {
        let cfg = make_bridge_config("Plugin", None, None, None);
        let trait_def = make_trait_def("Plugin");
        let spec = make_spec(&trait_def, &cfg);
        let generator = make_generator();

        assert!(generator.gen_registration_fn(&spec).is_empty());
        assert!(generator.gen_unregistration_fn(&spec).is_empty());
        assert!(generator.gen_clear_fn(&spec).is_empty());
    }

    // --- Interface conformance in registration parameter type -----------------

    #[test]
    fn registration_fn_accepts_i_trait_interface_not_raw_impl() {
        // Regression: the generated Kotlin JVM registration wrapper must accept
        // `I{TraitName}` (the hand-authored interface) as its parameter type.
        // Passing a raw object instead of the typed interface would allow
        // untyped stubs to bypass the bridge.
        let cfg = make_bridge_config("TextProcessor", Some("register_text_processor"), None, None);
        let trait_def = make_trait_def("TextProcessor");
        let spec = make_spec(&trait_def, &cfg);
        let generator = make_generator();

        let out = generator.gen_registration_fn(&spec);
        assert!(
            out.contains("impl: dev.sample_crate.ITextProcessor"),
            "registration fn must accept I{{TraitName}} interface;\nactual:\n{out}"
        );
    }
}
