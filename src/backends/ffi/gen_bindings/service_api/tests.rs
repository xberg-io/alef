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
fn test_gen_service_rs_produces_valid_rust() {
    let api = make_fixture_surface();
    let config = ResolvedCrateConfig {
        name: "test_crate".to_owned(),
        ..ResolvedCrateConfig::default()
    };

    let rs = gen_service_rs(&api, &config);

    assert!(rs.contains("#[no_mangle]"));
    assert!(rs.contains("extern \"C\""));
    assert!(rs.contains("TestServiceOpaque"));
    assert!(rs.contains("test_service_new"));
    assert!(rs.contains("test_service_free"));
    assert!(rs.contains("FfiRequestHandlerBridge"));
    assert!(rs.contains("Pin<Box<dyn std::future::Future"));
}

#[test]
fn test_handler_bridge_struct_is_generated() {
    let api = make_fixture_surface();
    let config = ResolvedCrateConfig {
        name: "test_crate".to_owned(),
        ..ResolvedCrateConfig::default()
    };

    let rs = gen_service_rs(&api, &config);

    assert!(rs.contains("struct FfiRequestHandlerBridge"));
    assert!(rs.contains("callback: extern \"C\" fn"));
    assert!(rs.contains("context: *mut c_void"));
}

#[test]
fn test_opaque_has_constructor_and_destructor() {
    let api = make_fixture_surface();
    let config = ResolvedCrateConfig {
        name: "test_crate".to_owned(),
        ..ResolvedCrateConfig::default()
    };

    let rs = gen_service_rs(&api, &config);

    assert!(rs.contains("pub extern \"C\" fn test_crate_test_service_new()"));
    assert!(rs.contains("pub extern \"C\" fn test_crate_test_service_free"));
}

#[test]
fn test_registration_function_exists() {
    let api = make_fixture_surface();
    let config = ResolvedCrateConfig {
        name: "test_crate".to_owned(),
        ..ResolvedCrateConfig::default()
    };

    let rs = gen_service_rs(&api, &config);

    assert!(rs.contains("test_crate_test_service_register_add_handler"));
    assert!(rs.contains("extern \"C\" fn(*mut c_void, *const c_char) -> *mut c_char"));
}

#[test]
fn test_entrypoint_function_exists() {
    let api = make_fixture_surface();
    let config = ResolvedCrateConfig {
        name: "test_crate".to_owned(),
        ..ResolvedCrateConfig::default()
    };

    let rs = gen_service_rs(&api, &config);

    assert!(rs.contains("test_crate_test_service_ep_run"));
    assert!(rs.contains("tokio::runtime::Runtime"));
}

#[test]
fn test_service_header_declares_metadata_and_entrypoint_params() {
    let api = make_fixture_surface();
    let header = gen_service_h(&api, "test_crate");

    assert!(
        header.contains("handler_callback_t callback,\n    void* context,\n    const char* path\n);"),
        "registration metadata param missing from service header:\n{header}"
    );
    assert!(
        header.contains(
            "test_crate_test_service_ep_run(\n    test_crateTestServiceOpaque* owner,\n    const char* addr\n);"
        ),
        "entrypoint param missing from service header:\n{header}"
    );
}

fn make_surface_with_variant() -> ApiSurface {
    use crate::core::ir::{
        ParamDef, RegistrationVariant, RegistrationVariantOverride, WrapperConstructorArg, WrapperConstructorCall,
    };

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

    let get_variant = RegistrationVariant {
        name: "get".to_owned(),
        overrides: vec![RegistrationVariantOverride {
            param_name: "method".to_owned(),
            value_expr: "my_crate::Method::GET".to_owned(),
        }],
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
        method: "add_route".to_owned(),
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
        doc: "Register a route.".to_owned(),
        variants: vec![get_variant],
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
        crate_name: "my_crate".to_owned(),
        version: "1.0.0".to_owned(),
        services: vec![ServiceDef {
            name: "App".to_owned(),
            rust_path: "my_crate::App".to_owned(),
            constructor,
            configurators: vec![],
            registrations: vec![registration],
            entrypoints: vec![],
            doc: "App service.".to_owned(),
            cfg: None,
        }],
        handler_contracts: vec![handler_contract],
        ..ApiSurface::default()
    }
}

#[test]
fn test_variant_fn_is_emitted() {
    let api = make_surface_with_variant();
    let config = ResolvedCrateConfig {
        name: "my_crate".to_owned(),
        ..ResolvedCrateConfig::default()
    };

    let rs = gen_service_rs(&api, &config);

    assert!(
        rs.contains("fn my_crate_app_get("),
        "expected variant fn my_crate_app_get not found in:\n{rs}"
    );
}

