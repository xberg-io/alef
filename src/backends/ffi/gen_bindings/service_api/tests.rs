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
        cfg: None,
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
            cfg: None,
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

    assert!(rs.contains("#[unsafe(no_mangle)]"));
    assert!(rs.contains("extern \"C\""));
    // The host's matching response deallocator is part of the registration contract, so the
    // bridge must not emit a process-global `free` shim whose allocator may differ on Windows.
    assert!(
        !rs.contains("fn free(ptr: *mut c_void)"),
        "service bridge must not emit a C-runtime free shim"
    );
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
fn test_handler_bridge_frees_response_before_deserializing() {
    // Regression: the response pointer from the C callback used to leak on a deserialization
    // failure. `serde_json::from_str(...)?` ran before releasing resp_ptr, so a malformed response
    // returned early via `?` and the host-allocated buffer was never released. free() must run
    // unconditionally, ahead of the fallible parse, so every path (success or failure) releases
    // ownership of the pointer. ~keep
    let api = make_fixture_surface();
    let config = ResolvedCrateConfig {
        name: "test_crate".to_owned(),
        ..ResolvedCrateConfig::default()
    };

    let rs = gen_service_rs(&api, &config);

    let free_pos = rs
        .find("(self.response_free)(resp_ptr);")
        .expect("handler bridge must free the response pointer");
    let parse_pos = rs
        .find("serde_json::from_str(&resp_json)")
        .expect("handler bridge must deserialize the response");

    assert!(
        free_pos < parse_pos,
        "resp_ptr must be freed before the fallible deserialize, otherwise a malformed \
         response leaks the C-allocated buffer:\n{rs}"
    );

    // Freeing before the parse is only sound because the response was copied out of the C buffer
    // first. Without `into_owned()` the borrow would still point into the freed allocation and the
    // ordering asserted above would turn the leak into a use-after-free — a strictly worse defect
    // that the ordering assertion alone cannot see. ~keep
    let owned_pos = rs
        .find(".to_string_lossy().into_owned()")
        .expect("handler bridge must copy the response out of the C buffer before freeing it");

    assert!(
        owned_pos < free_pos,
        "the response must be copied into an owned String before releasing resp_ptr; otherwise \
         `resp_json` borrows freed memory:\n{rs}"
    );
}

#[test]
fn test_handler_bridge_uses_host_response_deallocator() {
    let api = make_fixture_surface();
    let config = ResolvedCrateConfig {
        name: "test_crate".to_owned(),
        ..ResolvedCrateConfig::default()
    };

    let rs = gen_service_rs(&api, &config);

    assert!(
        rs.contains("response_free: extern \"C\" fn(*mut c_char)"),
        "service bridge must retain the host allocator's matching deallocator:\n{rs}"
    );
    assert!(
        rs.contains("(self.response_free)(resp_ptr);"),
        "service bridge must release callback responses through the host deallocator:\n{rs}"
    );
    assert!(
        !rs.contains("fn free(ptr: *mut c_void);"),
        "service bridge must not assume host responses came from the C allocator:\n{rs}"
    );
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
    assert!(rs.contains("response_free: extern \"C\" fn(*mut c_char)"));
}

