//! Integration test: Verify JNI service Rust symbols match Kotlin external fun declarations.
//!
//! This test ensures that the jni and kotlin service-API backends emit symbols that link correctly.
//! For each kotlin `external fun`, the corresponding Rust JNI symbol is computed and verified
//! to exist in the jni output.

use alef::backends::jni::JniBackend;
use alef::backends::kotlin::KotlinBackend;
use alef::core::backend::Backend;
use alef::core::config::ResolvedCrateConfig;
use alef::core::ir::{
    ApiSurface, EntrypointDef, EntrypointKind, HandlerContractDef, MethodDef, ParamDef, PrimitiveType, RegistrationDef,
    ServiceDef, TypeRef,
};
use alef::core::jni::{jni_package, jni_symbol, service_bridge_class_name};

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

/// Make a test config with kotlin_android package set.
fn make_test_config() -> ResolvedCrateConfig {
    use alef::core::config::NewAlefConfig;

    let toml_str = r#"
[workspace]
languages = ["kotlin_android", "jni"]

[[crates]]
name = "test-crate"
sources = ["src/lib.rs"]

[crates.kotlin_android]
package = "dev.sample_crate"
namespace = "dev.sample_crate"
"#;

    let raw: NewAlefConfig = toml::from_str(toml_str).expect("failed to parse test config");
    let resolved = raw.resolve().expect("failed to resolve config");
    resolved.into_iter().next().expect("should have at least one crate")
}

#[test]
fn service_api_jvm_symbol_consistency() {
    let api = make_test_surface();
    let config = make_test_config();

    // Generate jni service Rust code via the public Backend hook.
    let jni_files = JniBackend
        .generate_service_api(&api, &config)
        .expect("jni generate_service_api should succeed");
    assert!(!jni_files.is_empty(), "jni should generate service.rs");
    let jni_content = jni_files[0].content.clone();

    // Generate kotlin service Kotlin code via the public Backend hook.
    let kotlin_files = KotlinBackend
        .generate_service_api(&api, &config)
        .expect("kotlin generate_service_api should succeed");
    assert!(!kotlin_files.is_empty(), "kotlin should generate service file");
    let kotlin_content = kotlin_files[0].content.clone();

    // Resolve package and the per-service bridge class exactly as jni and kotlin do.
    let package = jni_package(&config);
    assert_eq!(package, "dev.sample_crate", "package should come from kotlin_android");

    // The class hosting the external funs is the per-service bridge object — both jni
    // (symbol class) and kotlin (object name) must use this identical string.
    let bridge_class = service_bridge_class_name(&api.services[0].name);
    assert_eq!(bridge_class, "ApiSurfaceServiceBridge");

    // Extract Kotlin external fun names from the bridge object
    // Simple string search for "external fun <name>("
    let mut kotlin_external_names = Vec::new();
    let mut remaining: &str = kotlin_content.as_str();
    while let Some(pos) = remaining.find("external fun ") {
        remaining = &remaining[pos + 13..];
        if let Some(paren) = remaining.find('(') {
            let method_name = remaining[..paren].trim();
            kotlin_external_names.push(method_name.to_owned());
            remaining = &remaining[paren..];
        }
    }

    assert!(
        !kotlin_external_names.is_empty(),
        "kotlin should declare external funs:\n{kotlin_content}"
    );

    // For each kotlin external fun, compute the expected jni symbol
    let expected_jni_symbols: Vec<String> = kotlin_external_names
        .iter()
        .map(|method_name| jni_symbol(&package, &bridge_class, method_name))
        .collect();

    // Verify each symbol appears in the jni Rust code
    for symbol in &expected_jni_symbols {
        assert!(
            jni_content.contains(&format!("pub extern \"system\" fn {symbol}("))
                || jni_content.contains(&format!("pub extern \"system\" fn {symbol} ("))
                || jni_content.contains(&format!("pub extern \"system\" fn {symbol}\n")),
            "jni should emit symbol {symbol}:\n{jni_content}"
        );
    }

    // Count expected symbols: 1 constructor + 1 destructor + 1 register + 2 entrypoints = 5
    assert_eq!(
        kotlin_external_names.len(),
        5,
        "should have 5 external funs (constructor, destructor, register, run, finalize)"
    );

    // Verify specific method names match the pattern (based on bridge_method_name scheme)
    let has_new = kotlin_external_names.iter().any(|n| n.contains("New"));
    let has_free = kotlin_external_names.iter().any(|n| n.contains("Free"));
    let has_register = kotlin_external_names
        .iter()
        .any(|n| n.contains("Register") && n.contains("OnRequest"));
    let has_run = kotlin_external_names.iter().any(|n| n.contains("Run"));
    let has_finalize = kotlin_external_names
        .iter()
        .any(|n| n.contains("Shutdown") || n.contains("Finalize"));

    assert!(has_new, "should have constructor (New) method");
    assert!(has_free, "should have destructor (Free) method");
    assert!(has_register, "should have register method");
    assert!(has_run, "should have run entrypoint");
    assert!(has_finalize, "should have finalize entrypoint");

    println!("✓ JVM service-API symbol consistency verified");
    println!("  Package: {}", package);
    println!("  Bridge class: {}", bridge_class);
    println!("  External funs: {:?}", kotlin_external_names);
    println!("  JNI symbols: {:?}", expected_jni_symbols);
}
