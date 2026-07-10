use super::*;
use crate::core::ir::{
    EntrypointDef, EntrypointKind, HandlerContractDef, MethodDef, ParamDef, PrimitiveType, RegistrationDef, ServiceDef,
    TypeRef,
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

    use crate::core::ir::{RegistrationVariant, WrapperConstructorArg, WrapperConstructorCall};

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
        variants: vec![RegistrationVariant {
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
            doc: Some("Register a GET handler for a path.".to_owned()),
            style: crate::core::ir::RegistrationVariantStyle::Hybrid,
            ..Default::default()
        }],
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

/// `gen_service_rb` emits a class named after the service owner.
#[test]
fn ruby_output_contains_service_class() {
    let surface = make_fixture_surface();
    let output = gen_service_rb(&surface, "MyCrate", "my_crate");
    assert!(
        output.contains("class TestService"),
        "expected `class TestService` in output:\n{output}"
    );
}

/// `gen_service_rb` emits `initialize` with registrations state init.
#[test]
fn ruby_output_contains_initialize_with_registrations() {
    let surface = make_fixture_surface();
    let output = gen_service_rb(&surface, "MyCrate", "my_crate");
    assert!(
        output.contains("def initialize"),
        "expected `def initialize` in output:\n{output}"
    );
    assert!(
        output.contains("@registrations = []"),
        "expected `@registrations = []` in output:\n{output}"
    );
}

/// `gen_service_rb` emits configurator methods that return `self`.
#[test]
fn ruby_output_contains_configurator() {
    let surface = make_fixture_surface();
    let output = gen_service_rb(&surface, "MyCrate", "my_crate");
    assert!(
        output.contains("def with_timeout(timeout_ms)"),
        "expected `with_timeout` configurator with positional param:\n{output}"
    );
    assert!(
        output.contains("self"),
        "expected `self` return in configurator:\n{output}"
    );
}

/// `gen_service_rb` emits a registration method accepting a block.
#[test]
fn ruby_output_contains_registration_block_param() {
    let surface = make_fixture_surface();
    let output = gen_service_rb(&surface, "MyCrate", "my_crate");
    assert!(
        output.contains("def add_handler("),
        "expected `add_handler` registration method:\n{output}"
    );
    assert!(
        output.contains("&block"),
        "expected `&block` parameter in registration:\n{output}"
    );
    assert!(
        output.contains("@registrations.push"),
        "expected `@registrations.push` in registration:\n{output}"
    );
}

/// `gen_service_rb` emits registration variant shortcut methods.
#[test]
fn ruby_output_contains_registration_variant() {
    let surface = make_fixture_surface();
    let output = gen_service_rb(&surface, "MyCrate", "my_crate");
    assert!(
        output.contains("def get("),
        "expected `def get(` variant method:\n{output}"
    );
    assert!(
        output.contains("&block"),
        "expected `&block` parameter in variant:\n{output}"
    );
    assert!(
        output.contains("@registrations.push"),
        "expected `@registrations.push` in variant:\n{output}"
    );
}

/// `gen_service_rb` emits the `run` entrypoint.
#[test]
fn ruby_output_contains_run_entrypoint() {
    let surface = make_fixture_surface();
    let output = gen_service_rb(&surface, "MyCrate", "my_crate");
    assert!(output.contains("def run("), "expected `def run(` entrypoint:\n{output}");
    assert!(
        output.contains(".test_service_run("),
        "expected native call `.test_service_run(` in run:\n{output}"
    );
}

/// `gen_service_rs` emits the handler bridge struct.
#[test]
fn rust_output_contains_handler_bridge_struct() {
    let surface = make_fixture_surface();
    let config = make_test_config();
    let output = gen_service_rs(&surface, &config);
    assert!(
        output.contains("pub struct RbRequestHandlerBridge"),
        "expected `RbRequestHandlerBridge` struct:\n{output}"
    );
}

/// `gen_service_rs` emits the handler bridge trait impl.
#[test]
fn rust_output_contains_handler_bridge_impl() {
    let surface = make_fixture_surface();
    let config = make_test_config();
    let output = gen_service_rs(&surface, &config);
    assert!(
        output.contains("impl my_crate::RequestHandler for RbRequestHandlerBridge"),
        "expected trait impl:\n{output}"
    );
    assert!(
        output.contains("fn handle(") && output.contains("Pin<Box<dyn std::future::Future<Output"),
        "expected boxed-future dispatch method:\n{output}"
    );
}

/// `gen_service_rs` emits GVL handling via Ruby::get() for #[magnus::function] callbacks
/// and rb_sys for async handler bridge contexts.
#[test]
fn rust_output_contains_gvl_handling() {
    let surface = make_fixture_surface();
    let config = make_test_config();
    let output = gen_service_rs(&surface, &config);
    let ruby_get_count = output.matches("Ruby::get()").count();
    assert!(
        ruby_get_count >= 2,
        "expected at least 2 `Ruby::get()` calls (main function + GVL callback): count={}, output:\n{}",
        ruby_get_count,
        output
    );
    assert!(
        output.contains("rb_sys::rb_thread_call_with_gvl"),
        "expected `rb_sys::rb_thread_call_with_gvl` for handler bridge GVL:\n{output}"
    );
}

/// `gen_service_rs` emits the run entry point registered via `function!`.
#[test]
fn rust_output_contains_magnus_function_run() {
    let surface = make_fixture_surface();
    let config = make_test_config();
    let output = gen_service_rs(&surface, &config);
    assert!(
        output.contains("function! macro callbacks run on a Ruby thread"),
        "expected `function!` callback handling in run entry point:\n{output}"
    );
    assert!(
        output.contains("pub fn test_service_run("),
        "expected `test_service_run` function:\n{output}"
    );
    assert!(
        output.contains("rb_sys::rb_thread_call_without_gvl"),
        "expected `rb_thread_call_without_gvl` for GVL-safe async run:\n{output}"
    );
    assert!(
        output.contains("new_current_thread"),
        "expected `new_current_thread` Tokio runtime in run:\n{output}"
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

/// `gen_service_rs` emits variant match arms.
#[test]
fn rust_output_contains_variant_dispatch() {
    let surface = make_fixture_surface();
    let config = make_test_config();
    let output = gen_service_rs(&surface, &config);
    assert!(
        output.contains("\"get\""),
        "expected `\"get\"` variant match arm:\n{output}"
    );
    assert!(
        output.contains("RouteBuilder::new"),
        "expected `RouteBuilder::new` wrapper constructor:\n{output}"
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
    assert!(paths.contains(&"service.rb"), "expected service.rb in output");
}

/// Full `generate()` returns empty for a surface with no services.
#[test]
fn generate_returns_empty_for_no_services() {
    let surface = ApiSurface::default();
    let config = make_test_config();
    let files = generate(&surface, &config).expect("generate should not fail");
    assert!(files.is_empty(), "expected no files for surface without services");
}

fn make_test_config() -> ResolvedCrateConfig {
    use crate::core::config::resolved::ResolvedCrateConfig;
    ResolvedCrateConfig {
        name: "my-crate".to_owned(),
        ..ResolvedCrateConfig::default()
    }
}
