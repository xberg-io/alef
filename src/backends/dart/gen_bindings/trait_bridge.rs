//! Dart-side trait bridge code generation for flutter_rust_bridge.
//!
//! Generates Dart wrapper methods that allow registering, unregistering, and clearing
//! trait implementations on the Rust side. These static methods are added to the
//! bridge class and forward to Rust-generated `pub fn` items via FRB's free-function
//! bridging mechanism.

use crate::core::config::TraitBridgeConfig;
use heck::ToLowerCamelCase;

/// Emit static wrapper methods for trait bridge registration/unregistration/clearing.
///
/// For each configured trait bridge that has `register_fn`, `unregister_fn`, or `clear_fn`
/// set, emits a corresponding static method on the Dart bridge class. These methods:
/// - Take the bridge implementation (opaque `{Trait}DartImpl` struct for register)
/// - Forward to the Rust-generated `pub fn` items via FRB's free-function mechanism
/// - Return `Future<void>` for async trait bridge operations
///
/// The Rust side generates:
/// - `pub fn register_ocr_backend(impl: OcrBackendDartImpl) -> Result<(), String>`
/// - `pub fn unregister_ocr_backend(name: String) -> Result<(), String>`
/// - `pub fn clear_ocr_backends() -> Result<(), String>`
///
/// FRB auto-bridges these as free Dart functions, and we wrap them here as static
/// methods for discoverability and a unified bridge-class interface.
pub(super) fn emit_trait_bridge_methods(bridge_cfg: &TraitBridgeConfig, out: &mut String) {
    let trait_name = &bridge_cfg.trait_name;
    let impl_type = format!("{trait_name}DartImpl");

    if let Some(register_fn) = bridge_cfg.register_fn.as_deref() {
        let dart_name = register_fn.to_lower_camel_case();
        out.push_str(&crate::backends::dart::template_env::render(
            "dart_trait_register_method.jinja",
            minijinja::context! {
                trait_name => trait_name.as_str(),
                dart_name => dart_name.as_str(),
                impl_type => impl_type.as_str(),
            },
        ));
    }

    if let Some(unregister_fn) = bridge_cfg.unregister_fn.as_deref() {
        let dart_name = unregister_fn.to_lower_camel_case();
        out.push_str(&crate::backends::dart::template_env::render(
            "dart_trait_unregister_method.jinja",
            minijinja::context! {
                trait_name => trait_name.as_str(),
                dart_name => dart_name.as_str(),
            },
        ));
    }

    if let Some(clear_fn) = bridge_cfg.clear_fn.as_deref() {
        let dart_name = clear_fn.to_lower_camel_case();
        out.push_str(&crate::backends::dart::template_env::render(
            "dart_trait_clear_method.jinja",
            minijinja::context! {
                trait_name => trait_name.as_str(),
                dart_name => dart_name.as_str(),
            },
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_emit_trait_bridge_methods_register_only() {
        let cfg = TraitBridgeConfig {
            trait_name: "OcrBackend".to_string(),
            register_fn: Some("register_ocr_backend".to_string()),
            unregister_fn: None,
            clear_fn: None,
            ..Default::default()
        };

        let mut out = String::new();
        emit_trait_bridge_methods(&cfg, &mut out);

        assert!(out.contains("registerOcrBackend"));
        assert!(!out.contains("unregisterOcrBackend"));
        assert!(!out.contains("clearOcrBackends"));
    }

    #[test]
    fn test_emit_trait_bridge_methods_all_three() {
        let cfg = TraitBridgeConfig {
            trait_name: "PostProcessor".to_string(),
            register_fn: Some("register_post_processor".to_string()),
            unregister_fn: Some("unregister_post_processor".to_string()),
            clear_fn: Some("clear_post_processors".to_string()),
            ..Default::default()
        };

        let mut out = String::new();
        emit_trait_bridge_methods(&cfg, &mut out);

        assert!(out.contains("registerPostProcessor"));
        assert!(out.contains("unregisterPostProcessor"));
        assert!(out.contains("clearPostProcessors"));
    }

    #[test]
    fn test_trait_name_to_dart_convention() {
        let dart_name = "register_ocr_backend".to_lower_camel_case();
        assert_eq!(dart_name, "registerOcrBackend");

        let dart_name2 = "unregister_post_processor".to_lower_camel_case();
        assert_eq!(dart_name2, "unregisterPostProcessor");

        let dart_name3 = "clear_validators".to_lower_camel_case();
        assert_eq!(dart_name3, "clearValidators");
    }
}
