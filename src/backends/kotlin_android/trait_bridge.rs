//! Kotlin user-facing trait bridge support.
//!
//! For Kotlin Android backends, this module generates:
//! 1. `interface I{TraitName}` — Kotlin interface with Plugin lifecycle + trait methods
//! 2. `object {TraitName}Bridge` — registration/unregistration wrapper that:
//!    - Stores registered impls in a static map
//!    - Calls native JNI methods for registration/unregistration
//!    - Throws SampleCrateException on failure

use crate::backends::kotlin_android::naming::bridge_object_name;
use crate::backends::kotlin_android::template_env;
use crate::core::config::TraitBridgeConfig;
use crate::core::ir::TypeDef;
use heck::ToUpperCamelCase;
use std::collections::BTreeSet;

/// Generate the complete trait bridge (interface + bridge object + adapter) for Kotlin Android.
///
/// For each bridge in the config:
/// - The interface is generated elsewhere by `emit_trait_interfaces` in gen_bindings.rs
/// - This function generates the bridge object with registration/unregistration methods
/// - This function generates the adapter class that wraps user impls for JNI invocation
///
/// Returns a list of (filename, content) tuples ready for GeneratedFile emission.
pub fn gen_trait_bridge_files(
    package: &str,
    trait_name: &str,
    bridge_cfg: &TraitBridgeConfig,
    trait_def: &TypeDef,
    bridge_class_name: &str,
) -> Vec<(String, String)> {
    let interface_name = format!("I{trait_name}");
    let bridge_obj = bridge_object_name(trait_name);
    let adapter_class_name = format!("{}Adapter", trait_name);

    let mut files = Vec::new();

    // Emit bridge object
    {
        let has_super_trait = bridge_cfg.super_trait.is_some();
        let register_native_fn = format!("nativeRegister{}", trait_name.to_upper_camel_case());
        let unregister_native_fn = bridge_cfg
            .unregister_fn
            .as_ref()
            .map(|_| format!("nativeUnregister{}", trait_name.to_upper_camel_case()));
        let clear_native_fn = bridge_cfg
            .clear_fn
            .as_ref()
            .map(|_| format!("nativeClear{}s", trait_name.to_upper_camel_case()));
        let body = template_env::render(
            "trait_bridge_object.jinja",
            minijinja::context! {
                bridge_obj => bridge_obj,
                interface_name => interface_name,
                bridge_class_name => bridge_class_name,
                has_super_trait => has_super_trait,
                register_native_fn => register_native_fn,
                unregister_native_fn => unregister_native_fn,
                clear_native_fn => clear_native_fn,
            },
        );

        let content = assemble_kt_content(package, &BTreeSet::new(), &body);
        files.push((format!("{bridge_obj}.kt"), content));
    }

    // Emit adapter class
    {
        use heck::ToLowerCamelCase;

        let mut adapter_methods = String::new();

        // Delegate Plugin super-trait methods if present
        adapter_methods.push_str("    override fun name(): String = impl.name()\n\n");
        adapter_methods.push_str("    override fun version(): String = impl.version()\n\n");
        adapter_methods.push_str("    override fun initialize() = impl.initialize()\n\n");
        adapter_methods.push_str("    override fun shutdown() = impl.shutdown()\n\n");

        // Delegate all trait methods
        for method in &trait_def.methods {
            let method_camel = method.name.to_lower_camel_case();
            let params: Vec<String> = method
                .params
                .iter()
                .map(|p| {
                    let pname = p.name.to_lower_camel_case();
                    format!(
                        "{pname}: {}",
                        match &p.ty {
                            crate::core::ir::TypeRef::String => "String",
                            crate::core::ir::TypeRef::Primitive(_) => "String",
                            crate::core::ir::TypeRef::Vec(_) => "String",
                            crate::core::ir::TypeRef::Named(_) => "String",
                            _ => "Any",
                        }
                    )
                })
                .collect();
            let params_str = params.join(", ");
            let delegate_args = method
                .params
                .iter()
                .map(|p| p.name.to_lower_camel_case())
                .collect::<Vec<_>>()
                .join(", ");

            if method.is_async {
                adapter_methods.push_str(&format!(
                    "    override suspend fun {method_camel}({params_str}): String {{\n"
                ));
                adapter_methods.push_str(&format!(
                    "        return impl.{method_camel}({delegate_args}).toString()\n"
                ));
                adapter_methods.push_str("    }\n\n");
            } else {
                adapter_methods.push_str(&format!("    override fun {method_camel}({params_str}): String {{\n"));
                adapter_methods.push_str(&format!(
                    "        return impl.{method_camel}({delegate_args}).toString()\n"
                ));
                adapter_methods.push_str("    }\n\n");
            }
        }

        let adapter_body = template_env::render(
            "trait_bridge_adapter.jinja",
            minijinja::context! {
                interface_name => interface_name,
                adapter_class_name => adapter_class_name,
                adapter_methods => adapter_methods,
            },
        );

        let content = assemble_kt_content(package, &BTreeSet::new(), &adapter_body);
        files.push((format!("{adapter_class_name}.kt"), content));
    }

    files
}

/// Generate the bridge object (legacy entry point for compatibility).
pub fn gen_trait_bridge_object(
    package: &str,
    trait_name: &str,
    bridge_cfg: &TraitBridgeConfig,
    trait_def: &TypeDef,
    bridge_class_name: &str,
) -> Option<(String, String)> {
    let files = gen_trait_bridge_files(package, trait_name, bridge_cfg, trait_def, bridge_class_name);
    files.first().map(|(name, content)| (name.clone(), content.clone()))
}

