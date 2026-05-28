//! Service-API codegen for the Kotlin backend.
//!
//! Generates idiomatic Kotlin source that declares `external` native methods matching
//! the JNI contract and wraps them with Kotlin lambda types for service registration
//! and entrypoint invocation.
//!
//! For each [`ServiceDef`]:
//! - A Kotlin class wrapping the opaque `long` owner handle (constructor, close(), Closeable)
//! - For each [`RegistrationDef`]: a registration method accepting a Kotlin lambda (handler)
//! - For each [`EntrypointDef`]: a method calling the native entrypoint
//! - Private `external fun` declarations matching JNI-mangled names

use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{ApiSurface, EntrypointKind, ServiceDef, TypeRef};
use crate::core::jni::{bridge_method_name, jni_package, service_bridge_class_name};
use heck::{ToLowerCamelCase, ToUpperCamelCase};
use std::path::PathBuf;

// ──────────────────────────────────────────────────────────────── helpers ──

/// Emit external fun declarations inside the service bridge object.
fn emit_service_bridge_externals(out: &mut String, service: &ServiceDef) {
    // Constructor: uses bridge_method_name for naming consistency
    let ctor_method = bridge_method_name(&service.name, "new");
    out.push_str("    /**\n");
    out.push_str("     * Allocate a new service instance via JNI.\n");
    out.push_str("     */\n");
    out.push_str(&format!("    external fun {ctor_method}(): Long\n\n"));

    // Destructor
    let dtor_method = bridge_method_name(&service.name, "free");
    out.push_str("    /**\n");
    out.push_str("     * Free the service instance via JNI.\n");
    out.push_str("     */\n");
    out.push_str(&format!("    external fun {dtor_method}(handle: Long)\n\n"));

    // Registration externals
    for reg in &service.registrations {
        // Use bridge_method_name for consistency with jni backend
        let register_method = bridge_method_name(&service.name, &format!("register_{}", reg.method));

        out.push_str("    /**\n");
        out.push_str(&format!("     * Register a handler for {} via JNI.\n", reg.method));
        out.push_str("     */\n");
        out.push_str(&format!("    external fun {register_method}(\n"));
        out.push_str("        handle: Long,\n");
        out.push_str("        handler: (String) -> String");

        // Metadata parameter types
        for meta_param in &reg.metadata_params {
            let kotlin_ty = kotlin_type_for_metadata(&meta_param.ty);
            out.push_str(&format!(",\n        {}: {}", meta_param.name, kotlin_ty));
        }

        out.push_str("\n    ): Int\n\n");
    }

    // Entrypoint externals
    for ep in &service.entrypoints {
        let ep_method = bridge_method_name(&service.name, &ep.method);
        let return_type = match ep.kind {
            EntrypointKind::Run => "Unit",
            EntrypointKind::Finalize => "Long",
        };

        out.push_str("    /**\n");
        out.push_str(&format!("     * {} the service via JNI.\n", ep.method));
        out.push_str("     */\n");
        out.push_str(&format!("    external fun {ep_method}(\n"));

        out.push_str("        handle: Long");

        for param in &ep.params {
            let kotlin_ty = kotlin_type_for_metadata(&param.ty);
            out.push_str(&format!(",\n        {}: {}", param.name, kotlin_ty));
        }

        out.push_str(&format!("\n    ): {}\n\n", return_type));
    }
}

/// Map a Kotlin type for metadata parameters.
fn kotlin_type_for_metadata(ty: &TypeRef) -> String {
    match ty {
        TypeRef::String | TypeRef::Char => "String".to_owned(),
        TypeRef::Primitive(p) => {
            use crate::core::ir::PrimitiveType;
            match p {
                PrimitiveType::Bool => "Boolean".to_owned(),
                PrimitiveType::U8 | PrimitiveType::I8 => "Byte".to_owned(),
                PrimitiveType::U16 | PrimitiveType::I16 => "Short".to_owned(),
                PrimitiveType::U32 | PrimitiveType::I32 => "Int".to_owned(),
                PrimitiveType::U64 | PrimitiveType::I64 => "Long".to_owned(),
                PrimitiveType::F32 => "Float".to_owned(),
                PrimitiveType::F64 => "Double".to_owned(),
                PrimitiveType::Usize => "Long".to_owned(),
                PrimitiveType::Isize => "Long".to_owned(),
            }
        }
        TypeRef::Bytes => "ByteArray".to_owned(),
        TypeRef::Unit => "Unit".to_owned(),
        _ => "Any".to_owned(),
    }
}

