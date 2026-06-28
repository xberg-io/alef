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
        error_type: Some("HandlerError".to_owned()),
        doc: "Register a request handler.".to_owned(),
        variants: vec![],
        ..Default::default()
    };

    let run_entrypoint = EntrypointDef {
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
fn test_emit_service_owner_with_frb_opaque() {
    let api = make_fixture_surface();
    let config = ResolvedCrateConfig {
        name: "test_crate".to_owned(),
        ..ResolvedCrateConfig::default()
    };

    let rust = gen_service_rust(&api, &config);

    // Verify FRB opaque marker
    assert!(
        rust.contains("#[frb(opaque)]"),
        "expected `#[frb(opaque)]` marker in:\n{rust}"
    );

    // Verify service owner struct
    assert!(
        rust.contains("pub struct TestService"),
        "expected service owner struct in:\n{rust}"
    );

    // Verify no registrations field (handlers registered immediately)
    assert!(
        !rust.contains("registrations: tokio::sync::Mutex"),
        "should not have registrations field in:\n{rust}"
    );
}

#[test]
fn test_emit_registration_with_dartfnfuture() {
    let api = make_fixture_surface();
    let config = ResolvedCrateConfig {
        name: "test_crate".to_owned(),
        ..ResolvedCrateConfig::default()
    };

    let rust = gen_service_rust(&api, &config);

    // Verify DartFnFuture parameter
    assert!(
        rust.contains("DartFnFuture<String>"),
        "expected `DartFnFuture<String>` in registration method:\n{rust}"
    );
}

#[test]
fn test_emit_handler_bridge_with_manual_pin_box() {
    let api = make_fixture_surface();
    let config = ResolvedCrateConfig {
        name: "test_crate".to_owned(),
        ..ResolvedCrateConfig::default()
    };

    let rust = gen_service_rust(&api, &config);

    // Verify manual Pin<Box<dyn Future>> form (not #[async_trait])
    assert!(
        rust.contains("Pin<Box<dyn std::future::Future"),
        "expected manual `Pin<Box<dyn Future>>` form in handler bridge:\n{rust}"
    );

    // Verify NOT async_trait
    assert!(
        !rust.contains("#[async_trait]"),
        "should NOT emit #[async_trait] in:\n{rust}"
    );
}

#[test]
fn test_no_dart_ffi_symbols() {
    let api = make_fixture_surface();
    let config = ResolvedCrateConfig {
        name: "test_crate".to_owned(),
        ..ResolvedCrateConfig::default()
    };

    let rust = gen_service_rust(&api, &config);

    // Verify NO dart:ffi symbols
    assert!(!rust.contains("dart:ffi"), "should not contain dart:ffi");
    assert!(!rust.contains("NativeCallable"), "should not contain NativeCallable");
    assert!(!rust.contains("lookupFunction"), "should not contain lookupFunction");
    assert!(!rust.contains("ffi.Pointer"), "should not contain ffi.Pointer");
}

#[test]
fn test_skip_unrepresentable_finalize() {
    let constructor = MethodDef {
        name: "new".to_owned(),
        params: vec![],
        return_type: TypeRef::Unit,
        is_async: false,
        is_static: true,
        error_type: None,
        doc: String::new(),
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

    // Finalize with unrepresentable Named return type
    let finalize_ep = EntrypointDef {
        method: "into_router".to_owned(),
        kind: EntrypointKind::Finalize,
        is_async: false,
        params: vec![],
        return_type: TypeRef::Named("ExternalRouter".to_owned()), // NOT in types
        error_type: None,
        doc: String::new(),
    };

    let api = ApiSurface {
        crate_name: "test_crate".to_owned(),
        version: "1.0.0".to_owned(),
        services: vec![ServiceDef {
            name: "TestService".to_owned(),
            rust_path: "my_crate::TestService".to_owned(),
            constructor,
            configurators: vec![],
            registrations: vec![],
            entrypoints: vec![finalize_ep],
            doc: String::new(),
            cfg: None,
        }],
        ..ApiSurface::default()
    };

    let config = ResolvedCrateConfig {
        name: "test_crate".to_owned(),
        ..ResolvedCrateConfig::default()
    };

    let rust = gen_service_rust(&api, &config);

    // Verify that the unrepresentable finalize method is NOT emitted
    assert!(
        !rust.contains("into_router"),
        "should not emit unrepresentable finalize method:\n{rust}"
    );
}

#[test]
fn test_skip_sanitized_finalize() {
    let constructor = MethodDef {
        name: "new".to_owned(),
        params: vec![],
        return_type: TypeRef::Unit,
        is_async: false,
        is_static: true,
        error_type: None,
        doc: String::new(),
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

    // Finalize method marked as sanitized (returns an unknown type mapped to String)
    let finalize_method = MethodDef {
        name: "into_router".to_owned(),
        params: vec![],
        return_type: TypeRef::String, // sanitized from unknown Router type
        is_async: false,
        is_static: false,
        error_type: None,
        doc: String::new(),
        receiver: Some(crate::core::ir::ReceiverKind::RefMut),
        sanitized: true, // <-- KEY: method is marked as sanitized
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    };

    // Finalize entrypoint that references the sanitized method
    let finalize_ep = EntrypointDef {
        method: "into_router".to_owned(),
        kind: EntrypointKind::Finalize,
        is_async: false,
        params: vec![],
        return_type: TypeRef::String, // representable as String, but source is sanitized
        error_type: None,
        doc: String::new(),
    };

    // Service type that owns the sanitized method
    use crate::core::ir::TypeDef;
    let service_type = TypeDef {
        name: "TestService".to_owned(),
        rust_path: "my_crate::TestService".to_owned(),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![finalize_method],
        is_opaque: false,
        is_clone: false,
        is_copy: false,
        doc: String::new(),
        cfg: None,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
    };

    let api = ApiSurface {
        crate_name: "test_crate".to_owned(),
        version: "1.0.0".to_owned(),
        types: vec![service_type],
        services: vec![ServiceDef {
            name: "TestService".to_owned(),
            rust_path: "my_crate::TestService".to_owned(),
            constructor,
            configurators: vec![],
            registrations: vec![],
            entrypoints: vec![finalize_ep],
            doc: String::new(),
            cfg: None,
        }],
        ..ApiSurface::default()
    };

    let config = ResolvedCrateConfig {
        name: "test_crate".to_owned(),
        ..ResolvedCrateConfig::default()
    };

    let rust = gen_service_rust(&api, &config);

    // Verify that the sanitized finalize method is NOT emitted
    assert!(
        !rust.contains("into_router"),
        "should not emit finalize method when source is sanitized:\n{rust}"
    );
}

#[test]
fn generate_returns_empty_for_no_services() {
    let api = ApiSurface::default();
    let config = ResolvedCrateConfig {
        name: "test_crate".to_owned(),
        ..ResolvedCrateConfig::default()
    };

    let files = generate(&api, &config).expect("generate should not fail");
    assert!(files.is_empty(), "expected no files for surface without services");
}

#[test]
fn generate_returns_one_file_for_services() {
    let api = make_fixture_surface();
    let config = ResolvedCrateConfig {
        name: "test_crate".to_owned(),
        ..ResolvedCrateConfig::default()
    };

    let files = generate(&api, &config).expect("generate should not fail");
    assert_eq!(files.len(), 1, "expected 1 generated file");
    assert!(
        files[0].path.to_string_lossy().ends_with("service_api.rs"),
        "expected service_api.rs file"
    );
}

#[test]
fn frb_user_callback_param_uses_non_shadowing_name() {
    let api = make_fixture_surface();
    let config = ResolvedCrateConfig {
        name: "test_crate".to_owned(),
        ..ResolvedCrateConfig::default()
    };

    let rust = gen_service_rust(&api, &config);

    // Verify that user-callback parameters use `cb`, not `handler`, to avoid
    // shadowing FRB's internal `BaseHandler` field in generated Dart code.
    // The parameter name `handler` caused FRB to emit `handler.executeSync()`
    // calls against the user callback (a plain Function) instead of the field.
    assert!(
        rust.contains("cb: impl Fn(String) -> DartFnFuture<String>"),
        "expected callback param named `cb` in:\n{rust}"
    );

    // Ensure the old shadowing name is NOT present
    let service_method = rust
        .split("pub fn add_handler(")
        .nth(1)
        .and_then(|body| body.split(") -> i32").next())
        .expect("generated add_handler signature should be present");
    assert!(
        !service_method.contains("handler: impl Fn(String) -> DartFnFuture<String> + Send + Sync + 'static"),
        "callback param must not be named `handler` to avoid FRB shadowing in:\n{rust}"
    );

    // Verify that the callback is forwarded using the new name
    assert!(
        rust.contains("::new(cb)"),
        "expected callback forwarding via `cb` in:\n{rust}"
    );
}

#[test]
fn test_registration_calls_inner_directly() {
    let api = make_fixture_surface();
    let config = ResolvedCrateConfig {
        name: "test_crate".to_owned(),
        ..ResolvedCrateConfig::default()
    };

    let rust = gen_service_rust(&api, &config);

    // Verify that registration methods call inner.<method_name> directly
    // (not deferred dispatch logic)
    assert!(
        rust.contains("inner.add_handler("),
        "expected immediate inner.add_handler() call in:\n{rust}"
    );

    // Verify NO draining logic or match arms in registrations
    assert!(
        !rust.contains("for reg in registrations"),
        "should not have registration draining loop in:\n{rust}"
    );

    assert!(
        !rust.contains("match reg.method.as_str()"),
        "should not have method dispatch match in:\n{rust}"
    );
}

#[test]
fn test_emit_registration_variants() {
    use crate::core::ir::{RegistrationVariant, WrapperConstructorArg, WrapperConstructorCall};

    let constructor = MethodDef {
        name: "new".to_owned(),
        params: vec![],
        return_type: TypeRef::Unit,
        is_async: false,
        is_static: true,
        error_type: None,
        doc: String::new(),
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

    let variant_with_wrapper = RegistrationVariant {
        name: "get".to_owned(),
        overrides: vec![],
        wrapper_call: Some(WrapperConstructorCall {
            metadata_param: "builder".to_owned(),
            wrapper_type_path: "my_crate::RouteBuilder".to_owned(),
            wrapper_type_name: "RouteBuilder".to_owned(),
            constructor_method: "new".to_owned(),
            args: vec![
                WrapperConstructorArg::Fixed {
                    param_name: "method".to_owned(),
                    value_expr: "my_crate::Method::GET".to_owned(),
                },
                WrapperConstructorArg::Free {
                    param: ParamDef {
                        name: "path".to_owned(),
                        ty: TypeRef::String,
                        optional: false,
                        default: None,
                        ..ParamDef::default()
                    },
                },
            ],
        }),
        signature_params: vec![ParamDef {
            name: "path".to_owned(),
            ty: TypeRef::String,
            optional: false,
            default: None,
            ..ParamDef::default()
        }],
        doc: Some("Register a GET handler.".to_owned()),
        style: Default::default(),
        ..Default::default()
    };

    let registration = RegistrationDef {
        method: "route".to_owned(),
        callback_param: "handler".to_owned(),
        callback_contract: "RequestHandler".to_owned(),
        metadata_params: vec![ParamDef {
            name: "builder".to_owned(),
            ty: TypeRef::Named("RouteBuilder".to_owned()),
            optional: false,
            default: None,
            ..ParamDef::default()
        }],
        receiver: Some(crate::core::ir::ReceiverKind::RefMut),
        return_type: TypeRef::Unit,
        error_type: Some("HandlerError".to_owned()),
        doc: "Register a handler with a route builder.".to_owned(),
        variants: vec![variant_with_wrapper],
        ..Default::default()
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
            doc: String::new(),
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
        doc: String::new(),
    };

    let api = ApiSurface {
        crate_name: "test_crate".to_owned(),
        version: "1.0.0".to_owned(),
        services: vec![ServiceDef {
            name: "TestService".to_owned(),
            rust_path: "my_crate::TestService".to_owned(),
            constructor,
            configurators: vec![],
            registrations: vec![registration],
            entrypoints: vec![],
            doc: String::new(),
            cfg: None,
        }],
        handler_contracts: vec![handler_contract],
        ..ApiSurface::default()
    };

    let config = ResolvedCrateConfig {
        name: "test_crate".to_owned(),
        ..ResolvedCrateConfig::default()
    };

    let rust = gen_service_rust(&api, &config);

    // Verify variant method is emitted
    assert!(
        rust.contains("pub fn get("),
        "expected registration variant 'get' method in:\n{rust}"
    );

    // Verify wrapper constructor is called
    assert!(
        rust.contains("my_crate::RouteBuilder::new("),
        "expected wrapper constructor call in:\n{rust}"
    );

    // Verify fixed arg is substituted
    assert!(
        rust.contains("my_crate::Method::GET"),
        "expected fixed wrapper arg in:\n{rust}"
    );

    // Verify calls back to base registration method, wrapping the constructed
    // inner value in the local Dart wrapper newtype before forwarding.
    assert!(
        rust.contains("self.route(RouteBuilder { inner }"),
        "expected call to base route method with local newtype wrapper in:\n{rust}"
    );
}
