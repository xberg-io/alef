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

/// Generate the complete trait bridge (bridge object + JNI dispatcher) for Kotlin Android.
///
/// For each bridge in the config:
/// - The interface is generated elsewhere by `emit_trait_interfaces` in gen_bindings.rs
/// - This function generates the bridge object with registration/unregistration methods
/// - This function generates the JNI dispatcher that wraps user impls for native dispatch
///
/// Returns a list of (filename, content) tuples ready for GeneratedFile emission.
pub fn gen_trait_bridge_files(
    package: &str,
    trait_name: &str,
    bridge_cfg: &TraitBridgeConfig,
    trait_def: &TypeDef,
    bridge_class_name: &str,
    api: &crate::core::ir::ApiSurface,
    effective_excluded_types: &std::collections::HashSet<String>,
) -> Vec<(String, String)> {
    let interface_name = format!("I{trait_name}");
    let bridge_obj = bridge_object_name(trait_name);

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
                dispatcher_class => format!("{trait_name}JniDispatcher"),
            },
        );

        let content = assemble_kt_content(package, &BTreeSet::new(), &body);
        files.push((format!("{bridge_obj}.kt"), content));
    }

    // Emit the JNI dispatcher: the JSON entry point the native trait bridge calls.
    // Suspend interface methods cannot be invoked over raw JNI (they need a
    // continuation), so the dispatcher bridges them with runBlocking.
    {
        use heck::ToLowerCamelCase;

        let visible_type_names: std::collections::HashSet<&str> = api
            .types
            .iter()
            .filter(|t| !t.binding_excluded && !effective_excluded_types.contains(&t.name))
            .map(|t| t.name.as_str())
            .chain(api.enums.iter().map(|e| e.name.as_str()))
            .collect();

        let mut imports = BTreeSet::new();
        imports.insert("com.fasterxml.jackson.module.kotlin.jacksonObjectMapper".to_string());
        let mut any_async = false;

        let own_methods: Vec<&crate::core::ir::MethodDef> = trait_def
            .methods
            .iter()
            .filter(|m| !m.sanitized && !m.is_static)
            .collect();

        let methods: Vec<minijinja::Value> = own_methods
            .iter()
            .map(|method| {
                let method_camel = method.name.to_lower_camel_case();
                let args = method
                    .params
                    .iter()
                    .map(|p| {
                        let ty_ref = substitute_trait_carrier_type(api, bridge_cfg, &p.ty);
                        let ty = kotlin_type_str_visible(&ty_ref, p.optional, &visible_type_names, &mut imports);
                        format!(
                            "mapper.convertValue(args.get(\"{}\"), object : com.fasterxml.jackson.core.type.TypeReference<{}>() {{}})",
                            p.name, ty
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                let call = format!("impl.{method_camel}({args})");
                let call = if method.is_async {
                    any_async = true;
                    format!("runBlocking {{ {call} }}")
                } else {
                    call
                };
                let call_expr = if matches!(method.return_type, crate::core::ir::TypeRef::Unit) {
                    format!("{{
                        {call}
                        null
                    }}")
                } else {
                    call
                };
                minijinja::context! {
                    rust_name => &method.name,
                    call_expr => call_expr,
                }
            })
            .collect();

        if any_async {
            imports.insert("kotlinx.coroutines.runBlocking".to_string());
        }

        // The interface currently makes every trait method abstract, so a host
        // provides all of them; lifecycle hooks are interface defaults and always
        // dispatchable. When interface defaults land for Rust-defaulted methods,
        // this switches to reflection-based overridden detection.
        let mut implemented: Vec<&str> = own_methods.iter().map(|m| m.name.as_str()).collect();
        if bridge_cfg.super_trait.is_some() {
            implemented.extend(["name", "version", "initialize", "shutdown"]);
        }

        let body = template_env::render(
            "trait_bridge_dispatcher.jinja",
            minijinja::context! {
                trait_name => trait_name,
                has_super_trait => bridge_cfg.super_trait.is_some(),
                methods => methods,
                implemented => implemented,
            },
        );

        let mut final_imports = BTreeSet::new();
        for import_line in &imports {
            if let Some(stripped) = import_line.strip_prefix("import ") {
                final_imports.insert(stripped.to_string());
            } else {
                final_imports.insert(import_line.clone());
            }
        }
        let content = assemble_kt_content(package, &final_imports, &body);
        files.push((format!("{trait_name}JniDispatcher.kt"), content));
    }

    files
}

/// Generate the bridge object (legacy entry point for compatibility).
/// This is used primarily by tests and creates an empty ApiSurface for backward compatibility.
pub fn gen_trait_bridge_object(
    package: &str,
    trait_name: &str,
    bridge_cfg: &TraitBridgeConfig,
    trait_def: &TypeDef,
    bridge_class_name: &str,
) -> Option<(String, String)> {
    use crate::core::ir::ApiSurface;

    // Create an empty ApiSurface for backward-compatible test calls
    let api = ApiSurface {
        crate_name: "test_crate".to_string(),
        version: "0.0.0".to_string(),
        types: vec![],
        enums: vec![],
        functions: vec![],
        errors: vec![],
        excluded_type_paths: std::collections::HashMap::new(),
        excluded_trait_names: std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: vec![],
    };
    let excluded_types = std::collections::HashSet::new();

    let files = gen_trait_bridge_files(
        package,
        trait_name,
        bridge_cfg,
        trait_def,
        bridge_class_name,
        &api,
        &excluded_types,
    );
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

/// Map a `TypeRef` to its Kotlin representation, substituting `String` for any
/// `Named` type that is not in the set of visible (generated) types.
/// This prevents excluded/internal types like `InternalDocument` from appearing
/// in trait interface signatures where they are not defined.
///
/// Mirrors the logic from emit_trait_methods in gen_bindings.rs.
fn kotlin_type_str_visible(
    ty: &crate::core::ir::TypeRef,
    optional: bool,
    visible_type_names: &std::collections::HashSet<&str>,
    imports: &mut BTreeSet<String>,
) -> String {
    use crate::backends::kotlin::kotlin_type_str_pub;

    match ty {
        crate::core::ir::TypeRef::Named(name) if !visible_type_names.contains(name.as_str()) => {
            if optional {
                "String?".to_string()
            } else {
                "String".to_string()
            }
        }
        crate::core::ir::TypeRef::Optional(inner) => kotlin_type_str_visible(inner, true, visible_type_names, imports),
        other => kotlin_type_str_pub(other, optional, imports),
    }
}

/// Substitute trait carrier types for named types that are context or result types.
/// This mirrors the logic from emit_trait_methods in gen_bindings.rs.
fn substitute_trait_carrier_type(
    api: &crate::core::ir::ApiSurface,
    bridge: &TraitBridgeConfig,
    ty: &crate::core::ir::TypeRef,
) -> crate::core::ir::TypeRef {
    use crate::core::ir::TypeRef;

    match ty {
        TypeRef::Named(name) if should_project_trait_carrier(api, bridge, name) => TypeRef::Named(
            bridge
                .result_type
                .as_ref()
                .expect("checked by should_project_trait_carrier")
                .clone(),
        ),
        TypeRef::Optional(inner) => TypeRef::Optional(Box::new(substitute_trait_carrier_type(api, bridge, inner))),
        TypeRef::Vec(inner) => TypeRef::Vec(Box::new(substitute_trait_carrier_type(api, bridge, inner))),
        TypeRef::Map(key, value) => TypeRef::Map(
            Box::new(substitute_trait_carrier_type(api, bridge, key)),
            Box::new(substitute_trait_carrier_type(api, bridge, value)),
        ),
        other => other.clone(),
    }
}

/// Check if a type should be projected to a trait carrier type.
/// This mirrors the logic from emit_trait_methods in gen_bindings.rs.
fn should_project_trait_carrier(
    api: &crate::core::ir::ApiSurface,
    bridge: &TraitBridgeConfig,
    type_name: &str,
) -> bool {
    bridge.context_type.as_deref() == Some(type_name)
        && bridge.result_type.is_some()
        && (api.excluded_type_paths.contains_key(type_name)
            || api
                .types
                .iter()
                .any(|typ| typ.name == type_name && (typ.binding_excluded || typ.is_opaque)))
}

#[cfg(test)]
fn tests_support_bridge_cfg(trait_name: &str, super_trait: Option<&str>) -> TraitBridgeConfig {
    use heck::ToSnakeCase;
    TraitBridgeConfig {
        trait_name: trait_name.to_string(),
        super_trait: super_trait.map(|s| s.to_string()),
        register_fn: Some(format!("register_{}", trait_name.to_snake_case())),
        unregister_fn: Some(format!("unregister_{}", trait_name.to_snake_case())),
        clear_fn: Some(format!("clear_{}", trait_name.to_snake_case())),
        bind_via: crate::core::config::BridgeBinding::FunctionParam,
        ..TraitBridgeConfig::default()
    }
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
            has_private_fields: false,
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
        assert!(content.contains("TestBridge.nativeRegisterOcrBackend(OcrBackendJniDispatcher(impl))"));
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
        assert!(content.contains("TestBridge.nativeRegisterOcrBackend(OcrBackendJniDispatcher(impl), name)"));
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

#[cfg(test)]
mod dispatcher_tests {
    use super::*;
    use crate::core::ir::{MethodDef, ParamDef, PrimitiveType, ReceiverKind, TypeRef};

    fn make_api(trait_def: &TypeDef) -> crate::core::ir::ApiSurface {
        crate::core::ir::ApiSurface {
            crate_name: "testcrate".into(),
            version: "0.1.0".into(),
            types: vec![trait_def.clone()],
            ..Default::default()
        }
    }

    fn ocr_like_trait() -> TypeDef {
        TypeDef {
            name: "OcrBackend".into(),
            rust_path: "testcrate::OcrBackend".into(),
            is_trait: true,
            is_opaque: true,
            methods: vec![
                MethodDef {
                    name: "process_image".into(),
                    params: vec![
                        ParamDef {
                            name: "image_bytes".into(),
                            ty: TypeRef::Bytes,
                            is_ref: true,
                            ..Default::default()
                        },
                        ParamDef {
                            name: "config".into(),
                            ty: TypeRef::Named("OcrConfig".into()),
                            is_ref: true,
                            ..Default::default()
                        },
                    ],
                    return_type: TypeRef::Named("ExtractedDocument".into()),
                    is_async: true,
                    receiver: Some(ReceiverKind::Ref),
                    error_type: Some("Error".into()),
                    ..Default::default()
                },
                MethodDef {
                    name: "supports_language".into(),
                    params: vec![ParamDef {
                        name: "lang".into(),
                        ty: TypeRef::String,
                        is_ref: true,
                        ..Default::default()
                    }],
                    return_type: TypeRef::Primitive(PrimitiveType::Bool),
                    receiver: Some(ReceiverKind::Ref),
                    ..Default::default()
                },
                MethodDef {
                    name: "supports_tables".into(),
                    params: vec![],
                    return_type: TypeRef::Primitive(PrimitiveType::Bool),
                    receiver: Some(ReceiverKind::Ref),
                    has_default_impl: true,
                    ..Default::default()
                },
            ],
            ..Default::default()
        }
    }

    #[test]
    fn dispatcher_file_emitted_with_json_dispatch_and_run_blocking() {
        let trait_def = ocr_like_trait();
        let bridge_cfg =
            crate::backends::kotlin_android::trait_bridge::tests_support_bridge_cfg("OcrBackend", Some("Plugin"));
        let api = make_api(&trait_def);
        let files = gen_trait_bridge_files(
            "dev.sample_crate",
            "OcrBackend",
            &bridge_cfg,
            &trait_def,
            "DemoBridge",
            &api,
            &std::collections::HashSet::new(),
        );
        let (_, dispatcher) = files
            .iter()
            .find(|(name, _)| name == "OcrBackendJniDispatcher.kt")
            .expect("dispatcher file must be emitted");

        assert!(
            dispatcher.contains("fun dispatch(method: String, argsJson: String): String"),
            "dispatcher must expose the JSON dispatch entry point:\n{dispatcher}"
        );
        assert!(
            dispatcher.contains("runBlocking"),
            "suspend trait methods must be bridged via runBlocking:\n{dispatcher}"
        );
        assert!(
            dispatcher.contains("\"process_image\" ->") && dispatcher.contains("\"supports_language\" ->"),
            "dispatch must switch on the Rust method names:\n{dispatcher}"
        );
        assert!(
            dispatcher.contains("\"initialize\" ->") && dispatcher.contains("\"shutdown\" ->"),
            "lifecycle hooks must be dispatchable:\n{dispatcher}"
        );
        assert!(
            dispatcher.contains("fun implementedMethods(): String"),
            "dispatcher must report the implemented method names:\n{dispatcher}"
        );
    }

    #[test]
    fn bridge_object_registers_dispatcher_wrapper() {
        let trait_def = ocr_like_trait();
        let bridge_cfg =
            crate::backends::kotlin_android::trait_bridge::tests_support_bridge_cfg("OcrBackend", Some("Plugin"));
        let api = make_api(&trait_def);
        let files = gen_trait_bridge_files(
            "dev.sample_crate",
            "OcrBackend",
            &bridge_cfg,
            &trait_def,
            "DemoBridge",
            &api,
            &std::collections::HashSet::new(),
        );
        let (_, bridge_obj) = files
            .iter()
            .find(|(name, _)| name.ends_with("Bridge.kt"))
            .expect("bridge object file");
        assert!(
            bridge_obj.contains("nativeRegisterOcrBackend(OcrBackendJniDispatcher(impl))"),
            "register must wrap the impl in the JNI dispatcher:\n{bridge_obj}"
        );
    }
}
