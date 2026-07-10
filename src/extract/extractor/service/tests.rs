use super::*;
use crate::core::config::service::{EntrypointSpec, HandlerContractConfig, RegistrationSpec, ServiceConfig};
use crate::extract::extractor;

/// Write a temporary Rust source file and extract its surface.
fn extract_source(src: &str) -> ApiSurface {
    let dir = tempfile::tempdir().expect("tempdir");
    let file_path = dir.path().join("lib.rs");
    std::fs::write(&file_path, src).expect("write test source");
    extractor::extract(&[file_path.as_path()], "test_crate", "0.1.0", None).expect("extraction must succeed")
}

/// Like [`extract_source`] but keeps the temp dir alive and returns the source
/// path, so the service pass can re-parse it to recover generic registration
/// methods that the main extraction skipped.
fn extract_source_persistent(src: &str) -> (tempfile::TempDir, std::path::PathBuf, ApiSurface) {
    let dir = tempfile::tempdir().expect("tempdir");
    let file_path = dir.path().join("lib.rs");
    std::fs::write(&file_path, src).expect("write test source");
    let surface =
        extractor::extract(&[file_path.as_path()], "test_crate", "0.1.0", None).expect("extraction must succeed");
    (dir, file_path, surface)
}

/// after a web app; replace with neutral owner/contract examples.
/// Minimal Rust source with an owner type, a contract trait, and supporting
/// methods that exercise every classification bucket.
///
/// Note: the constructor is named `new` and returns `Self`, which the main
/// extraction pass intentionally skips (treated as a field-constructed
/// default).  The service pass must recover it from source so the
/// `ServiceDef` constructor is populated.
const SERVICE_SOURCE: &str = r#"
/// App documentation.
pub struct App {
    addr: String,
}

impl App {
    /// Create a new App.
    pub fn new() -> Self { unimplemented!() }

    /// Set bind address (configurator).
    pub fn set_address(mut self, addr: String) -> Self { unimplemented!() }

    /// Register a route (registration — generic param H: IntoHandler).
    pub fn add_route<H: IntoHandler>(mut self, path: String, handler: H) -> Self { unimplemented!() }

    /// Run the service (async entrypoint).
    pub async fn run(self) -> Result<(), String> { unimplemented!() }

    /// Consume into a router (finalize entrypoint).
    pub fn into_router(self) -> Router { unimplemented!() }
}

/// Handler contract trait.
pub trait Handler {
    async fn call(&self, req: RequestData) -> ResponseData;
}

/// Wire request DTO.
pub struct RequestData {
    pub path: String,
}

/// Wire response DTO.
pub struct ResponseData {
    pub status: u32,
}

/// Router type (returned by finalize).
pub struct Router {}

// IntoHandler is a bound used in generic registration — not an exported binding type.
pub trait IntoHandler {}
"#;

fn make_resolved_config_with_service() -> crate::core::config::ResolvedCrateConfig {
    crate::core::config::ResolvedCrateConfig {
        name: "test_crate".to_string(),
        services: vec![ServiceConfig {
            owner_type: "App".to_string(),
            constructor: Some("new".to_string()),
            configurators: vec!["set_address".to_string()],
            registrations: vec![RegistrationSpec {
                method: "add_route".to_string(),
                callback_param: "handler".to_string(),
                callback_bound: "IntoHandler".to_string(),
                callback_contract: "Handler".to_string(),
                variants: vec![],
            }],
            entrypoints: vec![
                EntrypointSpec {
                    method: "run".to_string(),
                    kind: "run".to_string(),
                },
                EntrypointSpec {
                    method: "into_router".to_string(),
                    kind: "finalize".to_string(),
                },
            ],
            skip_languages: vec![],
            host_app_inner_accessor: None,
        }],
        handler_contracts: vec![HandlerContractConfig {
            trait_name: "Handler".to_string(),
            dispatch_method: "call".to_string(),
            is_async: true,
            wire_request_type: Some("RequestData".to_string()),
            wire_response_type: Some("ResponseData".to_string()),
            optional_overrides: vec![],
            dispatch_extra_params: vec![],
            wire_param_name: None,
            dispatch_return_type: None,
            response_adapter: None,
        }],
        ..Default::default()
    }
}

