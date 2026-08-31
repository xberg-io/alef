//! Tests for Rustler service-API generation.

use super::*;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{
    ApiSurface, EntrypointDef, EntrypointKind, HandlerContractDef, MethodDef, ParamDef, PrimitiveType, RegistrationDef,
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

/// `gen_service_ex` emits a module named after the service owner.
#[test]
fn elixir_output_contains_service_module() {
    let surface = make_fixture_surface();
    let output = gen_service_ex(&surface, "");
    assert!(
        output.contains("defmodule TestService do"),
        "expected `defmodule TestService do` in output:\n{output}"
    );
}

/// `gen_service_ex` emits a struct definition.
#[test]
fn elixir_output_contains_struct_definition() {
    let surface = make_fixture_surface();
    let output = gen_service_ex(&surface, "");
    assert!(
        output.contains("defstruct"),
        "expected `defstruct` in output:\n{output}"
    );
    assert!(
        output.contains(":registrations"),
        "expected `:registrations` field in output:\n{output}"
    );
}

/// `gen_service_ex` emits a constructor.
#[test]
fn elixir_output_contains_constructor() {
    let surface = make_fixture_surface();
    let output = gen_service_ex(&surface, "");
    assert!(output.contains("def new("), "expected `def new(` in output:\n{output}");
}

/// `gen_service_ex` emits configurator methods.
#[test]
fn elixir_output_contains_configurator() {
    let surface = make_fixture_surface();
    let output = gen_service_ex(&surface, "");
    assert!(
        output.contains("def with_timeout("),
        "expected `with_timeout` configurator:\n{output}"
    );
}

/// `gen_service_ex` emits a registration method.
#[test]
fn elixir_output_contains_registration() {
    let surface = make_fixture_surface();
    let output = gen_service_ex(&surface, "");
    assert!(
        output.contains("def add_handler("),
        "expected `add_handler` registration method:\n{output}"
    );
}

/// `gen_service_ex` emits a GenServer module.
#[test]
fn elixir_output_contains_genserver_module() {
    let surface = make_fixture_surface();
    let output = gen_service_ex(&surface, "");
    assert!(
        output.contains("defmodule TestService.Handler do"),
        "expected `TestService.Handler` GenServer:\n{output}"
    );
    assert!(
        output.contains("use GenServer"),
        "expected `use GenServer` in output:\n{output}"
    );
}

/// The GenServer module must not contain a double blank line between `handle_info` and
/// `decode_args_and_dispatch` — `mix format` collapses any run of 2+ blank lines to a single
/// blank line, and a consumer's own `mix format` (pre-commit hook, CI) rewriting the file
/// after alef has already hashed and stamped it strands the file outside alef's ownership on
/// the next `alef generate` (the regenerated bytes no longer match what's on disk).
#[test]
fn genserver_module_has_no_double_blank_lines() {
    let surface = make_fixture_surface();
    let output = gen_service_ex(&surface, "");
    assert!(
        !output.contains("\n\n\n"),
        "generated service module must not contain a double blank line; got:\n{output}"
    );
}

/// Negative control: a single blank line between functions (the common, intentional case)
/// must survive unaltered — catches an overzealous fix that strips every blank line instead
/// of just collapsing doubled ones.
#[test]
fn genserver_module_keeps_single_blank_lines_between_functions() {
    let surface = make_fixture_surface();
    let output = gen_service_ex(&surface, "");
    assert!(
        output.contains("    end\n\n    defp decode_args_and_dispatch"),
        "expected a single blank line before `decode_args_and_dispatch`; got:\n{output}"
    );
}

/// Whether `mix` runs, not merely resolves: a version-manager shim spawns fine then
/// exits non-zero, so a spawn check leaves the skip below unreachable and fires the
/// assert everywhere Elixir is absent. ~keep
fn mix_is_runnable() -> bool {
    static RUNNABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *RUNNABLE.get_or_init(|| {
        std::process::Command::new("mix")
            .arg("--version")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    })
}

/// Full round-trip of the `service_api_genserver.ex.jinja` template against the real `mix
/// format` binary, mirroring the self-skipping shape Go's formatter round-trip tests use
/// elsewhere in this codebase (e.g. `e2e::codegen::go::snippet::snippet_matches_gofmt_when_available`):
/// skips when `mix` is not on `PATH`, so `genserver_module_has_no_double_blank_lines` above
/// still runs the structural assertion on every machine and this test only exercises the real
/// formatter where it can.
#[test]
fn genserver_template_matches_mix_format_when_available() {
    use std::io::Write as _;

    let rendered = crate::backends::rustler::template_env::render(
        "service_api_genserver.ex.jinja",
        minijinja::context! { server_module => "TestService.Handler" },
    );
    let code = format!("defmodule Wrapper do\n{rendered}end\n");

    if !mix_is_runnable() {
        return;
    }
    let Ok(mut child) = std::process::Command::new("mix")
        .args(["format", "-"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    else {
        return;
    };
    child
        .stdin
        .take()
        .expect("mix stdin")
        .write_all(code.as_bytes())
        .expect("write Elixir source");
    let output = child.wait_with_output().expect("wait for mix format");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(
        String::from_utf8(output.stdout).expect("mix format output is UTF-8"),
        code,
        "service_api_genserver.ex.jinja output must already be mix-format-canonical"
    );
}

/// `gen_service_ex` emits the `run` entrypoint.
#[test]
fn elixir_output_contains_run_entrypoint() {
    let surface = make_fixture_surface();
    let output = gen_service_ex(&surface, "");
    assert!(output.contains("def run("), "expected `def run(` in output:\n{output}");
}

/// `gen_service_rs` emits the handler bridge struct.
#[test]
fn rust_output_contains_handler_bridge_struct() {
    let surface = make_fixture_surface();
    let config = make_test_config();
    let output = gen_service_rs(&surface, &config);
    assert!(
        output.contains("pub struct ElixirRequestHandlerBridge"),
        "expected `ElixirRequestHandlerBridge` struct:\n{output}"
    );
}

/// `gen_service_rs` emits the handler bridge trait impl.
#[test]
fn rust_output_contains_handler_bridge_impl() {
    let surface = make_fixture_surface();
    let config = make_test_config();
    let output = gen_service_rs(&surface, &config);
    assert!(
        output.contains("impl my_crate::RequestHandler for ElixirRequestHandlerBridge"),
        "expected trait impl:\n{output}"
    );
    assert!(
        output.contains("fn handle(") && output.contains("Pin<Box<dyn std::future::Future<Output"),
        "expected boxed-future dispatch method:\n{output}"
    );
}

/// `gen_service_rs` emits the `#[rustler::nif]` run entry point.
#[test]
fn rust_output_contains_nif_run() {
    let surface = make_fixture_surface();
    let config = make_test_config();
    let output = gen_service_rs(&surface, &config);
    assert!(
        output.contains("#[rustler::nif(schedule = \"DirtyCpu\")]"),
        "expected `#[rustler::nif(schedule = \"DirtyCpu\")]` attribute:\n{output}"
    );
    assert!(
        output.contains("pub fn test_service_run("),
        "expected `test_service_run` function:\n{output}"
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
    assert!(paths.contains(&"service.ex"), "expected service.ex in output");
}

/// Full `generate()` returns empty for a surface with no services.
#[test]
fn generate_returns_empty_for_no_services() {
    let surface = ApiSurface::default();
    let config = make_test_config();
    let files = generate(&surface, &config).expect("generate should not fail");
    assert!(files.is_empty(), "expected no files for surface without services");
}

/// Elixir GenServer `handle_info` actually decodes args and calls handler.
///
/// Regression test: the Rust bridge sends `{:trait_call, ...}` via raw
/// `send`/`send_and_clear`, which BEAM delivers to `handle_info/2`, not
/// `handle_cast/2`. A `handle_cast`-based GenServer silently discards the
/// message (falls through to the default `handle_info` no-op), hanging the
/// request forever.
#[test]
fn elixir_genserver_handle_info_decodes_args_and_dispatches() {
    let surface = make_fixture_surface();
    let output = gen_service_ex(&surface, "");

    assert!(
        output.contains("def handle_info({:trait_call, method, args, reply_id}, registrations) do"),
        "expected trait_call dispatch on handle_info/2, not handle_cast/2:\n{output}"
    );
    assert!(
        !output.contains("handle_cast({:trait_call,"),
        "found trait_call dispatch on handle_cast/2 — raw send/2 is delivered to handle_info/2, not handle_cast/2:\n{output}"
    );

    assert!(
        output.contains("decode_args_and_dispatch(method, args, registrations)"),
        "expected decode_args_and_dispatch call in handle_info:\n{output}"
    );

    assert!(
        output.contains("Native.complete_trait_call(reply_id, response)"),
        "expected Native.complete_trait_call(reply_id, response) call:\n{output}"
    );

    assert!(
        !output.contains("simplified stub"),
        "found 'simplified stub' comment — dispatch should not be stubbed:\n{output}"
    );
    assert!(
        !output.contains("placeholder"),
        "found unsupported comment in dispatch logic:\n{output}"
    );
    assert!(
        !output.contains("# This is a simplified stub"),
        "found stub marker in dispatch:\n{output}"
    );
}

/// Elixir GenServer dispatch helper receives a native args map and calls the registered handler.
#[test]
fn elixir_genserver_dispatch_helper_invokes_handler() {
    let surface = make_fixture_surface();
    let output = gen_service_ex(&surface, "");

    assert!(
        output.contains("defp decode_args_and_dispatch(method, args, registrations) do"),
        "expected decode_args_and_dispatch helper function:\n{output}"
    );

    assert!(
        !output.contains("Jason.decode"),
        "args must arrive as a native map, not be Jason.decode'd:\n{output}"
    );

    assert!(
        output.contains("response = handler.(args)"),
        "expected handler.(args) invocation:\n{output}"
    );

    assert!(
        output.contains("Jason.encode(response)"),
        "expected Jason.encode(response) in dispatch:\n{output}"
    );

    assert!(
        output.contains("defp find_handler"),
        "expected find_handler helper function:\n{output}"
    );
}

/// Rust NIF parses registrations and constructs service owner.
#[test]
fn rust_nif_parses_registrations_and_constructs_owner() {
    let surface = make_fixture_surface();
    let config = make_test_config();
    let output = gen_service_rs(&surface, &config);

    assert!(
        output.contains("let registration_list: Vec<rustler::Term<'_>> = registrations"),
        "expected registration list parsing in NIF:\n{output}"
    );

    assert!(
        output.contains("let mut owner = my_crate::TestService::new()"),
        "expected owner construction in NIF:\n{output}"
    );

    assert!(
        output.contains("for reg_entry in registration_list"),
        "expected registration iteration in NIF:\n{output}"
    );

    assert!(
        !output.contains("placeholder: parse registrations"),
        "found placeholder in registration parsing — should be implemented:\n{output}"
    );
    assert!(
        !output.contains("For now, return a stub"),
        "found stub return in NIF — should be fully implemented:\n{output}"
    );
}

/// No empty-JSON or stub responses in generated code.
///
/// Verifies that the Rust NIF actually invokes `owner.run(...)` or `owner.finalize(...)`
/// and does not emit stub placeholder responses.
#[test]
fn no_stub_responses_in_generated_code() {
    let surface = make_fixture_surface();
    let config = make_test_config();

    let elixir_output = gen_service_ex(&surface, "");
    let rust_output = gen_service_rs(&surface, &config);

    assert!(
        !elixir_output.contains("response = {:ok, %{}}"),
        "found stub response {{:ok, %{{}}}} in Elixir generated code:\n{elixir_output}"
    );

    assert!(
        !elixir_output.contains("# Native.complete_trait_call"),
        "found commented-out complete_trait_call in Elixir:\n{elixir_output}"
    );

    assert!(
        !rust_output.contains("would be called here"),
        "found 'would be called here' stub comment in Rust NIF:\n{rust_output}"
    );
    assert!(
        !rust_output.contains("would happen here"),
        "found 'would happen here' stub comment in Rust NIF:\n{rust_output}"
    );

    assert!(
        rust_output.contains("owner.run(") || rust_output.contains("owner.finalize("),
        "Rust NIF should call owner.run(...) or owner.finalize(...), found neither:\n{rust_output}"
    );

    assert!(
        rust_output.contains("ElixirRequestHandlerBridge"),
        "Rust NIF should create handler bridge instances:\n{rust_output}"
    );

    assert!(
        !rust_output.contains("): Result<"),
        "found illegal if-let type ascription pattern '): Result<' in generated Rust:\n{rust_output}"
    );

    assert!(
        rust_output.contains("Term<'_>"),
        "expected lifetime-annotated Term<'_> in generated Rust NIF signature:\n{rust_output}"
    );
}

/// Verify that registration variant style is respected in generated Elixir code.
///
/// Regression test for issue #26: the rustler backend must pattern-match on
/// `RegistrationVariantStyle` and emit the appropriate Elixir registration forms.
#[test]
fn registration_variant_style_hybrid_emits_both_forms() {
    let mut surface = make_fixture_surface();
    let _config = make_test_config();

    surface.services[0].registrations[0]
        .variants
        .push(crate::core::ir::RegistrationVariant {
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
            doc: None,
            style: RegistrationVariantStyle::Hybrid,
            ..Default::default()
        });

    let elixir_output = gen_service_ex(&surface, "");

    assert!(
        elixir_output.contains("def get(app, path, handler) do"),
        "expected verb-decorator form 'def get(app, path, handler) do' in Elixir output:\n{elixir_output}"
    );

    assert!(
        elixir_output.contains("def get_decorator(app, path) do"),
        "expected builder form 'def get_decorator(app, path) do' in Elixir output:\n{elixir_output}"
    );
}

/// Verify that send_trait_call message is emitted in generated handler bridge.
///
/// Regression test for issue #119: the handler bridge must send the trait_call message
/// to the Elixir GenServer via OwnedEnv::send_and_clear, not just await silently.
#[test]
fn handler_bridge_sends_trait_call_message() {
    let surface = make_fixture_surface();
    let config = make_test_config();

    let rust_output = gen_service_rs(&surface, &config);

    assert!(
        rust_output.contains("OwnedEnv"),
        "expected OwnedEnv import in generated code"
    );

    assert!(
        rust_output.contains("env.send_and_clear(&pid"),
        "expected env.send_and_clear(&pid, ...) call in generated handler bridge:\n{rust_output}"
    );

    assert!(
        rust_output.contains("Atom::from_str(env, \"trait_call\")"),
        "expected atom::from_str for 'trait_call' in generated message:\n{rust_output}"
    );

    assert!(
        rust_output.contains("method_name"),
        "expected method_name variable in trait_call message"
    );

    assert!(
        rust_output.contains("request_json_clone"),
        "expected request JSON to be sent in trait_call message"
    );

    assert!(
        rust_output.contains("reply_id)"),
        "expected reply_id in trait_call tuple"
    );

    assert!(
        !rust_output.contains("// crate::nif_support::send_trait_call"),
        "found old commented-out send_trait_call in output — should be replaced with real call"
    );

    assert!(
        rust_output.contains("tokio::task::spawn_blocking(move || {"),
        "expected spawn_blocking to wrap the message send"
    );
}

/// Verify that Rust codegen emits core crate import + trait implementation.
/// This tests GAP 1 (core import) and GAP 3 (trait cast).
#[test]
fn rust_codegen_emits_core_import_and_trait_impl() {
    let surface = make_fixture_surface();
    let config = make_test_config();
    let rust_output = gen_service_rs(&surface, &config);

    assert!(
        rust_output.contains("use my_crate::*;"),
        "expected core crate wildcard import in gen_service_rs output:\n{rust_output}"
    );

    assert!(
        rust_output.contains("impl my_crate::RequestHandler for ElixirRequestHandlerBridge"),
        "expected trait impl for bridge in generated output:\n{rust_output}"
    );

    assert!(
        rust_output.contains("let handler: Arc<dyn my_crate::RequestHandler> = Arc::new(bridge);"),
        "expected handler trait cast in registration code:\n{rust_output}"
    );

    assert!(
        rust_output.contains("pub struct ElixirRequestHandlerBridge"),
        "expected ElixirRequestHandlerBridge struct definition:\n{rust_output}"
    );
}

/// Build on [`make_fixture_surface`] with a registration method named `"route"`
/// (the only name that triggers `HandlerWrapper` emission, see
/// `registration.rs:40`) whose sole metadata param is an **opaque** type
/// (`RouteBuilder`) present in `api.types`. `RouteBuilder` also carries one
/// chainable builder method (`&mut self -> Self`) so the `returns_self`
/// re-wrap path can be exercised.
///
/// Coverage gap this closes: `make_fixture_surface()` names its registration
/// `"add_handler"` with only string metadata params and an empty `api.types`,
/// so `service_api_handler_wrapper.ex.jinja` and the opaque-metadata `.ref`
/// unwrap path were entirely untested before this fixture existed.
fn make_route_fixture_surface() -> ApiSurface {
    let mut surface = make_fixture_surface();

    surface.services[0].registrations[0].method = "route".to_owned();
    // The wrapper gate dispatches on `handler_shape`, not the method name — set it
    // explicitly so this fixture keeps exercising `HandlerWrapper` emission. ~keep
    surface.services[0].registrations[0].handler_shape = crate::core::ir::HandlerShape::ContextObject;
    surface.services[0].registrations[0].metadata_params = vec![ParamDef {
        name: "builder".to_owned(),
        ty: TypeRef::Named("RouteBuilder".to_owned()),
        optional: false,
        default: None,
        ..ParamDef::default()
    }];

    let chainable_method = MethodDef {
        name: "request_timeout".to_owned(),
        params: vec![ParamDef {
            name: "ms".to_owned(),
            ty: TypeRef::Primitive(PrimitiveType::U64),
            optional: false,
            default: None,
            ..ParamDef::default()
        }],
        return_type: TypeRef::Named("RouteBuilder".to_owned()),
        is_async: false,
        is_static: false,
        error_type: None,
        doc: "Set the per-route request timeout.".to_owned(),
        receiver: Some(crate::core::ir::ReceiverKind::RefMut),
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

    let route_builder_type = crate::core::ir::TypeDef {
        name: "RouteBuilder".to_owned(),
        rust_path: "my_crate::RouteBuilder".to_owned(),
        is_opaque: true,
        methods: vec![chainable_method],
        doc: "Builder for per-route metadata.".to_owned(),
        ..crate::core::ir::TypeDef::default()
    };
    surface.types.push(route_builder_type);

    surface
}

/// Opaque metadata params must be unwrapped with `.ref` in the registration
/// method's metadata tuple, matching the `rustler::ResourceArc<T>` the Rust
/// NIF side decodes (`rust.rs` / `registration_nif.rs`).
///
/// Regression test for defect 1: the Elixir side previously emitted the bare
/// param name (`{builder}`), passing the `%RouteBuilder{ref: ...}` wrapper
/// struct itself where Rustler expected the resource reference.
#[test]
fn elixir_opaque_metadata_param_unwraps_ref_in_registration_tuple() {
    let surface = make_route_fixture_surface();
    let output = gen_service_ex(&surface, "");

    assert!(
        output.contains("entry = {\"route\", {builder.ref}, handler_pid}"),
        "expected opaque metadata param unwrapped via `.ref` in the registration tuple:\n{output}"
    );
    assert!(
        !output.contains("entry = {\"route\", {builder}, handler_pid}"),
        "found raw wrapper struct (not `.ref`) passed as opaque metadata:\n{output}"
    );
}

/// A registration method named `"route"` emits the `HandlerWrapper` GenServer
/// and it dispatches `{:trait_call, ...}` on `handle_info/2`, matching the
/// raw-`send/2` wire contract used by `service_api_handler_bridge.rs.jinja`.
///
/// Regression test for defect 3: `HandlerWrapper` previously implemented
/// `handle_cast/2`, so the Rust bridge's `send_and_clear` message fell
/// through to the default `handle_info/2` and was silently discarded,
/// hanging every request that reached a closure-based handler forever.
#[test]
fn elixir_handler_wrapper_dispatches_on_handle_info() {
    let surface = make_route_fixture_surface();
    let output = gen_service_ex(&surface, "");

    assert!(
        output.contains("defmodule HandlerWrapper do"),
        "expected HandlerWrapper module for a `route`-named registration:\n{output}"
    );
    assert!(
        output.contains("def handle_info({:trait_call, _method, args_json, reply_id}, handler_fn) do"),
        "expected HandlerWrapper to dispatch trait_call on handle_info/2:\n{output}"
    );
    assert!(
        !output.contains("handle_cast({:trait_call,"),
        "found trait_call dispatch on handle_cast/2 in HandlerWrapper — raw send/2 lands on handle_info/2:\n{output}"
    );
}

/// Chainable opaque builder methods (`&mut self -> Self`) must re-wrap the
/// NIF's returned reference in `%__MODULE__{ref: ...}`, not return the bare
/// reference.
///
/// Regression test for defect 2: `returns_self` was gated on
/// `is_static && returns_self`, so every receiver-based chainable method
/// (the only kind builder chains actually use) fell through to the bare
/// `Native.<nif>(...)` call, degrading the struct to a raw `reference()` as
/// soon as any option was chained.
#[test]
fn elixir_opaque_builder_chain_method_rewraps_ref() {
    let surface = make_route_fixture_surface();
    let route_builder = surface.types.iter().find(|t| t.name == "RouteBuilder").unwrap();
    let config = make_test_config();

    let output = crate::backends::rustler::gen_bindings::helpers::gen_elixir_opaque_module(route_builder, "", &config);

    assert!(
        output.contains("ref = Native.routebuilder_request_timeout(obj.ref, ms)"),
        "expected chainable builder method to call the NIF and capture the returned ref:\n{output}"
    );
    assert!(
        output.contains("%__MODULE__{ref: ref}"),
        "expected chainable builder method to re-wrap the returned ref in the struct:\n{output}"
    );
    assert!(
        !output.contains("Native.routebuilder_request_timeout(obj.ref, ms)\n  end"),
        "found bare NIF call as the method body — chainable method must return the wrapped struct:\n{output}"
    );
}

/// The `HandlerWrapper` gate dispatches on `handler_shape`, not on the registration
/// method being literally named `"route"`. `make_fixture_surface()` names its
/// registration `"add_handler"` (matching alef's own dogfood config) — proving the
/// wrapper still renders here demonstrates the gate is decoupled from that
/// product-specific method name.
#[test]
fn elixir_handler_wrapper_emits_for_context_object_registration_named_add_handler() {
    let mut surface = make_fixture_surface();
    assert_eq!(surface.services[0].registrations[0].method, "add_handler");
    surface.services[0].registrations[0].handler_shape = crate::core::ir::HandlerShape::ContextObject;

    let output = gen_service_ex(&surface, "");

    assert!(
        output.contains("defmodule HandlerWrapper do"),
        "expected HandlerWrapper module for a context_object-shaped `add_handler` registration:\n{output}"
    );
    assert!(
        output.contains("def handle_info({:trait_call, _method, args_json, reply_id}, handler_fn) do"),
        "expected HandlerWrapper to dispatch trait_call on handle_info/2:\n{output}"
    );
}

/// A `bare_callable` registration (the default) must NOT emit `HandlerWrapper` —
/// only `context_object`-shaped registrations need the `Conn`-building adapter.
#[test]
fn elixir_handler_wrapper_absent_for_bare_callable_registration() {
    let surface = make_fixture_surface();
    assert_eq!(
        surface.services[0].registrations[0].handler_shape,
        crate::core::ir::HandlerShape::BareCallable
    );

    let output = gen_service_ex(&surface, "");

    assert!(
        !output.contains("defmodule HandlerWrapper do"),
        "bare_callable registration should not emit HandlerWrapper:\n{output}"
    );
}

fn make_test_config() -> ResolvedCrateConfig {
    use crate::core::config::resolved::ResolvedCrateConfig;
    ResolvedCrateConfig {
        name: "my-crate".to_owned(),
        ..ResolvedCrateConfig::default()
    }
}
