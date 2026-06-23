//! Service-API codegen for the PyO3 backend.
//!
//! Generates two outputs per [`ServiceDef`]:
//!
//! 1. **`service.rs`** — Rust pyo3 glue that wraps each registered Python
//!    callable as `Arc<dyn <HandlerContractDef::trait_name>>` via an async
//!    callback bridge, builds the core service via the owner type's
//!    registration and run entrypoints, and exposes a `#[pyfunction]` entry
//!    point.
//!
//! 2. **`service.py`** — An idiomatic Python class mirroring the service's
//!    constructor, configurator methods, and registration decorators, with a
//!    `run(...)` method that delegates to the native extension.
//!
//! All names are derived entirely from the [`ApiSurface`] IR — no transport-
//! or domain-specific assumptions are made anywhere in this module.

mod helpers;
mod python_service;
mod registration_variants;
mod rust_service;

use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::ApiSurface;
use std::path::PathBuf;

pub(super) fn gen_service_py(api: &ApiSurface, module_name: &str) -> String {
    python_service::gen_service_py(api, module_name)
}

pub(super) fn gen_service_rs(api: &ApiSurface, config: &ResolvedCrateConfig) -> String {
    rust_service::gen_service_rs(api, config)
}

pub fn generate(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    if api.services.is_empty() {
        return Ok(vec![]);
    }

    use crate::core::config::resolve_output_dir;

    let output_dir = resolve_output_dir(config.output_paths.get("python"), &config.name, "crates/{name}-py/src/");
    let module_name = config.python_module_name();

    // Rust glue
    let service_rs = gen_service_rs(api, config);

    // Python wrapper
    let service_py = gen_service_py(api, &module_name);

    // Python package output base (same logic as generate_public_api)
    let output_base = config
        .python
        .as_ref()
        .and_then(|p| p.stubs.as_ref())
        .map(|s| PathBuf::from(&s.output))
        .unwrap_or_else(|| {
            let package_name = config.name.replace('-', "_");
            PathBuf::from(format!("packages/python/{}", package_name))
        });

    Ok(vec![
        GeneratedFile {
            path: PathBuf::from(&output_dir).join("service.rs"),
            content: service_rs,
            generated_header: true,
        },
        GeneratedFile {
            path: output_base.join("service.py"),
            content: service_py,
            generated_header: true,
        },
    ])
}