#[test]
fn service_extraction_populates_service_def_and_handler_contract() {
    let (_dir, file_path, mut surface) = extract_source_persistent(SERVICE_SOURCE);
    let mut config = make_resolved_config_with_service();
    config.sources = vec![file_path];

    let warnings = extract_services(&mut surface, &config);
    assert!(warnings.is_empty(), "no warnings expected; got {warnings:?}");

    assert_eq!(
        surface.handler_contracts.len(),
        1,
        "exactly one HandlerContractDef expected"
    );
    let hc = &surface.handler_contracts[0];
    assert_eq!(hc.trait_name, "Handler");
    assert_eq!(hc.dispatch.name, "call");
    assert!(hc.dispatch.is_async, "dispatch method must be detected as async");
    assert_eq!(hc.wire_request_type.as_deref(), Some("RequestData"));
    assert_eq!(hc.wire_response_type.as_deref(), Some("ResponseData"));

    let handler_type = surface.types.iter().find(|t| t.name == "Handler");
    if let Some(t) = handler_type {
        assert!(t.binding_excluded, "Handler trait must be marked binding_excluded");
    }

    assert_eq!(surface.services.len(), 1, "exactly one ServiceDef expected");
    let svc = &surface.services[0];
    assert_eq!(svc.name, "App");
    assert_eq!(
        svc.constructor.name, "new",
        "constructor `new` must be recovered from source"
    );
    assert_eq!(svc.configurators.len(), 1);
    assert_eq!(svc.configurators[0].name, "set_address");

    assert_eq!(svc.registrations.len(), 1, "add_route registration must be recovered");
    let reg = &svc.registrations[0];
    assert_eq!(reg.method, "add_route");
    assert_eq!(reg.callback_param, "handler");
    assert_eq!(reg.callback_contract, "Handler");
    assert!(
        reg.metadata_params.iter().all(|p| p.name != "handler"),
        "callback param must be excluded from metadata_params"
    );
    assert!(
        reg.metadata_params.iter().any(|p| p.name == "path"),
        "metadata param `path` expected"
    );

    assert_eq!(svc.entrypoints.len(), 2, "expected run + finalize entrypoints");
    let run_ep = svc
        .entrypoints
        .iter()
        .find(|e| e.method == "run")
        .expect("run entrypoint");
    assert_eq!(run_ep.kind, EntrypointKind::Run);
    assert!(run_ep.is_async, "run must be async");
    let fin_ep = svc
        .entrypoints
        .iter()
        .find(|e| e.method == "into_router")
        .expect("into_router entrypoint");
    assert_eq!(fin_ep.kind, EntrypointKind::Finalize);

    let app_type = surface.types.iter().find(|t| t.name == "App");
    if let Some(t) = app_type {
        assert!(t.binding_excluded, "App must be marked binding_excluded");
    }
}

#[test]
fn missing_owner_type_returns_warning_not_panic() {
    let mut surface = extract_source("pub struct Unrelated {}");
    let config = crate::core::config::ResolvedCrateConfig {
        name: "test_crate".to_string(),
        services: vec![ServiceConfig {
            owner_type: "NonExistent".to_string(),
            constructor: None,
            configurators: vec![],
            registrations: vec![],
            entrypoints: vec![],
            skip_languages: vec![],
            host_app_inner_accessor: None,
        }],
        ..Default::default()
    };
    let warnings = extract_services(&mut surface, &config);
    assert!(
        !warnings.is_empty(),
        "missing owner type must produce a warning, got none"
    );
    assert!(surface.services.is_empty(), "no ServiceDef must be pushed on failure");
}

#[test]
fn missing_configured_configurator_returns_error() {
    let (_dir, file_path, mut surface) = extract_source_persistent(SERVICE_SOURCE);
    let mut config = make_resolved_config_with_service();
    config.sources = vec![file_path];
    config.services[0].configurators = vec!["missing_configurator".to_string()];

    let errors = extract_services(&mut surface, &config);

    assert!(
        errors
            .iter()
            .any(|error| error.contains("configurator method `missing_configurator` not found")),
        "configured missing configurator must be fatal, got {errors:?}"
    );
    assert!(surface.services.is_empty(), "invalid service must not be emitted");
}

#[test]
fn unrecovered_configured_method_returns_error() {
    let (_dir, file_path, mut surface) = extract_source_persistent(SERVICE_SOURCE);
    let mut config = make_resolved_config_with_service();
    config.sources = vec![file_path];
    config.services[0].registrations[0].method = "missing_registration".to_string();

    let errors = extract_services(&mut surface, &config);

    assert!(
        errors.iter().any(|error| {
            error.contains("service `App`: configured method `missing_registration` could not be recovered")
        }),
        "configured missing service method must fail recovery, got {errors:?}"
    );
    assert!(surface.services.is_empty(), "invalid service must not be emitted");
}