/// Assemble Kotlin file content with package and imports.
fn assemble_kt_content(package: &str, imports: &BTreeSet<String>, body: &str) -> String {
    let mut out = String::new();
    let imports = imports
        .iter()
        .map(|import| format!("import {import}"))
        .collect::<Vec<_>>();
    out.push_str(&template_env::render(
        "kt_file.jinja",
        minijinja::context! {
            package => package,
            imports => imports,
            suppressions => Vec::<String>::new(),
            body => body,
        },
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use heck::ToSnakeCase;

    fn make_bridge_cfg(trait_name: &str, super_trait: Option<&str>) -> TraitBridgeConfig {
        TraitBridgeConfig {
            trait_name: trait_name.to_string(),
            param_name: None,
            type_alias: None,
            exclude_languages: vec![],
            super_trait: super_trait.map(|s| s.to_string()),
            registry_getter: None,
            register_fn: Some(format!("register_{}", trait_name.to_snake_case())),
            unregister_fn: Some(format!("unregister_{}", trait_name.to_snake_case())),
            clear_fn: Some(format!("clear_{}", trait_name.to_snake_case())),
            register_extra_args: None,
            bind_via: crate::core::config::BridgeBinding::FunctionParam,
            options_type: None,
            options_field: None,
            context_type: None,
            result_type: None,
            ffi_skip_methods: Vec::new(),
        }
    }

    fn make_trait_def(name: &str) -> TypeDef {
        TypeDef {
            name: name.to_string(),
            rust_path: format!("testcrate::{}", name),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
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

    #[test]
    fn test_bridge_object_with_super_trait() {
        let trait_def = make_trait_def("OcrBackend");
        let bridge_cfg = make_bridge_cfg("OcrBackend", Some("Plugin"));
        let (_filename, content) =
            gen_trait_bridge_object("dev.sample_crate", "OcrBackend", &bridge_cfg, &trait_def, "TestBridge")
                .expect("should generate bridge");

        assert!(content.contains("object OcrBackendBridge"));
        assert!(content.contains("fun register(impl: IOcrBackend): Unit"));
        assert!(content.contains("val name = impl.name()"));
        assert!(content.contains("TestBridge.nativeRegisterOcrBackend(impl)"));
        assert!(content.contains("nativeRegisterOcrBackend"));
    }

    #[test]
    fn test_bridge_object_without_super_trait() {
        let trait_def = make_trait_def("OcrBackend");
        let mut bridge_cfg = make_bridge_cfg("OcrBackend", None);
        bridge_cfg.super_trait = None;
        let (_filename, content) =
            gen_trait_bridge_object("dev.sample_crate", "OcrBackend", &bridge_cfg, &trait_def, "TestBridge")
                .expect("should generate bridge");

        assert!(content.contains("object OcrBackendBridge"));
        assert!(content.contains("fun register(impl: IOcrBackend, name: String): Unit"));
        assert!(!content.contains("val name = impl.name()"));
        assert!(content.contains("TestBridge.nativeRegisterOcrBackend(impl, name)"));
    }

    #[test]
    fn test_bridge_object_with_unregister() {
        let trait_def = make_trait_def("OcrBackend");
        let bridge_cfg = make_bridge_cfg("OcrBackend", Some("Plugin"));
        let (_filename, content) =
            gen_trait_bridge_object("dev.sample_crate", "OcrBackend", &bridge_cfg, &trait_def, "TestBridge")
                .expect("should generate bridge");

        assert!(content.contains("fun unregister(name: String): Unit"));
        assert!(content.contains("TestBridge.nativeUnregisterOcrBackend(name)"));
        assert!(content.contains("nativeUnregisterOcrBackend"));
    }

    #[test]
    fn test_bridge_object_with_clear() {
        let trait_def = make_trait_def("OcrBackend");
        let bridge_cfg = make_bridge_cfg("OcrBackend", Some("Plugin"));
        let (_filename, content) =
            gen_trait_bridge_object("dev.sample_crate", "OcrBackend", &bridge_cfg, &trait_def, "TestBridge")
                .expect("should generate bridge");

        assert!(content.contains("fun clearAll(): Unit"));
        assert!(content.contains("TestBridge.nativeClearOcrBackends()"));
        assert!(content.contains("nativeClearOcrBackends"));
    }

    #[test]
    fn test_bridge_object_without_unregister() {
        let trait_def = make_trait_def("OcrBackend");
        let mut bridge_cfg = make_bridge_cfg("OcrBackend", Some("Plugin"));
        bridge_cfg.unregister_fn = None;
        let (_filename, content) =
            gen_trait_bridge_object("dev.sample_crate", "OcrBackend", &bridge_cfg, &trait_def, "TestBridge")
                .expect("should generate bridge");

        assert!(!content.contains("fun unregister"));
    }

    #[test]
    fn test_bridge_object_without_clear() {
        let trait_def = make_trait_def("OcrBackend");
        let mut bridge_cfg = make_bridge_cfg("OcrBackend", Some("Plugin"));
        bridge_cfg.clear_fn = None;
        let (_filename, content) =
            gen_trait_bridge_object("dev.sample_crate", "OcrBackend", &bridge_cfg, &trait_def, "TestBridge")
                .expect("should generate bridge");

        assert!(!content.contains("fun clearAll"));
    }

    #[test]
    fn test_bridge_object_includes_getall() {
        let trait_def = make_trait_def("OcrBackend");
        let bridge_cfg = make_bridge_cfg("OcrBackend", Some("Plugin"));
        let (_filename, content) =
            gen_trait_bridge_object("dev.sample_crate", "OcrBackend", &bridge_cfg, &trait_def, "TestBridge")
                .expect("should generate bridge");

        assert!(content.contains("fun getAll(): Map<String, IOcrBackend>"));
    }
}
