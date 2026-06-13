use super::*;
use crate::core::ir::{
    EntrypointDef, EntrypointKind, HandlerContractDef, MethodDef, ParamDef, RegistrationDef, ServiceDef, TypeRef,
};

fn make_test_config() -> ResolvedCrateConfig {
    ResolvedCrateConfig {
        name: "test_crate".to_owned(),
        ..ResolvedCrateConfig::default()
    }
}

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
        version: Default::default(),
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
        doc: "Register a request handler.".to_owned(),
        variants: vec![],
        ..Default::default()
    };

    let run_entrypoint = EntrypointDef {
        method: "run".to_owned(),
        kind: EntrypointKind::Run,
        is_async: false,
        params: vec![ParamDef {
            name: "addr".to_owned(),
            ty: TypeRef::String,
            optional: false,
            default: None,
            ..ParamDef::default()
        }],
        return_type: TypeRef::Unit,
        error_type: Some("IoError".to_owned()),
        doc: "Start the service.".to_owned(),
    };

    let handler_contract = HandlerContractDef {
        trait_name: "RequestHandler".to_owned(),
        rust_path: "my_crate::RequestHandler".to_owned(),
        dispatch: MethodDef {
            name: "handle".to_owned(),
            params: vec![ParamDef {
                name: "req".to_owned(),
                ty: TypeRef::Named("RequestData".to_owned()),
                optional: false,
                default: None,
                ..ParamDef::default()
            }],
            return_type: TypeRef::Named("Response".to_owned()),
            is_async: true,
            is_static: false,
            error_type: None,
            doc: "Handle a request.".to_owned(),
            receiver: Some(crate::core::ir::ReceiverKind::Ref),
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        },
        optional_methods: vec![],
        wire_request_type: Some("RequestData".to_owned()),
        wire_response_type: Some("Response".to_owned()),
        dispatch_extra_params: vec![],
        wire_param_name: None,
        dispatch_return_type: None,
        response_adapter: None,
        doc: "Handler contract.".to_owned(),
    };

    ApiSurface {
        crate_name: "test_crate".to_owned(),
        version: "1.0.0".to_owned(),
        services: vec![ServiceDef {
            name: "TestService".to_owned(),
            rust_path: "my_crate::TestService".to_owned(),
            constructor,
            configurators: vec![],
            registrations: vec![registration],
            entrypoints: vec![run_entrypoint],
            doc: "Test service.".to_owned(),
            cfg: None,
        }],
        handler_contracts: vec![handler_contract],
        ..ApiSurface::default()
    }
}

#[test]
fn test_gen_service_swift_contains_class() {
    let api = make_fixture_surface();
    let service = &api.services[0];
    let config = make_test_config();
    let output = gen_service_swift(&api, service, &config);

    assert!(
        output.contains("public final class TestService"),
        "expected `public final class TestService` in output:\n{output}"
    );
}

#[test]
fn test_gen_service_swift_contains_init_and_deinit() {
    let api = make_fixture_surface();
    let service = &api.services[0];
    let config = make_test_config();
    let output = gen_service_swift(&api, service, &config);

    assert!(
        output.contains("public init()"),
        "expected `public init()` in output:\n{output}"
    );
    assert!(output.contains("deinit"), "expected `deinit` in output:\n{output}");
    assert!(
        output.contains("handlerBoxes.removeAll()"),
        "expected handler box cleanup in deinit:\n{output}"
    );
}

#[test]
fn test_gen_service_swift_boxes_handler() {
    let api = make_fixture_surface();
    let service = &api.services[0];
    let config = make_test_config();
    let output = gen_service_swift(&api, service, &config);

    assert!(
        output.contains("private final class HandlerBox"),
        "expected HandlerBox reference type:\n{output}"
    );
    assert!(
        output.contains("private var handlerBoxes: [UnsafeMutableRawPointer]"),
        "expected retained-box tracking array:\n{output}"
    );
    assert!(
        output.contains("Unmanaged.passRetained(handlerBox).toOpaque()"),
        "expected the handler box to be retained as the context pointer:\n{output}"
    );
    assert!(
        output.contains("Unmanaged<HandlerBox>.fromOpaque(contextPtr).release()"),
        "expected boxes to be released in deinit:\n{output}"
    );
}

#[test]
fn test_gen_service_swift_contains_registration_method() {
    let api = make_fixture_surface();
    let service = &api.services[0];
    let config = make_test_config();
    let output = gen_service_swift(&api, service, &config);

    assert!(
        output.contains("public func addHandler"),
        "expected registration method `addHandler`:\n{output}"
    );
    assert!(
        output.contains("@convention(c)"),
        "expected C-compatible closure:\n{output}"
    );
    assert!(
        output.contains("trampolineFunc"),
        "expected C trampoline function:\n{output}"
    );
}