#[test]
fn registration_variants_resolve_via_wrapper_constructor() {
    use crate::core::config::service::RegistrationVariantSpec;
    let src = r#"
pub struct App {}
impl App {
    pub fn new() -> Self { unimplemented!() }
    pub fn route<H: IntoHandler>(mut self, builder: RouteBuilder, handler: H) -> Self { unimplemented!() }
    pub async fn run(self) -> Result<(), String> { unimplemented!() }
}

pub struct RouteBuilder {}
impl RouteBuilder {
    pub fn new(method: Method, path: String) -> Self { unimplemented!() }
}

pub enum Method { Get, Post, Put }

pub trait Handler {
    async fn call(&self, req: RequestData) -> ResponseData;
}
pub struct RequestData {}
pub struct ResponseData {}
pub trait IntoHandler {}
"#;
    let (_dir, file_path, mut surface) = extract_source_persistent(src);
    let mut cfg = make_resolved_config_with_service();
    cfg.sources = vec![file_path];
    cfg.services[0].configurators.clear();
    cfg.services[0].registrations[0].method = "route".to_owned();
    cfg.services[0].registrations[0].variants = vec![
        RegistrationVariantSpec {
            name: "get".to_owned(),
            fixed: [("method".to_owned(), "Get".to_owned())].into_iter().collect(),
            doc: None,
            style: None,
        },
        RegistrationVariantSpec {
            name: "post".to_owned(),
            fixed: [("method".to_owned(), "Post".to_owned())].into_iter().collect(),
            doc: None,
            style: None,
        },
    ];
    cfg.services[0].entrypoints.retain(|e| e.method != "into_router");

    let warnings = extract_services(&mut surface, &cfg);
    assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");

    let svc = &surface.services[0];
    let reg = &svc.registrations[0];
    assert_eq!(reg.variants.len(), 2);

    let get = reg.variants.iter().find(|v| v.name == "get").expect("get variant");
    let wrapper = get.wrapper_call.as_ref().expect("wrapper_call resolved");
    assert_eq!(wrapper.metadata_param, "builder");
    assert_eq!(wrapper.wrapper_type_name, "RouteBuilder");
    assert_eq!(wrapper.constructor_method, "new");
    assert_eq!(wrapper.args.len(), 2);
    let method_arg = &wrapper.args[0];
    match method_arg {
        crate::core::ir::WrapperConstructorArg::Fixed { param_name, value_expr } => {
            assert_eq!(param_name, "method");
            assert!(
                value_expr.ends_with("Method::Get"),
                "expected resolved enum path ending in Method::Get, got `{value_expr}`"
            );
        }
        other => panic!("expected Fixed for method, got {other:?}"),
    }
    let path_arg = &wrapper.args[1];
    match path_arg {
        crate::core::ir::WrapperConstructorArg::Free { param } => {
            assert_eq!(param.name, "path");
        }
        other => panic!("expected Free for path, got {other:?}"),
    }
    assert_eq!(get.signature_params.len(), 1);
    assert_eq!(get.signature_params[0].name, "path");

    let route_builder = surface
        .types
        .iter()
        .find(|t| t.name == "RouteBuilder")
        .expect("RouteBuilder TypeDef must exist");
    assert!(
        route_builder.is_variant_wrapper,
        "RouteBuilder must be marked is_variant_wrapper after extract_services"
    );
    let request_data = surface.types.iter().find(|t| t.name == "RequestData");
    if let Some(rd) = request_data {
        assert!(
            !rd.is_variant_wrapper,
            "non-wrapper types must NOT be flagged is_variant_wrapper"
        );
    }
}

#[test]
fn registration_variant_unknown_enum_variant_returns_error() {
    use crate::core::config::service::RegistrationVariantSpec;
    let src = r#"
pub struct App {}
impl App {
    pub fn new() -> Self { unimplemented!() }
    pub fn route<H: IntoHandler>(mut self, builder: RouteBuilder, handler: H) -> Self { unimplemented!() }
    pub async fn run(self) -> Result<(), String> { unimplemented!() }
}
pub struct RouteBuilder {}
impl RouteBuilder {
    pub fn new(method: Method, path: String) -> Self { unimplemented!() }
}
pub enum Method { Get, Post }
pub trait Handler { async fn call(&self, r: R) -> S; }
pub struct R {}
pub struct S {}
pub trait IntoHandler {}
"#;
    let (_dir, file_path, mut surface) = extract_source_persistent(src);
    let mut cfg = make_resolved_config_with_service();
    cfg.sources = vec![file_path];
    cfg.services[0].configurators.clear();
    cfg.services[0].registrations[0].method = "route".to_owned();
    cfg.services[0].registrations[0].variants = vec![RegistrationVariantSpec {
        name: "bogus".to_owned(),
        fixed: [("method".to_owned(), "NotARealVariant".to_owned())]
            .into_iter()
            .collect(),
        doc: None,
        style: None,
    }];
    cfg.services[0].entrypoints.retain(|e| e.method != "into_router");
    let warnings = extract_services(&mut surface, &cfg);
    assert!(
        warnings.iter().any(|w| w.contains("no variant `NotARealVariant`")),
        "expected unknown-variant warning, got {warnings:?}"
    );
}