// ──────────────────────────────────────────────────────────────── Kotlin ──

/// Generate an idiomatic Kotlin service wrapper class.
///
/// Emits two components:
/// 1. A top-level `object {ServiceName}ServiceBridge` with `external fun` declarations
/// 2. A `public class {ServiceName}` wrapping the opaque handle and delegating to the bridge
fn gen_service_kotlin(_api: &ApiSurface, service: &ServiceDef, package: &str, lib_name: &str) -> String {
    let mut out = String::new();

    // File header and package
    out.push_str("// Auto-generated by alef — DO NOT EDIT\n");
    out.push('\n');
    out.push_str(&format!("package {}\n\n", package));

    // Imports
    out.push_str("import java.io.Closeable\n\n");

    // Bridge object: holds external fun declarations
    let class_name = service.name.to_upper_camel_case();
    let bridge_object_name = service_bridge_class_name(&service.name);
    out.push_str("/**\n");
    out.push_str(&format!(
        " * JNI bridge object for {} service.\n",
        service.name
    ));
    out.push_str(" *\n");
    out.push_str(" * Contains native declarations matched to JNI symbols.\n");
    out.push_str(" */\n");
    out.push_str(&format!("private object {bridge_object_name} {{\n"));
    out.push_str("    init {\n");
    out.push_str(&format!("        System.loadLibrary(\"{lib_name}\")\n"));
    out.push_str("    }\n\n");

    // Emit external funs in the bridge object (placed inline here; detailed below)
    emit_service_bridge_externals(&mut out, service);

    out.push_str("}\n\n");

    // Class declaration
    out.push_str("/**\n");
    out.push_str(&format!(" * Service wrapper for {}.\n", service.name));
    out.push_str(" *\n");
    out.push_str(" * Wraps an opaque native owner handle and provides type-safe registration\n");
    out.push_str(" * and entrypoint methods.\n");
    out.push_str(" */\n");
    out.push_str(&format!("public class {}(\n", class_name));
    out.push_str("    private var handle: Long = 0L,\n");
    out.push_str(") : Closeable {\n\n");

    // Constructor
    let ctor_method = bridge_method_name(&service.name, "new");
    out.push_str("    /**\n");
    out.push_str(&format!("     * Allocate a new {} instance.\n", service.name));
    out.push_str("     */\n");
    out.push_str("    init {\n");
    out.push_str(&format!(
        "        handle = {bridge_object_name}.{ctor_method}()\n"
    ));
    out.push_str("    }\n\n");

    // Closeable impl: destructor
    let dtor_method = bridge_method_name(&service.name, "free");
    out.push_str("    /**\n");
    out.push_str("     * Free the native owner.\n");
    out.push_str("     */\n");
    out.push_str("    override fun close() {\n");
    out.push_str("        if (handle != 0L) {\n");
    out.push_str(&format!(
        "            {bridge_object_name}.{dtor_method}(handle)\n"
    ));
    out.push_str("            handle = 0L\n");
    out.push_str("        }\n");
    out.push_str("    }\n\n");

    // Registration methods
    for reg in &service.registrations {
        let reg_method = &reg.method;
        // Use bridge_method_name for consistency with the bridge object externals
        let native_register_name = bridge_method_name(&service.name, &format!("register_{}", reg_method));

        out.push_str("    /**\n");
        out.push_str(&format!("     * Register a handler for {}.\n", reg_method));
        out.push_str("     *\n");
        out.push_str("     * @param handler A lambda accepting a request and returning a response\n");

        // Document metadata params
        for meta_param in &reg.metadata_params {
            out.push_str(&format!("     * @param {} Metadata: {}\n", meta_param.name, meta_param.name));
        }

        out.push_str("     * @return 0 on success, non-zero error code on failure\n");
        out.push_str("     */\n");

        out.push_str(&format!("    fun {}(\n", reg_method));
        out.push_str("        handler: (String) -> String");

        // Metadata parameters
        for meta_param in &reg.metadata_params {
            let kotlin_ty = kotlin_type_for_metadata(&meta_param.ty);
            let param_name = meta_param.name.to_lower_camel_case();
            out.push_str(&format!(",\n        {}: {}", param_name, kotlin_ty));
        }

        out.push_str("\n    ): Int {\n");
        out.push_str(&format!("        return {bridge_object_name}.{native_register_name}(\n"));
        out.push_str("            handle,\n");
        out.push_str("            handler,\n");

        // Pass metadata args to native call
        let mut first = true;
        for meta_param in &reg.metadata_params {
            if !first {
                out.push_str(",\n");
            }
            let param_name = meta_param.name.to_lower_camel_case();
            out.push_str(&format!("            {}", param_name));
            first = false;
        }

        out.push_str("\n        )\n");
        out.push_str("    }\n\n");
    }

    // Entrypoint methods
    for ep in &service.entrypoints {
        let ep_method = &ep.method;

        out.push_str("    /**\n");
        out.push_str(&format!("     * {}.\n", ep_method));
        out.push_str("     *\n");

        // Document parameters
        for param in &ep.params {
            out.push_str(&format!("     * @param {} {}\n", param.name, param.name));
        }

        match ep.kind {
            EntrypointKind::Run => {
                out.push_str("     */\n");
            }
            EntrypointKind::Finalize => {
                out.push_str("     * @return Result from finalize\n");
                out.push_str("     */\n");
            }
        }

        let return_type = match ep.kind {
            EntrypointKind::Run => "Unit".to_owned(),
            EntrypointKind::Finalize => "Long".to_owned(),
        };

        let params_str = ep
            .params
            .iter()
            .map(|param| {
                format!(
                    "{}: {}",
                    param.name.to_lower_camel_case(),
                    kotlin_type_for_metadata(&param.ty)
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        out.push_str(&format!("    fun {}({}): {} {{\n", ep_method, params_str, return_type));

        // Call native entrypoint via bridge object
        let ep_method_name = bridge_method_name(&service.name, &ep.method);
        match ep.kind {
            EntrypointKind::Run => {
                out.push_str(&format!(
                    "        {bridge_object_name}.{ep_method_name}(\n"
                ));
                out.push_str("            handle");

                for param in &ep.params {
                    let param_name = param.name.to_lower_camel_case();
                    out.push_str(&format!(",\n            {}", param_name));
                }

                out.push_str("\n        )\n");
            }
            EntrypointKind::Finalize => {
                out.push_str(&format!(
                    "        return {bridge_object_name}.{ep_method_name}(\n"
                ));
                out.push_str("            handle");

                for param in &ep.params {
                    let param_name = param.name.to_lower_camel_case();
                    out.push_str(&format!(",\n            {}", param_name));
                }

                out.push_str("\n        )\n");
            }
        }

        out.push_str("    }\n\n");
    }

    out.push_str("}\n");

    out
}

// ──────────────────────────────────────────────────────────────── public ──

/// Generate all service-API files for the Kotlin backend.
///
/// Returns one `GeneratedFile` per non-empty service list:
/// - `packages/kotlin/{ServiceName}.kt`
pub fn generate(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    if api.services.is_empty() {
        return Ok(vec![]);
    }

    // Use the same package resolver as the jni backend so the Kotlin `external fun`
    // declarations and the jni `Java_*` symbols agree.
    let package = jni_package(config);
    let lib_name = config.jni_lib_name();
    let output_dir = config
        .output_paths
        .get("kotlin")
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "packages/kotlin/".to_owned());

    let base_path = PathBuf::from(&output_dir).join(package.replace('.', "/"));

    let mut files = Vec::new();

    for service in &api.services {
        let kotlin = gen_service_kotlin(api, service, &package, &lib_name);
        let class_name = service.name.to_upper_camel_case();
        files.push(GeneratedFile {
            path: base_path.join(format!("{}.kt", class_name)),
            content: kotlin,
            generated_header: false,
        });
    }

    Ok(files)
}

// ─────────────────────────────────────────────────────────────── tests ──

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{
        EntrypointDef, EntrypointKind, HandlerContractDef, MethodDef, ParamDef, RegistrationDef, ServiceDef, TypeRef,
    };

    fn make_fixture_surface() -> ApiSurface {
        let constructor = MethodDef {
            name: "new".to_owned(),
            params: vec![],
            return_type: TypeRef::Unit,
            is_async: false,
            is_static: true,
            error_type: None,
            doc: "Create a new service owner.".to_owned(),
            receiver: None,
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
        };

        let registration = RegistrationDef {
            method: "add_handler".to_owned(),
            callback_param: "handler".to_owned(),
            callback_contract: "RequestHandler".to_owned(),
            metadata_params: vec![ParamDef {
                name: "path".to_owned(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                ..ParamDef::default()
            }],
            receiver: Some(crate::core::ir::ReceiverKind::RefMut),
            return_type: TypeRef::Unit,
            error_type: None,
            doc: "Register a handler.".to_owned(),
        };

        let run_ep = EntrypointDef {
            method: "run".to_owned(),
            kind: EntrypointKind::Run,
            is_async: true,
            params: vec![ParamDef {
                name: "addr".to_owned(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                ..ParamDef::default()
            }],
            return_type: TypeRef::Unit,
            error_type: Some("ServiceError".to_owned()),
            doc: "Run the service.".to_owned(),
        };

        let service = ServiceDef {
            name: "TestService".to_owned(),
            rust_path: "my_crate::TestService".to_owned(),
            constructor,
            configurators: vec![],
            registrations: vec![registration],
            entrypoints: vec![run_ep],
            doc: "A test service.".to_owned(),
            cfg: None,
        };

        let dispatch_method = MethodDef {
            name: "handle".to_owned(),
            params: vec![ParamDef {
                name: "request".to_owned(),
                ty: TypeRef::Named("RequestData".to_owned()),
                optional: false,
                default: None,
                ..ParamDef::default()
            }],
            return_type: TypeRef::Named("ResponseData".to_owned()),
            is_async: true,
            is_static: false,
            error_type: Some("HandlerError".to_owned()),
            doc: "Dispatch a request.".to_owned(),
            receiver: Some(crate::core::ir::ReceiverKind::Ref),
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
        };

        let contract = HandlerContractDef {
            trait_name: "RequestHandler".to_owned(),
            rust_path: "my_crate::RequestHandler".to_owned(),
            dispatch: dispatch_method,
            optional_methods: vec![],
            wire_request_type: Some("RequestData".to_owned()),
            wire_response_type: Some("ResponseData".to_owned()),
            doc: "Handler contract.".to_owned(),
        };

        ApiSurface {
            crate_name: "my_crate".to_owned(),
            version: "0.1.0".to_owned(),
            services: vec![service],
            handler_contracts: vec![contract],
            ..ApiSurface::default()
        }
    }

    #[test]
    fn gen_service_kotlin_contains_class() {
        let api = make_fixture_surface();
        let service = &api.services[0];
        let kotlin = gen_service_kotlin(&api, service, "com.example", "demo_jni");

        assert!(kotlin.contains("package com.example"));
        assert!(kotlin.contains("class TestService("));
        assert!(kotlin.contains("private var handle: Long"));
    }

    #[test]
    fn gen_service_kotlin_declares_external_constructor() {
        let api = make_fixture_surface();
        let service = &api.services[0];
        let kotlin = gen_service_kotlin(&api, service, "com.example", "demo_jni");

        assert!(kotlin.contains("external fun nativeTestServiceNew()"));
        assert!(kotlin.contains(": Long"));
    }

    #[test]
    fn gen_service_kotlin_declares_external_destructor() {
        let api = make_fixture_surface();
        let service = &api.services[0];
        let kotlin = gen_service_kotlin(&api, service, "com.example", "demo_jni");

        assert!(kotlin.contains("external fun nativeTestServiceFree(handle: Long)"));
    }

    #[test]
    fn gen_service_kotlin_implements_closeable() {
        let api = make_fixture_surface();
        let service = &api.services[0];
        let kotlin = gen_service_kotlin(&api, service, "com.example", "demo_jni");

        assert!(kotlin.contains(": Closeable"));
        assert!(kotlin.contains("override fun close()"));
        assert!(kotlin.contains("nativeTestServiceFree(handle)"));
    }

    #[test]
    fn gen_service_kotlin_registration_method_accepts_lambda() {
        let api = make_fixture_surface();
        let service = &api.services[0];
        let kotlin = gen_service_kotlin(&api, service, "com.example", "demo_jni");

        // Registration method name matches
        assert!(kotlin.contains("fun add_handler("));

        // Handler parameter is a lambda type
        assert!(kotlin.contains("handler: (String) -> String"));

        // Metadata param
        assert!(kotlin.contains("path: String"));
    }

    #[test]
    fn gen_service_kotlin_registration_calls_native() {
        let api = make_fixture_surface();
        let service = &api.services[0];
        let kotlin = gen_service_kotlin(&api, service, "com.example", "demo_jni");

        // Registration calls the native register function with exact name
        assert!(kotlin.contains("nativeTestServiceRegisterAddHandler("));

        // Passes handle, handler lambda, and metadata
        assert!(kotlin.contains("handle,"));
        assert!(kotlin.contains("handler,"));
        assert!(kotlin.contains("path"));
    }

    #[test]
    fn gen_service_kotlin_declares_external_register() {
        let api = make_fixture_surface();
        let service = &api.services[0];
        let kotlin = gen_service_kotlin(&api, service, "com.example", "demo_jni");

        // Native register declaration
        assert!(kotlin.contains("external fun nativeTestServiceRegisterAddHandler("));
        assert!(kotlin.contains("handle: Long"));
        assert!(kotlin.contains("handler: (String) -> String"));
        assert!(kotlin.contains("path: String"));
        assert!(kotlin.contains("): Int"));
    }

    #[test]
    fn gen_service_kotlin_entrypoint_method() {
        let api = make_fixture_surface();
        let service = &api.services[0];
        let kotlin = gen_service_kotlin(&api, service, "com.example", "demo_jni");

        // Entrypoint method signature
        assert!(kotlin.contains("fun run("));
        assert!(kotlin.contains("addr: String"));
        assert!(kotlin.contains("): Unit"));
    }

    #[test]
    fn gen_service_kotlin_entrypoint_calls_native() {
        let api = make_fixture_surface();
        let service = &api.services[0];
        let kotlin = gen_service_kotlin(&api, service, "com.example", "demo_jni");

        // Entrypoint calls native run
        assert!(kotlin.contains("nativeTestServiceRun("));
        assert!(kotlin.contains("handle,"));
        assert!(kotlin.contains("addr"));
    }

    #[test]
    fn gen_service_kotlin_declares_external_run() {
        let api = make_fixture_surface();
        let service = &api.services[0];
        let kotlin = gen_service_kotlin(&api, service, "com.example", "demo_jni");

        // Native run declaration
        assert!(kotlin.contains("external fun nativeTestServiceRun("));
        assert!(kotlin.contains("handle: Long"));
        assert!(kotlin.contains("addr: String"));
        assert!(kotlin.contains("): Unit"));
    }

    #[test]
    fn gen_service_kotlin_loads_native_library() {
        let api = make_fixture_surface();
        let service = &api.services[0];
        let kotlin = gen_service_kotlin(&api, service, "com.example", "demo_jni");

        assert!(kotlin.contains("System.loadLibrary"));
        assert!(kotlin.contains("demo_jni"));
    }

    #[test]
    fn gen_service_kotlin_has_no_stubs() {
        let api = make_fixture_surface();
        let service = &api.services[0];
        let kotlin = gen_service_kotlin(&api, service, "com.example", "demo_jni");

        assert!(!kotlin.contains("TODO"));
        assert!(!kotlin.contains("stub"));
        assert!(!kotlin.contains("placeholder"));
    }

    #[test]
    fn generate_returns_files() {
        let api = make_fixture_surface();
        let config = ResolvedCrateConfig {
            name: "my_crate".to_owned(),
            ..ResolvedCrateConfig::default()
        };

        let files = generate(&api, &config).expect("generate should not fail");
        assert!(!files.is_empty(), "expected at least one file");

        let has_service_file = files.iter().any(|f| f.path.to_string_lossy().contains("TestService.kt"));
        assert!(has_service_file, "expected TestService.kt in output");
    }

    #[test]
    fn generate_returns_empty_for_no_services() {
        let api = ApiSurface::default();
        let config = ResolvedCrateConfig {
            name: "my_crate".to_owned(),
            ..ResolvedCrateConfig::default()
        };

        let files = generate(&api, &config).expect("generate should not fail");
        assert!(files.is_empty(), "expected no files for surface without services");
    }

    #[test]
    fn gen_service_kotlin_finalize_entrypoint() {
        let mut api = make_fixture_surface();

        // Add a finalize entrypoint
        let finalize_ep = EntrypointDef {
            method: "shutdown".to_owned(),
            kind: EntrypointKind::Finalize,
            is_async: true,
            params: vec![],
            return_type: TypeRef::Primitive(crate::core::ir::PrimitiveType::I32),
            error_type: None,
            doc: "Shutdown the service.".to_owned(),
        };

        api.services[0].entrypoints.push(finalize_ep);

        let service = &api.services[0];
        let kotlin = gen_service_kotlin(&api, service, "com.example", "demo_jni");

        // Verify finalize method signature
        assert!(kotlin.contains("fun shutdown()"));
        assert!(kotlin.contains("): Long"));

        // Verify native finalize call
        assert!(kotlin.contains("nativeTestServiceShutdown("));

        // Verify external finalize declaration
        assert!(kotlin.contains("external fun nativeTestServiceShutdown("));
        assert!(kotlin.contains("): Long"));
    }
}