#[test]
fn test_gen_service_swift_contains_context_recovery() {
    let api = make_fixture_surface();
    let service = &api.services[0];
    let config = make_test_config();
    let output = gen_service_swift(&api, service, &config);

    assert!(
        output.contains("Unmanaged<HandlerBox>.fromOpaque(contextPtr).takeUnretainedValue()"),
        "expected the boxed handler to be recovered from the context pointer:\n{output}"
    );
    assert!(
        output.contains("handlerBox.handler(requestJSON)"),
        "expected the recovered handler to be invoked with the request:\n{output}"
    );
}

#[test]
fn test_gen_service_swift_contains_run_method() {
    let api = make_fixture_surface();
    let service = &api.services[0];
    let config = make_test_config();
    let output = gen_service_swift(&api, service, &config);

    assert!(
        output.contains("public func run"),
        "expected `run` entrypoint method:\n{output}"
    );
    assert!(
        output.contains("inner.run("),
        "expected instance method call to inner.run():\n{output}"
    );
}

#[test]
fn test_gen_rust_extern_blocks_contains_type_decl() {
    let api = make_fixture_surface();
    let service = &api.services[0];
    let output = gen_service_rust_extern_blocks(service, &api);

    assert!(
        output.contains("type TestService;"),
        "expected opaque type declaration:\n{output}"
    );
    assert!(
        output.contains("extern \"Rust\""),
        "expected extern \"Rust\" block:\n{output}"
    );
}

#[test]
fn test_gen_rust_extern_blocks_excludes_callback_registration() {
    let api = make_fixture_surface();
    let service = &api.services[0];
    let output = gen_service_rust_extern_blocks(service, &api);

    // Callback registration should NOT be in the bridge module
    assert!(
        !output.contains("extern \"C\" fn(*mut std::ffi::c_void, *const u8, usize) -> *mut u8"),
        "expected raw pointer callback signature to be EXCLUDED from bridge module:\n{output}"
    );
    assert!(
        !output.contains("_via_callback"),
        "expected callback-shim registration method to be EXCLUDED from bridge module:\n{output}"
    );
}

#[test]
fn test_generate_rust_callback_c_functions_contains_callback_signature() {
    let api = make_fixture_surface();
    let service = &api.services[0];
    let output = gen_rust_callback_c_functions_for_service(&api, service);

    // Callback registration SHOULD be in the C function output
    assert!(
        output.contains("extern \"C\" fn"),
        "expected extern \"C\" fn in callback C function:\n{output}"
    );
    assert!(
        output.contains("_via_callback"),
        "expected callback-shim function name:\n{output}"
    );
    assert!(
        output.contains("*mut std::ffi::c_void"),
        "expected raw c_void pointer in callback:\n{output}"
    );
    assert!(
        output.contains("#[unsafe(no_mangle)]") || output.contains("#[no_mangle]"),
        "expected #[unsafe(no_mangle)] or #[no_mangle] on extern \"C\" function:\n{output}"
    );
}

#[test]
fn test_gen_rust_extern_blocks_contains_result_return() {
    let api = make_fixture_surface();
    let service = &api.services[0];
    let output = gen_service_rust_extern_blocks(service, &api);

    // Fallible entrypoints return a JSON envelope string (swift-bridge 0.1.59
    // cannot parse Result<T, E> in extern blocks).
    assert!(
        output.contains("-> String") || output.contains("-> Result<(), String>"),
        "expected entrypoint return type (JSON envelope or unit):\n{output}"
    );
}

#[test]
fn test_generate_returns_file_for_non_empty_services() {
    let api = make_fixture_surface();
    let config = ResolvedCrateConfig {
        name: "test_crate".to_owned(),
        ..ResolvedCrateConfig::default()
    };

    let files = generate(&api, &config).expect("generate should not fail");
    assert!(!files.is_empty(), "expected at least one generated file");

    let has_service_file = files.iter().any(|f| {
        f.path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|s| s.ends_with("TestService.swift"))
            .unwrap_or(false)
    });
    assert!(has_service_file, "expected TestService.swift in output");
}

#[test]
fn test_generate_returns_empty_for_no_services() {
    let api = ApiSurface::default();
    let config = ResolvedCrateConfig {
        name: "test_crate".to_owned(),
        ..ResolvedCrateConfig::default()
    };

    let files = generate(&api, &config).expect("generate should not fail");
    assert!(files.is_empty(), "expected no files for surface without services");
}

#[test]
fn test_generate_skips_services_without_registrations() {
    let mut api = make_fixture_surface();
    api.services[0].registrations.clear();

    let config = ResolvedCrateConfig {
        name: "test_crate".to_owned(),
        ..ResolvedCrateConfig::default()
    };

    let files = generate(&api, &config).expect("generate should not fail");
    assert!(files.is_empty(), "expected no files for service without registrations");
}

