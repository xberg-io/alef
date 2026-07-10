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

    let get_variant = crate::core::ir::RegistrationVariant {
        name: "get".to_owned(),
        overrides: vec![crate::core::ir::RegistrationVariantOverride {
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
        doc: Some("Register a GET handler.".to_owned()),
        style: Default::default(),
        ..Default::default()
    };

    let registration = RegistrationDef {
        method: "add_handler".to_owned(),
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
        error_type: Some("HandlerError".to_owned()),
        doc: "Register a request handler.".to_owned(),
        variants: vec![get_variant],
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
fn test_gen_service_go_produces_valid_go() {
    let api = make_fixture_surface();
    let config = ResolvedCrateConfig {
        name: "test_crate".to_owned(),
        ..ResolvedCrateConfig::default()
    };

    let go = gen_service_go(&api, &config, "binding", "TEST_CRATE");

    assert!(go.contains("package binding"));
    assert!(go.contains("TestService"));
    assert!(go.contains("NewTestService"));
    assert!(go.contains("RegisterAddHandler"));
    assert!(go.contains("Run"));
    assert!(go.contains("HandlerFunc"));
    assert!(go.contains("handlerRegistry"));
    assert!(go.contains("service_handler_callback"));
    assert!(go.contains("/*\n#include <string.h>"));
    assert!(go.contains("#include \"test_crate.h\""));
    assert!(go.contains("//export service_handler_callback"));
    assert!(go.contains("import \"C\""));
    assert!(go.contains("*TEST_CRATETestServiceOpaque"));
}

#[test]
fn test_service_struct_is_generated() {
    let api = make_fixture_surface();
    let config = ResolvedCrateConfig {
        name: "test_crate".to_owned(),
        ..ResolvedCrateConfig::default()
    };

    let go = gen_service_go(&api, &config, "binding", "TEST_CRATE");

    assert!(go.contains("type TestService struct"));
    assert!(go.contains("owner unsafe.Pointer"));
    assert!(go.contains("*TEST_CRATETestServiceOpaque"));
    assert!(go.contains("mu    sync.Mutex"));
}

#[test]
fn test_constructor_is_generated() {
    let api = make_fixture_surface();
    let config = ResolvedCrateConfig {
        name: "test_crate".to_owned(),
        ..ResolvedCrateConfig::default()
    };

    let go = gen_service_go(&api, &config, "binding", "test_crate");

    assert!(go.contains("func NewTestService()"));
    assert!(go.contains("test_crate_test_service_new"));
}

#[test]
fn test_registration_method_exists() {
    let api = make_fixture_surface();
    let config = ResolvedCrateConfig {
        name: "test_crate".to_owned(),
        ..ResolvedCrateConfig::default()
    };

    let go = gen_service_go(&api, &config, "binding", "test_crate");

    assert!(go.contains("RegisterAddHandler"));
    assert!(go.contains("handler HandlerFunc"));
    assert!(go.contains("registerHandler(handler)"));
    assert!(go.contains("C.get_service_handler_callback(),"));
    assert!(go.contains("(*C.TEST_CRATETestServiceOpaque)"));
}

#[test]
fn test_entrypoint_method_exists() {
    let api = make_fixture_surface();
    let config = ResolvedCrateConfig {
        name: "test_crate".to_owned(),
        ..ResolvedCrateConfig::default()
    };

    let go = gen_service_go(&api, &config, "binding", "test_crate");

    assert!(go.contains("func (s *TestService) Run("));
    assert!(go.contains("test_crate_test_service_ep_run"));
    assert!(go.contains("(*C.TEST_CRATETestServiceOpaque)"));
}

#[test]
fn test_handler_registry_and_trampoline() {
    let api = make_fixture_surface();
    let config = ResolvedCrateConfig {
        name: "test_crate".to_owned(),
        ..ResolvedCrateConfig::default()
    };

    let go = gen_service_go(&api, &config, "binding", "test_crate");

    assert!(go.contains("handlerRegistry"));
    assert!(go.contains("service_handler_callback"));
    assert!(go.contains("invokeHandler"));
    assert!(go.contains("registerHandler"));
    assert!(go.contains("//export service_handler_callback"));
}

#[test]
fn test_c_ffi_imports_generated() {
    let api = make_fixture_surface();
    let config = ResolvedCrateConfig {
        name: "test_crate".to_owned(),
        ..ResolvedCrateConfig::default()
    };

    let go = gen_service_go(&api, &config, "binding", "test_crate");

    assert!(go.contains("test_crate_test_service_new"));
    assert!(go.contains("test_crate_test_service_free"));
    assert!(go.contains("test_crate_test_service_register_add_handler"));
}

#[test]
fn test_registration_variant_method_exists() {
    let api = make_fixture_surface();
    let config = ResolvedCrateConfig {
        name: "test_crate".to_owned(),
        ..ResolvedCrateConfig::default()
    };

    let go = gen_service_go(&api, &config, "binding", "test_crate");

    assert!(go.contains("func (s *TestService) Get("));
    assert!(go.contains("handler HandlerFunc"));
    assert!(go.contains("path string"));
    assert!(go.contains("C.test_crate_test_service_get"));
    assert!(!go.contains("C.test_crate_test_service_add_handler_get"));
    assert!(go.contains("C.CString(path)"));
}

#[test]
fn test_start_background_method_exists() {
    let api = make_fixture_surface();
    let config = ResolvedCrateConfig {
        name: "test_crate".to_owned(),
        ..ResolvedCrateConfig::default()
    };

    let go = gen_service_go(&api, &config, "binding", "test_crate");

    assert!(go.contains("func (s *TestService) StartBackground("));
    assert!(go.contains("type ServerHandle struct"));
    assert!(go.contains("func (h *ServerHandle) Stop()"));
    assert!(go.contains("host string, port uint16"));
    assert!(go.contains("*ServerHandle, error"));
}

#[test]
fn test_registration_variant_wrapper_call_emits_free_args() {
    use crate::core::ir::{WrapperConstructorArg, WrapperConstructorCall};

    let mut api = make_fixture_surface();
    let svc = &mut api.services[0];
    let reg = &mut svc.registrations[0];

    reg.variants[0] = crate::core::ir::RegistrationVariant {
        name: "get".to_owned(),
        overrides: vec![crate::core::ir::RegistrationVariantOverride {
            param_name: "method".to_owned(),
            value_expr: "\"GET\"".to_owned(),
        }],
        wrapper_call: Some(WrapperConstructorCall {
            metadata_param: "builder".to_owned(),
            wrapper_type_path: "test_crate::RouteBuilder".to_owned(),
            wrapper_type_name: "RouteBuilder".to_owned(),
            constructor_method: "new".to_owned(),
            args: vec![
                WrapperConstructorArg::Fixed {
                    param_name: "method".to_owned(),
                    value_expr: "\"GET\"".to_owned(),
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

    let config = ResolvedCrateConfig {
        name: "test_crate".to_owned(),
        ..ResolvedCrateConfig::default()
    };
    let go = gen_service_go(&api, &config, "binding", "test_crate");

    assert!(go.contains("C.CString(path)"), "missing CString(path) in:\n{go}");
    assert!(!go.contains("\"GET\""), "fixed arg must not be re-emitted:\n{go}");
}