#[test]
fn test_variant_fn_has_no_mangle_and_extern_c() {
    let api = make_surface_with_variant();
    let config = ResolvedCrateConfig {
        name: "my_crate".to_owned(),
        ..ResolvedCrateConfig::default()
    };

    let rs = gen_service_rs(&api, &config);

    let variant_start = rs.find("fn my_crate_app_get(").expect("variant fn not found");
    let preamble = &rs[..variant_start];
    let preamble_tail = preamble.rsplit("#[no_mangle]").next().unwrap_or(preamble);
    assert!(
        preamble.contains("#[no_mangle]"),
        "#[no_mangle] must precede the variant fn"
    );
    assert!(
        preamble_tail.trim().starts_with("pub extern") || preamble_tail.trim().starts_with("pub unsafe extern"),
        "#[no_mangle] must directly precede the extern fn (intervening: `{preamble_tail}`)"
    );
}

#[test]
fn test_variant_fn_has_free_param_and_wrapper_construction() {
    let api = make_surface_with_variant();
    let config = ResolvedCrateConfig {
        name: "my_crate".to_owned(),
        ..ResolvedCrateConfig::default()
    };

    let rs = gen_service_rs(&api, &config);

    assert!(
        rs.contains("path: *const c_char"),
        "free param 'path' missing from variant signature"
    );
    assert!(
        rs.contains("my_crate::Method::GET"),
        "fixed arg my_crate::Method::GET missing from wrapper construction"
    );
    assert!(
        rs.contains("my_crate::RouteBuilder::new("),
        "wrapper constructor call not emitted"
    );
    assert!(
        rs.contains("owner_ref.add_route(builder, handler)"),
        "variant dispatch call must pass wrapper metadata before handler"
    );
}

#[test]
fn test_variant_fn_has_null_check_for_owner() {
    let api = make_surface_with_variant();
    let config = ResolvedCrateConfig {
        name: "my_crate".to_owned(),
        ..ResolvedCrateConfig::default()
    };

    let rs = gen_service_rs(&api, &config);

    let start = rs.find("fn my_crate_app_get(").expect("variant fn not found");
    let body = &rs[start..];
    assert!(
        body.contains("if owner.is_null()"),
        "owner null check missing from variant fn"
    );
}

