//! Integration test: verify JVM service API layering stays consistent.
//!
//! The JVM Kotlin service backend is a coroutine-friendly wrapper over the Java/Panama
//! facade. It must not emit JNI `external fun`s; Java owns the native C FFI calls.

use alef::backends::java::JavaBackend;
use alef::backends::kotlin::KotlinBackend;
use alef::core::backend::Backend;
use alef::core::config::ResolvedCrateConfig;
use alef::core::ir::{
    ApiSurface, EntrypointDef, EntrypointKind, HandlerContractDef, MethodDef, ParamDef, PrimitiveType, RegistrationDef,
    ServiceDef, TypeRef,
};

/// Build a synthetic [`ApiSurface`] with one service: one constructor, one registration
/// with metadata, one Run entrypoint, and a corresponding handler contract.
fn make_test_surface() -> ApiSurface {
    let constructor = MethodDef {
        name: "new".to_owned(),
        params: vec![],
        return_type: TypeRef::Unit,
        is_async: false,
        is_static: true,
        error_type: None,
        doc: "Create service.".to_owned(),
        receiver: None,
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    };

    // One registration with a metadata parameter
    let registration = RegistrationDef {
        method: "on_request".to_owned(),
        callback_param: "handler".to_owned(),
        callback_contract: "RequestHandler".to_owned(),
        metadata_params: vec![ParamDef {
            name: "pattern".to_owned(),
            ty: TypeRef::String,
            optional: false,
            default: None,
            ..ParamDef::default()
        }],
        receiver: Some(alef::core::ir::ReceiverKind::RefMut),
        return_type: TypeRef::Unit,
        error_type: None,
        doc: "Register handler.".to_owned(),
        variants: vec![],
        ..Default::default()
    };

    // Run entrypoint
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
        doc: "Run service.".to_owned(),
    };

    // Finalize entrypoint
    let finalize_ep = EntrypointDef {
        method: "shutdown".to_owned(),
        kind: EntrypointKind::Finalize,
        is_async: true,
        params: vec![],
        return_type: TypeRef::Primitive(PrimitiveType::I32),
        error_type: None,
        doc: "Finalize service.".to_owned(),
    };

    let service = ServiceDef {
        name: "ApiSurface".to_owned(),
        rust_path: "test_crate::ApiSurface".to_owned(),
        constructor,
        configurators: vec![],
        registrations: vec![registration],
        entrypoints: vec![run_ep, finalize_ep],
        doc: "Test service.".to_owned(),
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
        doc: "Dispatch.".to_owned(),
        receiver: Some(alef::core::ir::ReceiverKind::Ref),
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    };

    let contract = HandlerContractDef {
        trait_name: "RequestHandler".to_owned(),
        rust_path: "test_crate::RequestHandler".to_owned(),
        dispatch: dispatch_method,
        optional_methods: vec![],
        wire_request_type: Some("RequestData".to_owned()),
        wire_response_type: Some("ResponseData".to_owned()),
        dispatch_extra_params: vec![],
        wire_param_name: None,
        dispatch_return_type: None,
        response_adapter: None,
        doc: "Handler contract.".to_owned(),
    };

    ApiSurface {
        crate_name: "test_crate".to_owned(),
        version: "0.1.0".to_owned(),
        services: vec![service],
        handler_contracts: vec![contract],
        ..ApiSurface::default()
    }
}

/// Make a test config with explicit Java and Kotlin JVM packages.
fn make_test_config() -> ResolvedCrateConfig {
    use alef::core::config::NewAlefConfig;

    let toml_str = r#"
[workspace]
languages = ["java", "kotlin"]

[[crates]]
name = "test-crate"
sources = ["src/lib.rs"]

[crates.java]
package = "dev.sample_crate"

[crates.kotlin]
package = "dev.sample_crate.kt"
"#;

    let raw: NewAlefConfig = toml::from_str(toml_str).expect("failed to parse test config");
    let resolved = raw.resolve().expect("failed to resolve config");
    resolved.into_iter().next().expect("should have at least one crate")
}

#[test]
fn service_api_jvm_symbol_consistency() {
    let api = make_test_surface();
    let config = make_test_config();

    let java_files = JavaBackend
        .generate_service_api(&api, &config)
        .expect("java generate_service_api should succeed");
    assert!(!java_files.is_empty(), "java should generate service files");
    let java_content = java_files
        .iter()
        .find(|file| file.path.ends_with("ApiSurface.java"))
        .expect("java should generate ApiSurface.java")
        .content
        .clone();

    let kotlin_files = KotlinBackend
        .generate_service_api(&api, &config)
        .expect("kotlin generate_service_api should succeed");
    assert!(!kotlin_files.is_empty(), "kotlin should generate service file");
    let kotlin_content = kotlin_files[0].content.clone();

    assert!(java_content.contains("package dev.sample_crate;"));
    assert!(java_content.contains("LOOKUP.find(\"test_crate_api_surface_new\")"));
    assert!(
        java_content
            .matches("test_crate_api_surface_register_on_request")
            .count()
            >= 2,
        "java should document and look up the registration C symbol:\n{java_content}"
    );
    assert!(java_content.contains("public int registerApiSurfaceOnRequest(Callable handler, String pattern)"));
    assert!(java_content.contains("public void run(String addr)"));
    assert!(java_content.contains("public long shutdown()"));

    assert!(kotlin_content.contains("package dev.sample_crate.kt"));
    assert!(kotlin_content.contains("internal val inner: dev.sample_crate.ApiSurface"));
    assert!(kotlin_content.contains("constructor() : this(dev.sample_crate.ApiSurface())"));
    assert!(kotlin_content.contains("import dev.sample_crate.Callable"));
    assert!(kotlin_content.contains("fun onRequest(handler: (String) -> String, pattern: String): Int"));
    assert!(
        kotlin_content.contains("inner.registerApiSurfaceOnRequest(Callable { request -> handler(request) }, pattern)")
    );
    assert!(kotlin_content.contains("suspend fun run(addr: String) = withContext(Dispatchers.IO) { inner.run(addr) }"));
    assert!(kotlin_content.contains("fun shutdown(): Int = inner.shutdown()"));
    assert!(
        !kotlin_content.contains("external fun"),
        "kotlin JVM service wrapper should delegate to Java/Panama, not declare JNI externs:\n{kotlin_content}"
    );
}