#[test]
fn registration_dispatch_preserves_domain_error_type_and_compiles() {
    let dispatch = render(
        "service_api_registration_dispatch_result.rs.jinja",
        minijinja::context! {
            method_name => "route",
            meta_args => "",
            opaque_name => "ServiceOpaque",
        },
    );
    let source = format!(
        r#"
#[derive(Debug)]
struct DomainError;
impl std::fmt::Display for DomainError {{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{
        write!(f, "domain error")
    }}
}}
#[derive(Debug)]
struct HandleError;
struct Application;
struct Owner;
impl Owner {{
    fn route(&mut self, _: Handler) -> Result<&mut Application, DomainError> {{
        Err(DomainError)
    }}
}}
struct ServiceOpaque {{ inner: Option<Owner> }}
struct Handler;
fn set_handle_error(_: &HandleError) {{}}
static LAST_ERROR: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
fn set_last_error(_: i32, message: &str) {{
    *LAST_ERROR.lock().unwrap() = Some(message.to_owned());
}}
fn with_handle_mut<T, R>(_: u64, body: impl FnOnce(&mut T) -> R) -> Result<R, HandleError> {{
    let mut value = ServiceOpaque {{ inner: Some(Owner) }};
    let erased = (&mut value as *mut ServiceOpaque).cast::<T>();
    // SAFETY: this compile-only harness instantiates T as ServiceOpaque. ~keep
    Ok(body(unsafe {{ &mut *erased }}))
}}
fn register(owner: u64, handler: Handler) -> i32 {{
{dispatch}
}}
fn main() {{
    assert_eq!(register(1, Handler), 1);
    assert_eq!(LAST_ERROR.lock().unwrap().as_deref(), Some("domain error"));
}}
"#
    );
    let directory = tempfile::tempdir().expect("temporary directory");
    let source_path = directory.path().join("service_registration.rs");
    let binary_path = directory.path().join("service-registration-test");
    std::fs::write(&source_path, source).expect("write compile harness");
    let compile = std::process::Command::new("rustc")
        .current_dir(directory.path())
        .args(["--edition=2024", "-o"])
        .arg(&binary_path)
        .arg(&source_path)
        .output()
        .expect("run rustc");
    assert!(compile.status.success(), "{}", String::from_utf8_lossy(&compile.stderr));
}

/// Regression: a fallible entrypoint returning a plain status code used to discard the
/// domain error entirely (`Err(_) => 1`), so the caller's `_last_error_code`/`_message`
/// channel never learned why the call failed. The error must now reach `set_last_error`. ~keep
#[test]
fn entrypoint_result_status_reports_domain_error_and_compiles() {
    let return_body = render(
        "service_api_entrypoint_return_result_status.rs.jinja",
        minijinja::context! { call => "call()" },
    );
    let source = format!(
        r#"
#[derive(Debug)]
struct DomainError;
impl std::fmt::Display for DomainError {{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{
        write!(f, "domain error")
    }}
}}
fn call() -> Result<(), DomainError> {{
    Err(DomainError)
}}
static LAST_ERROR: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
fn set_last_error(_: i32, message: &str) {{
    *LAST_ERROR.lock().unwrap() = Some(message.to_owned());
}}
fn run() -> i32 {{
{return_body}
}}
fn main() {{
    assert_eq!(run(), 1);
    assert_eq!(LAST_ERROR.lock().unwrap().as_deref(), Some("domain error"));
}}
"#
    );
    let directory = tempfile::tempdir().expect("temporary directory");
    let source_path = directory.path().join("entrypoint_result_status.rs");
    let binary_path = directory.path().join("entrypoint-result-status-test");
    std::fs::write(&source_path, source).expect("write compile harness");
    let compile = std::process::Command::new("rustc")
        .current_dir(directory.path())
        .args(["--edition=2024", "-o"])
        .arg(&binary_path)
        .arg(&source_path)
        .output()
        .expect("run rustc");
    assert!(compile.status.success(), "{}", String::from_utf8_lossy(&compile.stderr));
    let run = std::process::Command::new(&binary_path)
        .current_dir(directory.path())
        .output()
        .expect("run compiled harness");
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
}

