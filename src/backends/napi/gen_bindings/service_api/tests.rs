use super::*;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{
    ApiSurface, EntrypointDef, EntrypointKind, HandlerContractDef, MethodDef, ParamDef, PrimitiveType, ReceiverKind,
    RegistrationDef, ServiceDef, TypeRef,
};

/// Construct a minimal test config with default exclude settings.
fn make_test_config() -> ResolvedCrateConfig {
    ResolvedCrateConfig {
        name: "my_crate".to_owned(),
        ..ResolvedCrateConfig::default()
    }
}

/// Construct a minimal but realistic [`ApiSurface`] that exercises:
/// - A service with a constructor, one configurator, one registration
///   (bound to an async handler contract), and Run entrypoint.
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
        receiver: Some(ReceiverKind::RefMut),
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
        receiver: Some(ReceiverKind::RefMut),
        return_type: TypeRef::Unit,
        error_type: None,
        doc: "Register a request handler for a path and method.".to_owned(),
        variants: vec![],
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

    let service = ServiceDef {
        name: "TestService".to_owned(),
        rust_path: "my_crate::TestService".to_owned(),
        constructor,
        configurators: vec![configurator],
        registrations: vec![registration],
        entrypoints: vec![run_ep],
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

#[test]
fn typescript_output_contains_service_class() {
    let surface = make_fixture_surface();
    let config = make_test_config();
    let output = gen_service_ts(&surface, "my_crate", &config);
    assert!(
        output.contains("export class TestService"),
        "expected `export class TestService` in output:\n{output}"
    );
}

#[test]
fn typescript_output_contains_constructor() {
    let surface = make_fixture_surface();
    let config = make_test_config();
    let output = gen_service_ts(&surface, "my_crate", &config);
    assert!(
        output.contains("constructor()"),
        "expected `constructor()` in output:\n{output}"
    );
}

#[test]
fn typescript_output_contains_private_registrations() {
    let surface = make_fixture_surface();
    let config = make_test_config();
    let output = gen_service_ts(&surface, "my_crate", &config);
    // After the fix, the TypeScript wrapper no longer accumulates registrations.
    // Instead, it stores a reference to the Rust wrapper instance (_app)
    // and delegates variant methods to it directly.
    assert!(
        output.contains("private _app:"),
        "expected `private _app:` (Rust wrapper instance) in output:\n{output}"
    );
}

#[test]
fn typescript_output_contains_configurator() {
    let surface = make_fixture_surface();
    let config = make_test_config();
    let output = gen_service_ts(&surface, "my_crate", &config);
    assert!(
        output.contains("with_timeout(_timeout_ms: number)"),
        "expected `with_timeout` configurator (param prefixed with _ because configurators are no-op chain methods):\n{output}"
    );
    assert!(
        output.contains("return this;"),
        "expected `return this;` in configurator:\n{output}"
    );
}

#[test]
fn typescript_output_contains_registration_method() {
    let surface = make_fixture_surface();
    let config = make_test_config();
    let output = gen_service_ts(&surface, "my_crate", &config);
    assert!(
        output.contains("add_handler(path: string, method: string)"),
        "expected `add_handler` registration method:\n{output}"
    );
}

#[test]
fn typescript_output_direct_register_method_uses_lower_camel_case() {
    // The direct-register variant (non-decorator) emits as a class method on the
    // wrapper App. JS classes use lowerCamelCase, so `register_<method>` must be
    // converted: `register_add_handler` → `registerAddHandler`. Without conversion
    // consumers hit `TypeError: app.registerAddHandler is not a function` at
    // runtime because the wrapper exposes the snake_case identifier instead.
    let surface = make_fixture_surface();
    let config = make_test_config();
    let output = gen_service_ts(&surface, "my_crate", &config);
    assert!(
        output.contains("registerAddHandler("),
        "expected lowerCamelCase `registerAddHandler` direct method, found snake_case or missing:\n{output}"
    );
    assert!(
        !output.contains("register_add_handler("),
        "snake_case `register_add_handler` must not survive into the emitted class:\n{output}"
    );
}

#[test]
fn typescript_output_contains_run_entrypoint() {
    let surface = make_fixture_surface();
    let config = make_test_config();
    let output = gen_service_ts(&surface, "my_crate", &config);
    assert!(
        output.contains("async run(addr: string)"),
        "expected `async run` entrypoint:\n{output}"
    );
}

#[test]
fn rust_output_contains_handler_bridge() {
    let surface = make_fixture_surface();
    let config = ResolvedCrateConfig {
        name: "my_crate".to_owned(),
        ..ResolvedCrateConfig::default()
    };
    let output = gen_service_rs(&surface, &config);
    assert!(
        output.contains("pub struct RequestHandlerBridge"),
        "expected `RequestHandlerBridge` struct in output:\n{output}"
    );
}

#[test]
fn rust_output_contains_run_function() {
    let surface = make_fixture_surface();
    let config = ResolvedCrateConfig {
        name: "my_crate".to_owned(),
        ..ResolvedCrateConfig::default()
    };
    let output = gen_service_rs(&surface, &config);
    assert!(
        output.contains("pub async fn test_service_run"),
        "expected `test_service_run` function in output:\n{output}"
    );
}

#[test]
fn rust_output_contains_thread_safe_function() {
    let surface = make_fixture_surface();
    let config = ResolvedCrateConfig {
        name: "my_crate".to_owned(),
        ..ResolvedCrateConfig::default()
    };
    let output = gen_service_rs(&surface, &config);
    assert!(
        output.contains("ThreadsafeFunction"),
        "expected `ThreadsafeFunction` in output:\n{output}"
    );
}

#[test]
fn rust_output_implements_trait() {
    let surface = make_fixture_surface();
    let config = ResolvedCrateConfig {
        name: "my_crate".to_owned(),
        ..ResolvedCrateConfig::default()
    };
    let output = gen_service_rs(&surface, &config);
    assert!(
        output.contains("impl my_crate::RequestHandler for RequestHandlerBridge"),
        "expected trait impl in output:\n{output}"
    );
}

#[test]
fn rust_output_extracts_metadata_params() {
    let surface = make_fixture_surface();
    let config = ResolvedCrateConfig {
        name: "my_crate".to_owned(),
        ..ResolvedCrateConfig::default()
    };
    let output = gen_service_rs(&surface, &config);

    // After the fix, metadata extraction moved from app_run to variant methods.
    // Variant methods on JsApp receive metadata parameters directly as function args
    // (e.g. path, method) instead of extracting from an array.
    // The app_run function is now simplified and doesn't handle metadata extraction.

    // Assert that the variant methods are defined (they handle metadata registration)
    assert!(
        output.contains("#[napi]"),
        "expected #[napi] attribute in impl block for variant methods:\n{output}"
    );

    // The handler bridge should still be present
    assert!(
        output.contains("HandlerBridge"),
        "expected HandlerBridge in output:\n{output}"
    );
}

#[test]
fn registration_variants_emit_napi_methods() {
    use crate::core::ir::{RegistrationVariant, WrapperConstructorArg, WrapperConstructorCall};

    let mut surface = make_fixture_surface();

    // Add a variant to the registration
    if let Some(reg) = surface.services[0].registrations.first_mut() {
        reg.variants.push(RegistrationVariant {
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
        });
    }

    let config = ResolvedCrateConfig {
        name: "my_crate".to_owned(),
        ..ResolvedCrateConfig::default()
    };
    let output = gen_service_rs(&surface, &config);

    // Assert the variant methods are wrapped in an impl block (default prefix "Js")
    assert!(
        output.contains("impl JsTestService {"),
        "expected `impl JsTestService {{` wrapping in output:\n{output}"
    );

    // Assert the use statement is emitted before the impl block
    assert!(
        output.contains("use crate::JsTestService;"),
        "expected `use crate::JsTestService;` in output:\n{output}"
    );

    // Assert the variant method is emitted with #[napi] (indented inside impl block)
    assert!(
        output.contains("#[napi]\n    pub fn get("),
        "expected `#[napi]\\n    pub fn get(` inside impl block in output:\n{output}"
    );

    // Assert the wrapper builder is constructed
    assert!(
        output.contains("my_crate::RouteBuilder::new("),
        "expected wrapper constructor call in output:\n{output}"
    );

    // Assert the fixed arg is substituted
    assert!(
        output.contains("my_crate::Method::GET"),
        "expected fixed arg substitution in output:\n{output}"
    );
}

#[test]
fn typescript_output_emits_entrypoint_even_when_method_excluded() {
    // Service entrypoints are explicit config (`[[crates.services.entrypoints]]`)
    // and must always be emitted on the wrapper, even when the same method
    // appears in `exclude.methods`. The exclude list is used to suppress the
    // *standard* type-method placeholder for items that can't be auto-delegated
    // (consuming-self), not to suppress the wrapper class's run/finalize hooks.
    let surface = make_fixture_surface();
    let mut config = make_test_config();
    config.exclude.methods.push("TestService.run".to_string());

    let output = gen_service_ts(&surface, "my_crate", &config);

    assert!(
        output.contains("async run(addr: string)"),
        "service entrypoint `run` must still be emitted even when in exclude.methods:\n{output}"
    );
    assert!(
        output.contains("export class TestService"),
        "service class must still be present:\n{output}"
    );
}

#[test]
fn rust_output_emits_entrypoint_free_fn_even_when_method_excluded() {
    let surface = make_fixture_surface();
    let mut config = make_test_config();
    config.exclude.methods.push("TestService.run".to_string());

    let output = gen_service_rs(&surface, &config);

    assert!(
        output.contains("pub async fn test_service_run"),
        "service entrypoint free fn must still be emitted even when in exclude.methods:\n{output}"
    );
    assert!(
        output.contains("pub struct RequestHandlerBridge"),
        "RequestHandlerBridge should still be present:\n{output}"
    );
}

#[test]
fn typescript_variant_verb_decorator_style() {
    use crate::core::ir::{RegistrationVariant, RegistrationVariantStyle};

    let mut surface = make_fixture_surface();

    if let Some(reg) = surface.services[0].registrations.first_mut() {
        reg.variants.push(RegistrationVariant {
            name: "get".to_owned(),
            overrides: vec![],
            wrapper_call: None,
            signature_params: vec![ParamDef {
                name: "path".to_owned(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                ..ParamDef::default()
            }],
            doc: Some("Register a GET handler.".to_owned()),
            style: RegistrationVariantStyle::VerbDecorator,
        });
    }

    let config = make_test_config();
    let output = gen_service_ts(&surface, "my_crate", &config);

    // VerbDecorator should emit only the direct form: get(path, handler): this
    assert!(
        output.contains("get(path: string, handler: (...args: any[]) => any): this"),
        "expected VerbDecorator form `get(path, handler): this` in output:\n{output}"
    );

    // Should return `this` for chaining
    assert!(
        output.contains("return this;"),
        "expected `return this;` for chaining in VerbDecorator form:\n{output}"
    );

    // Should NOT emit decorator-factory form
    let get_count = output.matches("  get(").count();
    assert_eq!(
        get_count, 1,
        "expected exactly one `get(` method in VerbDecorator style, found {}: {}",
        get_count, output
    );
}

#[test]
fn typescript_variant_builder_style() {
    use crate::core::ir::{RegistrationVariant, RegistrationVariantStyle};

    let mut surface = make_fixture_surface();

    if let Some(reg) = surface.services[0].registrations.first_mut() {
        reg.variants.push(RegistrationVariant {
            name: "get".to_owned(),
            overrides: vec![],
            wrapper_call: None,
            signature_params: vec![ParamDef {
                name: "path".to_owned(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                ..ParamDef::default()
            }],
            doc: Some("Register a GET handler.".to_owned()),
            style: RegistrationVariantStyle::Builder,
        });
    }

    let config = make_test_config();
    let output = gen_service_ts(&surface, "my_crate", &config);

    // Builder should emit only the decorator-factory form: get(path) returns a function
    assert!(
        output.contains("get(path: string): (fn: (...args: any[]) => any) => (...args: any[]) => any"),
        "expected Builder form `get(path): (fn) => ...` in output:\n{output}"
    );

    // Should return the handler unchanged (for decorator form)
    assert!(
        output.contains("return fn;"),
        "expected `return fn;` in Builder form:\n{output}"
    );

    // Should NOT emit direct form with handler parameter
    assert!(
        !output.contains("get(path: string, handler: (...args: any[]) => any): this"),
        "Builder form should not emit direct method with handler parameter:\n{output}"
    );
}

#[test]
fn typescript_variant_hybrid_style() {
    use crate::core::ir::{RegistrationVariant, RegistrationVariantStyle};

    let mut surface = make_fixture_surface();

    if let Some(reg) = surface.services[0].registrations.first_mut() {
        reg.variants.push(RegistrationVariant {
            name: "get".to_owned(),
            overrides: vec![],
            wrapper_call: None,
            signature_params: vec![ParamDef {
                name: "path".to_owned(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                ..ParamDef::default()
            }],
            doc: Some("Register a GET handler.".to_owned()),
            style: RegistrationVariantStyle::Hybrid,
        });
    }

    let config = make_test_config();
    let output = gen_service_ts(&surface, "my_crate", &config);

    // Hybrid should emit both forms
    assert!(
        output.contains("get(path: string, handler: (...args: any[]) => any): this"),
        "expected Hybrid to include direct form `get(path, handler): this`:\n{output}"
    );

    assert!(
        output.contains("get(path: string): (fn: (...args: any[]) => any) => (...args: any[]) => any"),
        "expected Hybrid to include factory form `get(path): (fn) => ...`:\n{output}"
    );

    // Should have both `return this;` and `return fn;`
    let this_count = output.matches("return this;").count();
    let fn_count = output.matches("return fn;").count();
    assert!(
        this_count >= 1 && fn_count >= 1,
        "Hybrid form should have both return forms; this={}, fn={}: {}",
        this_count,
        fn_count,
        output
    );
}

#[test]
fn rust_output_emits_entrypoint_methods_with_inner_accessor() {
    let config = {
        let mut cfg = make_test_config();
        // Register TestService with a host_app_inner_accessor so entrypoint methods are emitted
        cfg.services = vec![crate::core::config::ServiceConfig {
            owner_type: "TestService".to_string(),
            constructor: None,
            configurators: vec![],
            registrations: vec![],
            entrypoints: vec![],
            skip_languages: vec![],
            host_app_inner_accessor: Some("self.inner.lock().expect(\"mutex poisoned\")".to_string()),
        }];
        cfg
    };

    let api = make_fixture_surface();
    let output = gen_service_rs(&api, &config);

    // Should emit entrypoint method on the wrapper class (not just free function)
    assert!(
        output.contains("#[napi(js_name = \"nativeRun\")]"),
        "entrypoint method should have napi attribute with js_name; output:\n{output}"
    );

    assert!(
        output.contains("pub async fn run(&self, addr: String)"),
        "entrypoint method should be emitted as async method on wrapper; output:\n{output}"
    );

    // Should use the configured inner accessor to move the owner out before awaiting.
    assert!(
        output.contains("let mut guard = self.inner.lock().expect(\"mutex poisoned\");"),
        "entrypoint method should use configured inner accessor; output:\n{output}"
    );

    assert!(
        output.contains("owner.run(addr)"),
        "entrypoint method should call the inner method; output:\n{output}"
    );

    // The free function should still be emitted for backward compatibility
    assert!(
        output.contains("pub async fn test_service_run("),
        "free function entrypoint should still be emitted; output:\n{output}"
    );
}

#[test]
fn rust_output_skips_entrypoint_methods_without_inner_accessor() {
    let config = make_test_config();
    // No host_app_inner_accessor configured, so entrypoint methods should NOT be emitted

    let api = make_fixture_surface();
    let output = gen_service_rs(&api, &config);

    // Should NOT emit entrypoint method on the wrapper class
    assert!(
        !output.contains("#[napi(js_name = \"nativeRun\")]"),
        "entrypoint method should not be emitted without host_app_inner_accessor; output:\n{output}"
    );

    // The free function should still be emitted
    assert!(
        output.contains("pub async fn test_service_run("),
        "free function entrypoint should still be emitted; output:\n{output}"
    );
}