#[test]
fn test_swift_wrapper_no_dlsym_or_dlopen() {
    let api = make_fixture_surface();
    let service = &api.services[0];
    let config = make_test_config();
    let output = gen_service_swift(&api, service, &config);

    assert!(
        !output.contains("dlsym"),
        "expected no dlsym (swift-bridge-based, not raw C lookup):\n{output}"
    );
    assert!(
        !output.contains("dlopen"),
        "expected no dlopen (swift-bridge-based, not raw C lookup):\n{output}"
    );
}

#[test]
fn test_rust_extern_blocks_no_raw_symbol_hardcode() {
    let api = make_fixture_surface();
    let service = &api.services[0];
    let output = gen_service_rust_extern_blocks(service, &api);

    // No hardcoded HTTP/framework names — everything from IR
    assert!(
        !output.contains("\"http\""),
        "expected no hardcoded HTTP references:\n{output}"
    );
    assert!(
        !output.contains("\"handler\""),
        "expected no hardcoded handler-trait names:\n{output}"
    );
}

#[test]
fn test_registration_no_empty_leading_comma() {
    let api = make_fixture_surface();
    let service = &api.services[0];
    let config = make_test_config();
    let output = gen_service_swift(&api, service, &config);

    // Should not have double comma like "(_ handler: ..., , builder: ...)"
    assert!(
        !output.contains(", , "),
        "expected no double comma in registration signature:\n{output}"
    );
}

#[test]
fn test_switch_case_on_own_lines() {
    let api = make_fixture_surface();
    let service = &api.services[0];
    let config = make_test_config();
    let output = gen_service_swift(&api, service, &config);

    // switch/case should not collapse onto the same line as preceding code
    assert!(
        !output.contains(")        switch"),
        "expected switch on its own line, not collapsed:\n{output}"
    );
    assert!(
        !output.contains("case .success:            break        case .failure"),
        "expected each case on its own line:\n{output}"
    );
}

#[test]
fn test_swift_uses_silgen_not_bridge_method() {
    let api = make_fixture_surface();
    let service = &api.services[0];
    let config = make_test_config();
    let output = gen_service_swift(&api, service, &config);

    // Should use @_silgen_name'd C function, NOT inner.addHandlerViaCallback()
    assert!(
        !output.contains("inner.addHandlerViaCallback("),
        "expected callback to use @_silgen_name C function, NOT swift-bridge method:\n{output}"
    );
    assert!(
        output.contains("_test_service_add_handler_via_callback("),
        "expected call to @_silgen_name'd C function:\n{output}"
    );
}

#[test]
fn test_swift_contains_silgen_declaration() {
    let api = make_fixture_surface();
    let service = &api.services[0];
    let config = make_test_config();
    let output = gen_service_swift(&api, service, &config);

    // Should have @_silgen_name declaration at module scope
    assert!(
        output.contains("@_silgen_name(\"test_service_add_handler_via_callback\")"),
        "expected @_silgen_name declaration for callback C function:\n{output}"
    );
    assert!(
        output.contains("private func _test_service_add_handler_via_callback("),
        "expected private func declaration for silgen'd C function:\n{output}"
    );
}

#[test]
fn test_named_metadata_types_preserved() {
    let mut api = make_fixture_surface();
    // Change the metadata param type from String to a Named type
    api.services[0].registrations[0].metadata_params[0].ty = TypeRef::Named("RouteBuilder".to_owned());

    let service = &api.services[0];
    let config = make_test_config();
    let output = gen_service_swift(&api, service, &config);

    // Should use the swift-bridge wrapper `RustBridge.RouteBuilder` as the type
    // (Named metadata params are owned by the bridge module and must be qualified).
    assert!(
        output.contains("path: RustBridge.RouteBuilder"),
        "expected Named metadata param typed as RustBridge.RouteBuilder, not String:\n{output}"
    );
}

#[test]
fn test_skip_non_representable_finalize() {
    let mut api = make_fixture_surface();
    // Add a finalize entrypoint with a non-representable return type (Vec<String>)
    api.services[0].entrypoints.push(crate::core::ir::EntrypointDef {
        method: "into_router".to_owned(),
        kind: crate::core::ir::EntrypointKind::Finalize,
        is_async: false,
        params: vec![],
        return_type: TypeRef::Vec(Box::new(TypeRef::String)),
        error_type: None,
        doc: "Build the router.".to_owned(),
    });

    let service = &api.services[0];
    let config = make_test_config();
    let output = gen_service_swift(&api, service, &config);

    // Should not contain intoRouter method
    assert!(
        !output.contains("func intoRouter"),
        "expected finalize with non-representable return to be skipped:\n{output}"
    );
}

#[test]
fn test_rust_extern_has_swift_bridge_names() {
    let api = make_fixture_surface();
    let service = &api.services[0];
    let output = gen_service_rust_extern_blocks(service, &api);

    // Should have #[swift_bridge(swift_name = ...)] attributes in method block
    assert!(
        output.contains("#[swift_bridge(swift_name ="),
        "expected swift_bridge swift_name attribute:\n{output}"
    );
}
