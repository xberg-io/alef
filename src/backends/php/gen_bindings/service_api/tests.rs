use super::*;
use crate::core::ir::{
    EntrypointDef, EntrypointKind, HandlerContractDef, MethodDef, ParamDef, PrimitiveType, RegistrationDef,
    RegistrationVariantStyle, ServiceDef, TypeRef,
};

/// Construct a minimal but realistic [`ApiSurface`] that exercises:
/// - A service with a constructor, one configurator, one registration
///   (bound to an async handler contract), and Run + Finalize entrypoints.
/// - One [`HandlerContractDef`] with wire request/response DTO names.
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

    let configurator = MethodDef {
        name: "with_timeout".to_owned(),
        params: vec![ParamDef {
            name: "timeout_ms".to_owned(),
            ty: TypeRef::Primitive(PrimitiveType::U64),
            optional: false,
            default: None,
            ..ParamDef::default()
        }],
        return_type: TypeRef::Named("TestService".to_owned()),
        is_async: false,
        is_static: false,
        error_type: None,
        doc: "Set request timeout.".to_owned(),
        receiver: Some(crate::core::ir::ReceiverKind::RefMut),
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
        metadata_params: vec![
            ParamDef {
                name: "path".to_owned(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                ..ParamDef::default()
            },
            ParamDef {
                name: "method".to_owned(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                ..ParamDef::default()
            },
        ],
        receiver: Some(crate::core::ir::ReceiverKind::RefMut),
        return_type: TypeRef::Unit,
        error_type: None,
        doc: "Register a request handler for a path and method.".to_owned(),
        variants: vec![],
        ..Default::default()
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

    let finalize_ep = EntrypointDef {
        method: "into_router".to_owned(),
        kind: EntrypointKind::Finalize,
        is_async: false,
        params: vec![],
        return_type: TypeRef::Named("Router".to_owned()),
        error_type: None,
        doc: "Consume and convert into a router.".to_owned(),
    };

    let service = ServiceDef {
        name: "TestService".to_owned(),
        rust_path: "my_crate::TestService".to_owned(),
        constructor,
        configurators: vec![configurator],
        registrations: vec![registration],
        entrypoints: vec![run_ep, finalize_ep],
        doc: "A test service owner.".to_owned(),
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
        version: Default::default(),
    };

    let contract = HandlerContractDef {
        trait_name: "RequestHandler".to_owned(),
        rust_path: "my_crate::RequestHandler".to_owned(),
        dispatch: dispatch_method,
        optional_methods: vec![],
        wire_request_type: Some("RequestData".to_owned()),
        wire_response_type: Some("ResponseData".to_owned()),
        dispatch_extra_params: vec![],
        wire_param_name: None,
        dispatch_return_type: None,
        response_adapter: None,
        doc: "Async trait for handling requests.".to_owned(),
    };

    ApiSurface {
        crate_name: "my_crate".to_owned(),
        version: "0.1.0".to_owned(),
        services: vec![service],
        handler_contracts: vec![contract],
        ..ApiSurface::default()
    }
}

/// `gen_service_php` emits a class named after the service owner.
#[test]
fn php_output_contains_service_class() {
    let surface = make_fixture_surface();
    let output = gen_service_php(&surface, "my_crate");
    assert!(
        output.contains("class TestService"),
        "expected `class TestService` in output:\n{output}"
    );
}

/// `gen_service_php` emits `__construct` with registrations initialization.
#[test]
fn php_output_contains_construct_with_registrations() {
    let surface = make_fixture_surface();
    let output = gen_service_php(&surface, "my_crate");
    assert!(
        output.contains("public function __construct()"),
        "expected `public function __construct()` in output:\n{output}"
    );
    assert!(
        output.contains("private array $registrations"),
        "expected `private array $registrations` in output:\n{output}"
    );
}

/// `gen_service_php` emits configurator methods that return `self`.
#[test]
fn php_output_contains_configurator() {
    let surface = make_fixture_surface();
    let output = gen_service_php(&surface, "my_crate");
    assert!(
        output.contains("public function with_timeout"),
        "expected `with_timeout` configurator:\n{output}"
    );
    assert!(
        output.contains("return $this"),
        "expected `return $this` in configurator:\n{output}"
    );
}

/// `gen_service_php` emits a registration method returning a closure.
#[test]
fn php_output_contains_registration_method() {
    let surface = make_fixture_surface();
    let output = gen_service_php(&surface, "my_crate");
    assert!(
        output.contains("public function add_handler("),
        "expected `add_handler` registration method:\n{output}"
    );
    assert!(
        output.contains("return function"),
        "expected inner `return function` closure:\n{output}"
    );
    assert!(
        output.contains("$this->registrations[]"),
        "expected `$this->registrations[]` append in registration:\n{output}"
    );
}

/// `gen_service_php` emits verb-decorator variant methods when variants are present.
#[test]
fn php_output_contains_registration_variants() {
    use crate::core::ir::{RegistrationVariant, RegistrationVariantOverride};

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

    let registration = RegistrationDef {
        method: "route".to_owned(),
        callback_param: "handler".to_owned(),
        callback_contract: "RequestHandler".to_owned(),
        metadata_params: vec![
            ParamDef {
                name: "method".to_owned(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                ..ParamDef::default()
            },
            ParamDef {
                name: "path".to_owned(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                ..ParamDef::default()
            },
        ],
        receiver: Some(crate::core::ir::ReceiverKind::RefMut),
        return_type: TypeRef::Unit,
        error_type: None,
        doc: String::new(),
        variants: vec![RegistrationVariant {
            name: "GET".to_owned(),
            overrides: vec![RegistrationVariantOverride {
                param_name: "method".to_owned(),
                value_expr: "\"GET\"".to_owned(),
            }],
            wrapper_call: None,
            signature_params: vec![ParamDef {
                name: "path".to_owned(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                ..ParamDef::default()
            }],
            doc: Some("Register a GET route.".to_owned()),
            style: Default::default(),
            ..Default::default()
        }],
        ..Default::default()
    };

    let service = ServiceDef {
        name: "Router".to_owned(),
        rust_path: "my_crate::Router".to_owned(),
        constructor,
        configurators: vec![],
        registrations: vec![registration],
        entrypoints: vec![],
        doc: String::new(),
        cfg: None,
    };

    let api = ApiSurface {
        crate_name: "my_crate".to_owned(),
        version: "0.1.0".to_owned(),
        services: vec![service],
        handler_contracts: vec![],
        ..ApiSurface::default()
    };

    let output = gen_service_php(&api, "my_crate");
    assert!(
        output.contains("public function get("),
        "expected `get` variant method (lowercase):\n{output}"
    );
    assert!(
        output.contains("\"GET\""),
        "expected fixed override `\"GET\"` in variant:\n{output}"
    );
}

/// `gen_service_php` emits only direct method form for VerbDecorator style.
#[test]
fn php_output_verb_decorator_style_direct_method_only() {
    use crate::core::ir::{RegistrationVariant, RegistrationVariantOverride};

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

    let registration = RegistrationDef {
        method: "route".to_owned(),
        callback_param: "handler".to_owned(),
        callback_contract: "RequestHandler".to_owned(),
        metadata_params: vec![
            ParamDef {
                name: "method".to_owned(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                ..ParamDef::default()
            },
            ParamDef {
                name: "path".to_owned(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                ..ParamDef::default()
            },
        ],
        receiver: Some(crate::core::ir::ReceiverKind::RefMut),
        return_type: TypeRef::Unit,
        error_type: None,
        doc: String::new(),
        variants: vec![RegistrationVariant {
            name: "GET".to_owned(),
            overrides: vec![RegistrationVariantOverride {
                param_name: "method".to_owned(),
                value_expr: "\"GET\"".to_owned(),
            }],
            wrapper_call: None,
            signature_params: vec![ParamDef {
                name: "path".to_owned(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                ..ParamDef::default()
            }],
            doc: Some("Register a GET route.".to_owned()),
            style: RegistrationVariantStyle::VerbDecorator,
            ..Default::default()
        }],
        ..Default::default()
    };

    let service = ServiceDef {
        name: "Router".to_owned(),
        rust_path: "my_crate::Router".to_owned(),
        constructor,
        configurators: vec![],
        registrations: vec![registration],
        entrypoints: vec![],
        doc: String::new(),
        cfg: None,
    };

    let api = ApiSurface {
        crate_name: "my_crate".to_owned(),
        version: "0.1.0".to_owned(),
        services: vec![service],
        handler_contracts: vec![],
        ..ApiSurface::default()
    };

    let output = gen_service_php(&api, "my_crate");

    assert!(
        output.contains("public function get(string $path, callable $handler): self"),
        "expected direct method form for VerbDecorator:\n{output}"
    );

    assert!(
        !output.contains("public function getDecorator("),
        "VerbDecorator should not emit factory method:\n{output}"
    );
}

/// `gen_service_php` emits only decorator-factory form for Builder style.
#[test]
fn php_output_builder_style_factory_only() {
    use crate::core::ir::{RegistrationVariant, RegistrationVariantOverride};

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

    let registration = RegistrationDef {
        method: "route".to_owned(),
        callback_param: "handler".to_owned(),
        callback_contract: "RequestHandler".to_owned(),
        metadata_params: vec![
            ParamDef {
                name: "method".to_owned(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                ..ParamDef::default()
            },
            ParamDef {
                name: "path".to_owned(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                ..ParamDef::default()
            },
        ],
        receiver: Some(crate::core::ir::ReceiverKind::RefMut),
        return_type: TypeRef::Unit,
        error_type: None,
        doc: String::new(),
        variants: vec![RegistrationVariant {
            name: "GET".to_owned(),
            overrides: vec![RegistrationVariantOverride {
                param_name: "method".to_owned(),
                value_expr: "\"GET\"".to_owned(),
            }],
            wrapper_call: None,
            signature_params: vec![ParamDef {
                name: "path".to_owned(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                ..ParamDef::default()
            }],
            doc: Some("Register a GET route.".to_owned()),
            style: RegistrationVariantStyle::Builder,
            ..Default::default()
        }],
        ..Default::default()
    };

    let service = ServiceDef {
        name: "Router".to_owned(),
        rust_path: "my_crate::Router".to_owned(),
        constructor,
        configurators: vec![],
        registrations: vec![registration],
        entrypoints: vec![],
        doc: String::new(),
        cfg: None,
    };

    let api = ApiSurface {
        crate_name: "my_crate".to_owned(),
        version: "0.1.0".to_owned(),
        services: vec![service],
        handler_contracts: vec![],
        ..ApiSurface::default()
    };

    let output = gen_service_php(&api, "my_crate");

    assert!(
        output.contains("public function getDecorator(string $path): Closure"),
        "expected factory method for Builder style:\n{output}"
    );

    assert!(
        !output.contains("public function get(string $path, callable $handler): self"),
        "Builder style should not emit direct method:\n{output}"
    );
}

/// `gen_service_php` emits both forms for Hybrid style.
#[test]
fn php_output_hybrid_style_both_forms() {
    use crate::core::ir::{RegistrationVariant, RegistrationVariantOverride};

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

    let registration = RegistrationDef {
        method: "route".to_owned(),
        callback_param: "handler".to_owned(),
        callback_contract: "RequestHandler".to_owned(),
        metadata_params: vec![
            ParamDef {
                name: "method".to_owned(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                ..ParamDef::default()
            },
            ParamDef {
                name: "path".to_owned(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                ..ParamDef::default()
            },
        ],
        receiver: Some(crate::core::ir::ReceiverKind::RefMut),
        return_type: TypeRef::Unit,
        error_type: None,
        doc: String::new(),
        variants: vec![RegistrationVariant {
            name: "GET".to_owned(),
            overrides: vec![RegistrationVariantOverride {
                param_name: "method".to_owned(),
                value_expr: "\"GET\"".to_owned(),
            }],
            wrapper_call: None,
            signature_params: vec![ParamDef {
                name: "path".to_owned(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                ..ParamDef::default()
            }],
            doc: Some("Register a GET route.".to_owned()),
            style: RegistrationVariantStyle::Hybrid,
            ..Default::default()
        }],
        ..Default::default()
    };

    let service = ServiceDef {
        name: "Router".to_owned(),
        rust_path: "my_crate::Router".to_owned(),
        constructor,
        configurators: vec![],
        registrations: vec![registration],
        entrypoints: vec![],
        doc: String::new(),
        cfg: None,
    };

    let api = ApiSurface {
        crate_name: "my_crate".to_owned(),
        version: "0.1.0".to_owned(),
        services: vec![service],
        handler_contracts: vec![],
        ..ApiSurface::default()
    };

    let output = gen_service_php(&api, "my_crate");

    assert!(
        output.contains("public function get(string $path, callable $handler): self"),
        "expected direct method form for Hybrid:\n{output}"
    );

    assert!(
        output.contains("public function getDecorator(string $path): Closure"),
        "expected factory method for Hybrid style:\n{output}"
    );
}

/// `gen_service_php` emits the `run` entrypoint.
#[test]
fn php_output_contains_run_entrypoint() {
    let surface = make_fixture_surface();
    let output = gen_service_php(&surface, "my_crate");
    assert!(
        output.contains("public function run("),
        "expected `public function run(` entrypoint:\n{output}"
    );
    assert!(
        output.contains("test_service_run("),
        "expected native call `test_service_run(` in run:\n{output}"
    );
}

/// `gen_service_rs` emits the handler bridge struct.
#[test]
fn rust_output_contains_handler_bridge_struct() {
    let surface = make_fixture_surface();
    let config = make_test_config();
    let output = gen_service_rs(&surface, &config);
    assert!(
        output.contains("pub struct PhpRequestHandlerBridge"),
        "expected `PhpRequestHandlerBridge` struct:\n{output}"
    );
}

/// `gen_service_rs` emits the handler bridge trait impl.
#[test]
fn rust_output_contains_handler_bridge_impl() {
    let surface = make_fixture_surface();
    let config = make_test_config();
    let output = gen_service_rs(&surface, &config);
    assert!(
        output.contains("impl my_crate::RequestHandler for PhpRequestHandlerBridge"),
        "expected trait impl:\n{output}"
    );
    assert!(
        output.contains("fn handle(") && output.contains("Pin<Box<dyn std::future::Future<Output"),
        "expected boxed-future dispatch method:\n{output}"
    );
}

/// `gen_service_rs` emits the `#[php_function]` run entry point.
#[test]
fn rust_output_contains_php_function_run() {
    let surface = make_fixture_surface();
    let config = make_test_config();
    let output = gen_service_rs(&surface, &config);
    assert!(
        output.contains("#[php_function]"),
        "expected `#[php_function]` attribute:\n{output}"
    );
    assert!(
        output.contains("pub fn test_service_run("),
        "expected `test_service_run` function:\n{output}"
    );
}

/// `gen_service_rs` emits registration dispatch via `match method_name`.
#[test]
fn rust_output_contains_registration_dispatch() {
    let surface = make_fixture_surface();
    let config = make_test_config();
    let output = gen_service_rs(&surface, &config);
    assert!(
        output.contains("\"add_handler\""),
        "expected `\"add_handler\"` match arm:\n{output}"
    );
    assert!(
        output.contains("Arc<dyn my_crate::RequestHandler>"),
        "expected Arc wrapping of handler:\n{output}"
    );
}

/// Full `generate()` call returns two files when services are non-empty.
#[test]
fn generate_returns_two_files_for_non_empty_services() {
    let surface = make_fixture_surface();
    let config = make_test_config();
    let files = generate(&surface, &config).expect("generate should not fail");
    assert_eq!(files.len(), 2, "expected 2 generated files, got {}", files.len());
    let paths: Vec<&str> = files
        .iter()
        .map(|f| f.path.file_name().unwrap().to_str().unwrap())
        .collect();
    assert!(paths.contains(&"service.rs"), "expected service.rs in output");
    assert!(paths.contains(&"Service.php"), "expected Service.php in output");
}

/// Full `generate()` returns empty for a surface with no services.
#[test]
fn generate_returns_empty_for_no_services() {
    let surface = ApiSurface::default();
    let config = make_test_config();
    let files = generate(&surface, &config).expect("generate should not fail");
    assert!(files.is_empty(), "expected no files for surface without services");
}

/// Verify that required &str/String parameters emit non-nullable PHP signatures,
/// while Option<T> parameters emit nullable signatures with = null defaults.
/// This is a regression test for the over-propagation of nullable params.
#[test]
fn php_output_required_params_not_nullable() {
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

    let service = ServiceDef {
        name: "TestService".to_owned(),
        rust_path: "my_crate::TestService".to_owned(),
        constructor,
        configurators: vec![],
        registrations: vec![],
        entrypoints: vec![EntrypointDef {
            method: "extract".to_owned(),
            kind: EntrypointKind::Run,
            is_async: false,
            params: vec![
                ParamDef {
                    name: "path".to_owned(),
                    ty: TypeRef::String,
                    optional: false,
                    default: None,
                    ..ParamDef::default()
                },
                ParamDef {
                    name: "mime_type".to_owned(),
                    ty: TypeRef::Optional(Box::new(TypeRef::String)),
                    optional: true,
                    default: None,
                    ..ParamDef::default()
                },
            ],
            return_type: TypeRef::Unit,
            error_type: None,
            doc: String::new(),
        }],
        doc: String::new(),
        cfg: None,
    };

    let api = ApiSurface {
        crate_name: "my_crate".to_owned(),
        version: "0.1.0".to_owned(),
        services: vec![service],
        handler_contracts: vec![],
        ..ApiSurface::default()
    };

    let output = gen_service_php(&api, "my_crate");

    assert!(
        output.contains("string $path,"),
        "required path param must be non-nullable: {output}"
    );

    assert!(
        output.contains("?string $mime_type = null"),
        "Option<T> mime_type param must be nullable with = null: {output}"
    );

    assert!(
        !output.contains("?string $path"),
        "required path must not be nullable: {output}"
    );
}

/// `gen_registration_variant` with a `wrapper_call` emits wrapper construction
/// and delegates to the base method instead of pushing to `$this->registrations[]`.
#[test]
fn php_output_wrapper_call_delegates_to_base_method() {
    use crate::core::ir::{
        ParamDef, RegistrationVariant, RegistrationVariantStyle, TypeRef, WrapperConstructorArg, WrapperConstructorCall,
    };

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
        error_type: None,
        doc: String::new(),
        variants: vec![RegistrationVariant {
            name: "GET".to_owned(),
            overrides: vec![],
            wrapper_call: Some(WrapperConstructorCall {
                metadata_param: "builder".to_owned(),
                wrapper_type_path: "my_crate::RouteBuilder".to_owned(),
                wrapper_type_name: "RouteBuilder".to_owned(),
                constructor_method: "new".to_owned(),
                args: vec![
                    WrapperConstructorArg::Fixed {
                        param_name: "method".to_owned(),
                        value_expr: "my_crate::Method::Get".to_owned(),
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
            doc: Some("Register a GET route.".to_owned()),
            style: RegistrationVariantStyle::Hybrid,
            ..Default::default()
        }],
        ..Default::default()
    };

    let service = ServiceDef {
        name: "Router".to_owned(),
        rust_path: "my_crate::Router".to_owned(),
        constructor,
        configurators: vec![],
        registrations: vec![registration],
        entrypoints: vec![],
        doc: String::new(),
        cfg: None,
    };

    let api = ApiSurface {
        crate_name: "my_crate".to_owned(),
        version: "0.1.0".to_owned(),
        services: vec![service],
        handler_contracts: vec![],
        ..ApiSurface::default()
    };

    let output = gen_service_php(&api, "my_crate");

    assert!(
        output.contains("$builder = RouteBuilder::new(Method::Get, $path);"),
        "expected wrapper construction statement:\n{output}"
    );

    assert!(
        output.contains("return $this->route($builder, $handler);"),
        "expected delegation to base route() method:\n{output}"
    );

    assert!(
        !output.contains("$this->registrations[] = ['route', [], $handler]"),
        "must not push empty metadata to registrations[]:\n{output}"
    );
}

fn make_test_config() -> ResolvedCrateConfig {
    use crate::core::config::resolved::ResolvedCrateConfig;
    ResolvedCrateConfig {
        name: "my-crate".to_owned(),
        ..ResolvedCrateConfig::default()
    }
}