/// Companion regression, opaque-handle-returning shape: a fallible entrypoint that returns
/// an owned handle used to discard the domain error the same way (`Err(_) => 0`). ~keep
#[test]
fn entrypoint_opaque_result_reports_domain_error_and_compiles() {
    let return_body = render(
        "service_api_entrypoint_return_opaque_result.rs.jinja",
        minijinja::context! { call => "call()" },
    );
    let source = format!(
        r#"
#[derive(Debug)]
struct DomainError;
impl std::fmt::Display for DomainError {{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{
        write!(f, "domain error")
    }}
}}
#[derive(Debug)]
struct HandleError;
struct Application;
fn call() -> Result<Application, DomainError> {{
    Err(DomainError)
}}
fn insert_handle<T>(_value: T) -> Result<u64, HandleError> {{
    Ok(1)
}}
fn set_handle_error(_: &HandleError) {{}}
static LAST_ERROR: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
fn set_last_error(_: i32, message: &str) {{
    *LAST_ERROR.lock().unwrap() = Some(message.to_owned());
}}
fn run() -> u64 {{
{return_body}
}}
fn main() {{
    assert_eq!(run(), 0);
    assert_eq!(LAST_ERROR.lock().unwrap().as_deref(), Some("domain error"));
}}
"#
    );
    let directory = tempfile::tempdir().expect("temporary directory");
    let source_path = directory.path().join("entrypoint_opaque_result.rs");
    let binary_path = directory.path().join("entrypoint-opaque-result-test");
    std::fs::write(&source_path, source).expect("write compile harness");
    let compile = std::process::Command::new("rustc")
        .current_dir(directory.path())
        .args(["--edition=2024", "-o"])
        .arg(&binary_path)
        .arg(&source_path)
        .output()
        .expect("run rustc");
    assert!(compile.status.success(), "{}", String::from_utf8_lossy(&compile.stderr));
    let run = std::process::Command::new(&binary_path)
        .current_dir(directory.path())
        .output()
        .expect("run compiled harness");
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
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
    assert!(rs.contains("tokio::runtime::Builder::new_multi_thread()"));
    assert!(
        rs.contains(".thread_stack_size(ENTRYPOINT_RUNTIME_STACK_SIZE_BYTES)"),
        "the entrypoint runtime must widen the worker stack past tokio's ~2 MB default, or a \
         deep consumer future overflows it and aborts the process with SIGBUS:\n{rs}"
    );
    assert!(
        !rs.contains("tokio::runtime::Runtime::new()"),
        "the entrypoint must not build a runtime with tokio's default (undersized) stack:\n{rs}"
    );
    assert_eq!(
        rs.matches("#[unsafe(no_mangle)]").count(),
        rs.matches("catch_ffi_panic(").count(),
        "every Rust-owned service export must have a panic guard"
    );
    assert!(
        rs.contains("fn catch_ffi_panic<T>(fallback: T, body: impl FnOnce() -> T) -> T"),
        "service modules must define their panic guard instead of depending on a parent module helper"
    );
}

#[test]
fn test_service_header_declares_metadata_and_entrypoint_params() {
    let api = make_fixture_surface();
    let header = gen_service_h(&api, "test_crate");

    assert!(
        header.contains(
            "handler_callback_t callback,\n    handler_response_free_t response_free,\n    void* context,\n    const char* path\n);"
        ),
        "registration metadata param missing from service header:\n{header}"
    );
    assert!(
        header.contains("test_crate_test_service_ep_run(\n    uint64_t owner,\n    const char* addr\n);"),
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
        cfg: None,
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
            cfg: None,
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
    let preamble_tail = preamble.rsplit("#[unsafe(no_mangle)]").next().unwrap_or(preamble);
    assert!(
        preamble.contains("#[unsafe(no_mangle)]"),
        "#[unsafe(no_mangle)] must precede the variant fn"
    );
    assert!(
        preamble_tail.trim().starts_with("pub extern") || preamble_tail.trim().starts_with("pub unsafe extern"),
        "#[unsafe(no_mangle)] must directly precede the extern fn (intervening: `{preamble_tail}`)"
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
        body.contains("if owner == 0"),
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
        cfg: None,
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
            cfg: None,
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
        cfg: None,
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
        cfg: None,
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
        rs.contains("let inner = match service.inner.take()"),
        "configurator must `take()` owner.inner before calling the consuming method; got:\n{rs}"
    );
    assert!(
        rs.contains("service.inner = Some(Box::new(inner.setup("),
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
        rs.contains("with_handle::<my_crate::RouteBuilder, _>(builder, Clone::clone)"),
        "opaque-pointer metadata param `builder` must be borrowed via &*ptr; got:\n{rs}"
    );
    assert!(
        rs.contains(".add_route(builder, handler)"),
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