// ───────────────────────────────────────────────────────────────────── tests ──

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{
        EntrypointDef, EntrypointKind, HandlerContractDef, MethodDef, ParamDef, PrimitiveType, RegistrationDef,
        ServiceDef, TypeRef,
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

    /// `gen_service_py` emits a class named after the service owner.
    #[test]
    fn python_output_contains_service_class() {
        let surface = make_fixture_surface();
        let output = gen_service_py(&surface, "_my_crate");
        assert!(
            output.contains("class TestService:"),
            "expected `class TestService:` in output:\n{output}"
        );
    }

    /// `gen_service_py` emits `__init__` with registration state initialisation.
    #[test]
    fn python_output_contains_init_with_registrations() {
        let surface = make_fixture_surface();
        let output = gen_service_py(&surface, "_my_crate");
        assert!(
            output.contains("def __init__(self)"),
            "expected `def __init__(self)` in output:\n{output}"
        );
        assert!(
            output.contains("self._registrations"),
            "expected `self._registrations` in output:\n{output}"
        );
    }

    /// `gen_service_py` emits configurator methods that return `self`.
    #[test]
    fn python_output_contains_configurator() {
        let surface = make_fixture_surface();
        let output = gen_service_py(&surface, "_my_crate");
        assert!(
            output.contains("def with_timeout(self, timeout_ms: int)"),
            "expected `with_timeout` configurator:\n{output}"
        );
        assert!(
            output.contains("return self"),
            "expected `return self` in configurator:\n{output}"
        );
    }

    /// `gen_service_py` emits a decorator for the registration method.
    #[test]
    fn python_output_contains_registration_decorator() {
        let surface = make_fixture_surface();
        let output = gen_service_py(&surface, "_my_crate");
        assert!(
            output.contains("def add_handler("),
            "expected `add_handler` registration method:\n{output}"
        );
        assert!(
            output.contains("def _decorator(fn"),
            "expected inner `_decorator` closure:\n{output}"
        );
        assert!(
            output.contains("self._registrations.append"),
            "expected `_registrations.append` in decorator:\n{output}"
        );
    }

    /// `gen_service_py` emits the `run` entrypoint.
    #[test]
    fn python_output_contains_run_entrypoint() {
        let surface = make_fixture_surface();
        let output = gen_service_py(&surface, "_my_crate");
        assert!(
            output.contains("def run(self"),
            "expected `def run(self` entrypoint:\n{output}"
        );
        assert!(
            output.contains("_my_crate.test_service_run("),
            "expected native call `_my_crate.test_service_run(` in run:\n{output}"
        );
    }

    /// `gen_service_py` emits registration variants with both method and decorator forms.
    #[test]
    fn python_output_contains_registration_variants() {
        let mut surface = make_fixture_surface();
        // Add a variant to the registration
        let variant = crate::core::ir::RegistrationVariant {
            name: "get".to_owned(),
            overrides: vec![],
            wrapper_call: Some(crate::core::ir::WrapperConstructorCall {
                metadata_param: "builder".to_owned(),
                wrapper_type_path: "mylib::RouteBuilder".to_owned(),
                wrapper_type_name: "RouteBuilder".to_owned(),
                constructor_method: "new".to_owned(),
                args: vec![
                    crate::core::ir::WrapperConstructorArg::Fixed {
                        param_name: "method".to_owned(),
                        value_expr: "mylib::Method::GET".to_owned(),
                    },
                    crate::core::ir::WrapperConstructorArg::Free {
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
            style: crate::core::ir::RegistrationVariantStyle::Hybrid,
            ..Default::default()
        };
        surface.services[0].registrations[0].variants.push(variant);

        let output = gen_service_py(&surface, "_my_crate");
        // Check for variant method form
        assert!(
            output.contains("def get(self, path: str, handler: Callable[..., Any])"),
            "expected `def get(self, path: str, handler)` method form:\n{output}"
        );
        // Check for variant decorator form
        assert!(
            output.contains("def get_decorator(self, path: str)"),
            "expected `def get_decorator(self, path: str)` decorator form:\n{output}"
        );
        // Check for wrapper constructor call (pyo3 opaque wrappers expose `.new()` classmethod)
        assert!(
            output.contains("builder = RouteBuilder.new(Method.GET, path)"),
            "expected wrapper constructor call with Method.GET:\n{output}"
        );
        // Wrapper-consumed params (path, method) must NOT appear in the metadata tuple
        assert!(
            output.contains("(\"add_handler\", (builder,), handler)"),
            "expected metadata tuple to contain only the constructed wrapper:\n{output}"
        );
    }

    /// `gen_service_rs` emits the handler bridge struct.
    #[test]
    fn rust_output_contains_handler_bridge_struct() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let output = gen_service_rs(&surface, &config);
        assert!(
            output.contains("pub struct PyRequestHandlerBridge"),
            "expected `PyRequestHandlerBridge` struct:\n{output}"
        );
    }

    /// `gen_service_rs` emits the handler bridge trait impl.
    #[test]
    fn rust_output_contains_handler_bridge_impl() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let output = gen_service_rs(&surface, &config);
        assert!(
            output.contains("impl my_crate::RequestHandler for PyRequestHandlerBridge"),
            "expected trait impl:\n{output}"
        );
        assert!(
            output.contains("fn handle(") && output.contains("Pin<Box<dyn Future<Output"),
            "expected dispatch method returning a boxed future:\n{output}"
        );
    }

    /// `gen_service_rs` emits the `#[pyfunction]` run entry point.
    #[test]
    fn rust_output_contains_pyfunction_run() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let output = gen_service_rs(&surface, &config);
        assert!(
            output.contains("#[pyfunction]"),
            "expected `#[pyfunction]` attribute:\n{output}"
        );
        assert!(
            output.contains("pub fn test_service_run("),
            "expected `test_service_run` function:\n{output}"
        );
    }

    /// Sync entrypoints must release the GIL around the blocking core call so a
    /// trait callback re-entering Python from a worker thread cannot deadlock on
    /// the GIL the entrypoint thread holds. The `into_router` fixture entrypoint
    /// is sync; its core call must be wrapped in `_py.detach(|| ...)`.
    #[test]
    fn rust_sync_entrypoint_releases_gil_around_core_call() {
        let surface = make_fixture_surface();
        let config = make_test_config();
        let output = gen_service_rs(&surface, &config);
        assert!(
            output.contains("_py.detach(|| owner.into_router())"),
            "expected sync entrypoint core call wrapped in `_py.detach(|| ...)`:\n{output}"
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
        assert!(paths.contains(&"service.py"), "expected service.py in output");
    }

    /// Full `generate()` returns empty for a surface with no services.
    #[test]
    fn generate_returns_empty_for_no_services() {
        let surface = ApiSurface::default();
        let config = make_test_config();
        let files = generate(&surface, &config).expect("generate should not fail");
        assert!(files.is_empty(), "expected no files for surface without services");
    }

    // ── helpers ──────────────────────────────────────────────────────────────

    fn make_test_config() -> ResolvedCrateConfig {
        use crate::core::config::resolved::ResolvedCrateConfig;
        ResolvedCrateConfig {
            name: "my-crate".to_owned(),
            ..ResolvedCrateConfig::default()
        }
    }
}