#[test]
fn test_variant_without_wrapper_call_is_not_emitted() {
    use crate::core::ir::{ParamDef, RegistrationVariant, RegistrationVariantOverride};

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

    let plain_variant = RegistrationVariant {
        name: "plain".to_owned(),
        overrides: vec![RegistrationVariantOverride {
            param_name: "path".to_owned(),
            value_expr: "\"/fixed\"".to_owned(),
        }],
        wrapper_call: None,
        signature_params: vec![],
        doc: None,
        style: Default::default(),
        ..Default::default()
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
        doc: String::new(),
        variants: vec![plain_variant],
        ..Default::default()
    };

    let handler_contract = HandlerContractDef {
        trait_name: "RequestHandler".to_owned(),
        rust_path: "my_crate::RequestHandler".to_owned(),
        dispatch: MethodDef {
            name: "handle".to_owned(),
            params: vec![],
            return_type: TypeRef::Unit,
            is_async: false,
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
        wire_request_type: None,
        wire_response_type: None,
        dispatch_extra_params: vec![],
        wire_param_name: None,
        dispatch_return_type: None,
        response_adapter: None,
        doc: String::new(),
    };

    let api = ApiSurface {
        crate_name: "my_crate".to_owned(),
        version: "1.0.0".to_owned(),
        services: vec![ServiceDef {
            name: "App".to_owned(),
            rust_path: "my_crate::App".to_owned(),
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
        name: "my_crate".to_owned(),
        ..ResolvedCrateConfig::default()
    };
    let rs = gen_service_rs(&api, &config);

    assert!(
        !rs.contains("fn my_crate_app_plain("),
        "plain variant without wrapper_call must not emit a C symbol"
    );
}

/// Configurator functions must take the owner's inner field out, call the
/// consuming method, and put the result back. The opaque handle stores the owner
/// as `Option<Box<OwnerType>>`, so the generator must emit
/// `let inner = match (*owner).inner.take() { Some(boxed) => *boxed, None => ... };`
/// followed by `(*owner).inner = Some(Box::new(inner.method(args)));`.
#[test]
fn configurator_function_unboxes_and_reboxes_inner() {
    use crate::core::ir::{MethodDef, ParamDef, ReceiverKind, ServiceDef, TypeRef};

    let configurator = MethodDef {
        name: "setup".to_owned(),
        params: vec![ParamDef {
            name: "opts".to_owned(),
            ty: TypeRef::Named("Options".to_owned()),
            optional: false,
            default: None,
            ..ParamDef::default()
        }],
        return_type: TypeRef::Named("Worker".to_owned()),
        is_async: false,
        is_static: false,
        error_type: None,
        doc: String::new(),
        receiver: Some(ReceiverKind::Owned),
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
    let constructor = MethodDef {
        name: "new".to_owned(),
        params: vec![],
        return_type: TypeRef::Named("Worker".to_owned()),
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
    let api = ApiSurface {
        crate_name: "worker_crate".to_owned(),
        version: "1.0.0".to_owned(),
        services: vec![ServiceDef {
            name: "Worker".to_owned(),
            rust_path: "worker_crate::Worker".to_owned(),
            constructor,
            configurators: vec![configurator],
            registrations: vec![],
            entrypoints: vec![],
            doc: String::new(),
            cfg: None,
        }],
        handler_contracts: vec![],
        ..ApiSurface::default()
    };
    let config = ResolvedCrateConfig {
        name: "worker_crate".to_owned(),
        ..ResolvedCrateConfig::default()
    };
    let rs = gen_service_rs(&api, &config);

    assert!(
        rs.contains("fn worker_crate_worker_setup("),
        "configurator fn must be emitted; got:\n{rs}"
    );
    assert!(
        rs.contains("let inner = match (*owner).inner.take()"),
        "configurator must `take()` owner.inner before calling the consuming method; got:\n{rs}"
    );
    assert!(
        rs.contains("(*owner).inner = Some(Box::new(inner.setup("),
        "configurator must re-box the result and assign to owner.inner; got:\n{rs}"
    );
}

/// Regression test for builder/config double-free bug (alef issue #TBD).
/// FFI registration functions that accept a builder or config pointer must
/// NOT transfer ownership (Box::from_raw) since the C caller still holds the
/// pointer and will call _free() or a deferred finalizer afterwards. Instead,
/// borrow the pointer as a reference (&*ptr).
///
/// Previously the emitted code was:
///   let builder = unsafe { *Box::from_raw(builder) };
/// which dropped the builder at function end, causing a double-free when
/// Java's finalizer or C's deferred _free() ran on the same pointer.
///
/// The fix borrows instead:
///   let builder = unsafe { &*builder };
/// The C caller retains ownership and responsibility for freeing.
#[test]
fn registration_function_does_not_consume_builder_ownership() {
    let api = make_fixture_surface();
    let config = ResolvedCrateConfig {
        name: "test_crate".to_owned(),
        ..ResolvedCrateConfig::default()
    };

    let rs = gen_service_rs(&api, &config);

    assert!(
        !rs.contains("*Box::from_raw(path)"),
        "registration function must not use Box::from_raw on metadata params; got:\n{rs}"
    );
    assert!(
        rs.contains("CStr::from_ptr(path)"),
        "registration function must convert string params via CStr::from_ptr; got:\n{rs}"
    );
}

/// Regression test: when a registration carries a `TypeRef::Named` metadata
/// param backed by a public `TypeDef` (i.e. an opaque pointer with `_new` /
/// `_free` exports), the conversion borrows the pointer (`unsafe { &*ptr }`)
/// AND the call site clones the borrow so the consuming Rust API can take
/// the value by ownership.
///
/// The borrow alone (without `.clone()`) was introduced in 16279dba9 to fix a
/// double-free, but it broke compilation: downstream methods like
/// `App::route(builder: RouteBuilder, ...)` and `App::config(config:
/// ServerConfig) -> Self` consume `T` by value, so passing `&T` produced
/// `error[E0308]: mismatched types`. The fix is to emit `.clone()` at the
/// call site (every opaque type wired through this path must derive `Clone`).
///
/// This test fails if either:
///   - the borrow is missing (double-free regression — alef 0.25.5)
///   - the `.clone()` is missing on the call-site arg expression
///     (E0308 regression — alef 0.25.5..=0.25.18)
#[test]
fn registration_named_opaque_param_clones_borrowed_pointer_at_call_site() {
    use crate::core::ir::TypeDef;

    let mut api = make_surface_with_variant();
    api.types.push(TypeDef {
        name: "RouteBuilder".to_owned(),
        rust_path: "my_crate::RouteBuilder".to_owned(),
        is_opaque: true,
        is_clone: true,
        ..TypeDef::default()
    });
    let config = ResolvedCrateConfig {
        name: "my_crate".to_owned(),
        ..ResolvedCrateConfig::default()
    };

    let rs = gen_service_rs(&api, &config);

    assert!(
        rs.contains("let builder = unsafe { &*builder };"),
        "opaque-pointer metadata param `builder` must be borrowed via &*ptr; got:\n{rs}"
    );
    assert!(
        rs.contains(".add_route(builder.clone(), handler)"),
        "opaque-pointer metadata param `builder` must be `.clone()`d at the \
         registration dispatch call site so the consuming Rust API receives \
         `T`, not `&T`; got:\n{rs}"
    );
    assert!(
        !rs.contains("*Box::from_raw(builder)"),
        "opaque-pointer metadata param `builder` must not be consumed via \
         `Box::from_raw` — the C caller still holds the pointer; got:\n{rs}"
    );
}