#[test]
fn registration_variant_unknown_style_returns_error() {
    use crate::core::config::service::RegistrationVariantSpec;
    let src = r#"
pub struct App {}
impl App {
    pub fn new() -> Self { unimplemented!() }
    pub fn route<H: IntoHandler>(mut self, builder: RouteBuilder, handler: H) -> Self { unimplemented!() }
    pub async fn run(self) -> Result<(), String> { unimplemented!() }
}
pub struct RouteBuilder {}
impl RouteBuilder {
    pub fn new(method: Method, path: String) -> Self { unimplemented!() }
}
pub enum Method { Get, Post }
pub trait Handler { async fn call(&self, r: R) -> S; }
pub struct R {}
pub struct S {}
pub trait IntoHandler {}
"#;
    let (_dir, file_path, mut surface) = extract_source_persistent(src);
    let mut cfg = make_resolved_config_with_service();
    cfg.sources = vec![file_path];
    cfg.services[0].configurators.clear();
    cfg.services[0].registrations[0].method = "route".to_owned();
    cfg.services[0].registrations[0].variants = vec![RegistrationVariantSpec {
        name: "bad_style".to_owned(),
        fixed: [("method".to_owned(), "Get".to_owned())].into_iter().collect(),
        doc: None,
        style: Some("future_magic".to_owned()),
    }];
    cfg.services[0].entrypoints.retain(|e| e.method != "into_router");

    let errors = extract_services(&mut surface, &cfg);

    assert!(
        errors
            .iter()
            .any(|error| error.contains("unknown registration variant style `future_magic`")),
        "expected unknown-style error, got {errors:?}"
    );
}

#[test]
fn empty_services_config_is_a_no_op() {
    let mut surface = extract_source("pub struct Foo {}");
    let config = crate::core::config::ResolvedCrateConfig {
        name: "test_crate".to_string(),
        ..Default::default()
    };
    let warnings = extract_services(&mut surface, &config);
    assert!(warnings.is_empty());
    assert!(surface.services.is_empty());
    assert!(surface.handler_contracts.is_empty());
}

/// A configurator method whose name collides with a private field of the owner
/// struct must still be extracted into `service.configurators`. The extractor's
/// method extraction filter only skips `new`-returning-Self on field-based types;
/// it has no rule that drops methods whose name matches a private field. This
/// test documents that invariant so a future extractor change does not silently
/// break it.
///
/// Pattern: `struct Foo { setup: BarConfig }` with `pub fn setup(self, c: BarConfig) -> Self`.
#[test]
fn configurator_with_same_name_as_private_field_is_extracted() {
    let src = r#"
pub struct Foo {
    setup: BarConfig,
}

impl Foo {
    pub fn new() -> Self { unimplemented!() }
    pub fn setup(mut self, c: BarConfig) -> Self { unimplemented!() }
    pub async fn run(self) -> Result<(), String> { unimplemented!() }
}

pub struct BarConfig {
    pub value: u32,
}
"#;
    let (_dir, file_path, mut surface) = extract_source_persistent(src);
    let config = crate::core::config::ResolvedCrateConfig {
        name: "test_crate".to_string(),
        services: vec![ServiceConfig {
            owner_type: "Foo".to_string(),
            constructor: Some("new".to_string()),
            configurators: vec!["setup".to_string()],
            registrations: vec![],
            entrypoints: vec![EntrypointSpec {
                method: "run".to_string(),
                kind: "run".to_string(),
            }],
            skip_languages: vec![],
            host_app_inner_accessor: None,
        }],
        sources: vec![file_path],
        ..Default::default()
    };

    let warnings = extract_services(&mut surface, &config);
    assert!(
        warnings.is_empty(),
        "no warnings expected for field/method name collision; got {warnings:?}"
    );

    assert_eq!(surface.services.len(), 1, "one ServiceDef must be emitted");
    let svc = &surface.services[0];
    assert_eq!(svc.name, "Foo");
    assert_eq!(
        svc.configurators.len(),
        1,
        "configurator `setup` must be in service.configurators even though \
             a private field named `setup` exists on the owner type; got {:?}",
        svc.configurators.iter().map(|m| m.name.as_str()).collect::<Vec<_>>()
    );
    assert_eq!(svc.configurators[0].name, "setup", "configurator name must be `setup`");
}
