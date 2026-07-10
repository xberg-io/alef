//! Workspace-level guard against stubbed service-API codegen.
//!
//! The per-backend golden tests only assert that generated strings *contain* expected
//! substrings — they cannot catch a backend that emits a placeholder instead of real
//! handler dispatch (e.g. a hardcoded return, an un-interpolated `{placeholder}`, or a
//! "would be called here" comment). This test drives every service-API-capable backend's
//! `generate_service_api` over one realistic service surface and asserts that no output
//! contains a forbidden stub marker, so the failure class that shipped four broken
//! backends cannot silently recur for a newly added backend.

use alef::backends::csharp::CsharpBackend;
use alef::backends::dart::DartBackend;
use alef::backends::extendr::ExtendrBackend;
use alef::backends::ffi::FfiBackend;
use alef::backends::go::GoBackend;
use alef::backends::java::JavaBackend;
use alef::backends::kotlin::KotlinBackend;
use alef::backends::magnus::MagnusBackend;
use alef::backends::napi::NapiBackend;
use alef::backends::php::PhpBackend;
use alef::backends::pyo3::Pyo3Backend;
use alef::backends::rustler::RustlerBackend;
use alef::backends::swift::SwiftBackend;
use alef::backends::zig::ZigBackend;
use alef::core::backend::Backend;
use alef::core::config::ResolvedCrateConfig;
use alef::core::ir::{
    ApiSurface, EntrypointDef, EntrypointKind, HandlerContractDef, MethodDef, ParamDef, ReceiverKind, RegistrationDef,
    ServiceDef, TypeRef,
};

/// Markers that must never appear in generated service-API output: each one indicates a
/// stub, an unimplemented path, or an un-interpolated codegen template placeholder.
const FORBIDDEN_MARKERS: &[&str] = &[
    "placeholder",
    concat!("FIX", "ME"),
    "unimplemented!",
    concat!("to", "do!()"),
    "would be called here",
    "would happen here",
    "simplified stub",
    "For now, return",
    // Un-interpolated codegen placeholders (a `push_str` that should have been `format!`).
    "{service_snake}",
    "{service_pascal}",
    "{class_name}",
    "{method_pascal}",
];

/// A service surface with one registration (carrying a single metadata param, the case
/// most likely to expose tuple-arity bugs) plus both a `Run` and a `Finalize` entrypoint.
fn service_surface() -> ApiSurface {
    let unit_method = |name: &str, is_static: bool| MethodDef {
        name: name.to_owned(),
        params: vec![],
        return_type: TypeRef::Unit,
        is_async: false,
        is_static,
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
        receiver: Some(ReceiverKind::RefMut),
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

    let finalize_entrypoint = EntrypointDef {
        method: "finalize".to_owned(),
        kind: EntrypointKind::Finalize,
        is_async: false,
        params: vec![],
        return_type: TypeRef::Named("Router".to_owned()),
        error_type: None,
        doc: "Finalize into a router.".to_owned(),
    };

    let dispatch = MethodDef {
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
        receiver: Some(ReceiverKind::Ref),
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

    ApiSurface {
        crate_name: "test_crate".to_owned(),
        version: "1.0.0".to_owned(),
        services: vec![ServiceDef {
            name: "TestService".to_owned(),
            rust_path: "my_crate::TestService".to_owned(),
            constructor: unit_method("new", true),
            configurators: vec![],
            registrations: vec![registration],
            entrypoints: vec![run_entrypoint, finalize_entrypoint],
            doc: "Test service.".to_owned(),
            cfg: None,
        }],
        handler_contracts: vec![HandlerContractDef {
            trait_name: "RequestHandler".to_owned(),
            rust_path: "my_crate::RequestHandler".to_owned(),
            dispatch,
            optional_methods: vec![],
            wire_request_type: Some("RequestData".to_owned()),
            wire_response_type: Some("Response".to_owned()),
            dispatch_extra_params: vec![],
            wire_param_name: None,
            dispatch_return_type: None,
            response_adapter: None,
            doc: "Handler contract.".to_owned(),
        }],
        ..ApiSurface::default()
    }
}

/// Every service-API-capable language backend. (jni is the JVM *native* contract validated
/// by its own tests; wasm/gleam/kotlin-android do not implement service generation.)
fn service_backends() -> Vec<(&'static str, Box<dyn Backend>)> {
    vec![
        ("pyo3", Box::new(Pyo3Backend)),
        ("napi", Box::new(NapiBackend)),
        ("magnus", Box::new(MagnusBackend)),
        ("php", Box::new(PhpBackend)),
        ("ffi", Box::new(FfiBackend)),
        ("go", Box::new(GoBackend)),
        ("csharp", Box::new(CsharpBackend)),
        ("dart", Box::new(DartBackend)),
        ("zig", Box::new(ZigBackend)),
        ("rustler", Box::new(RustlerBackend)),
        ("swift", Box::new(SwiftBackend)),
        ("java", Box::new(JavaBackend)),
        ("kotlin", Box::new(KotlinBackend)),
        ("extendr", Box::new(ExtendrBackend)),
    ]
}

#[test]
fn service_api_output_has_no_stub_markers() {
    let api = service_surface();
    let config = ResolvedCrateConfig {
        name: "test_crate".to_owned(),
        ..ResolvedCrateConfig::default()
    };

    for (name, backend) in service_backends() {
        if !backend.capabilities().supports_service_api {
            continue;
        }
        let files = backend
            .generate_service_api(&api, &config)
            .unwrap_or_else(|e| panic!("{name}: generate_service_api failed: {e}"));
        assert!(
            !files.is_empty(),
            "{name}: supports_service_api but generated no service files"
        );
        for file in &files {
            let path = file.path.to_string_lossy();
            for marker in FORBIDDEN_MARKERS {
                assert!(
                    !file.content.contains(marker),
                    "{name}: generated `{path}` contains forbidden stub marker `{marker}`:\n{}",
                    file.content
                );
            }
        }
    }
}
