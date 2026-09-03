use super::*;
use crate::core::ir::{
    EntrypointDef, EntrypointKind, HandlerContractDef, MethodDef, ParamDef, RegistrationDef, ServiceDef, TypeRef,
};
use crate::test_support::toolchain;

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
    assert!(go.contains("//export service_handler_response_free"));
    assert!(go.contains("import \"C\""));
    assert!(go.contains("owner C.uint64_t"));
}

/// A `go` that actually runs, or `None` when it is not installed.
///
/// Delegates to the crate-wide [`toolchain::GO`] gate rather than calling `which::which` here, so
/// the two real `go test` compile checks below are counted in the same attempted/executed census
/// every other generated-Go fixture reports into -- a skip that nothing counts is the failure
/// mode the gate exists to close, and this file used to have its own uncounted copy of it. The
/// gate also panics rather than skipping when `ALEF_REQUIRE_GO` is set, which is what makes a
/// runner whose Go setup silently regressed fail loudly. ~keep
fn required_go() -> Option<std::path::PathBuf> {
    toolchain::GO.open()
}

#[test]
fn go_service_response_deallocator_compiles_and_runs_when_go_is_available() {
    let Some(go) = required_go() else {
        return;
    };
    let directory = tempfile::tempdir().expect("temporary Go allocator directory");
    std::fs::write(directory.path().join("test_crate.h"), GO_ALLOCATOR_HEADER).expect("write allocator header");
    let preamble = crate::backends::go::template_env::render(
        "service_file_preamble.jinja",
        minijinja::context! { pkg_name => "binding", ffi_header => "test_crate.h" },
    );
    let registry = crate::backends::go::template_env::render("service_handler_registry.jinja", minijinja::context! {});
    let source = format!("{preamble}\n{registry}\n{GO_ALLOCATOR_CONTROL}");
    std::fs::write(directory.path().join("service.go"), source).expect("write generated Go allocator control");
    std::fs::write(directory.path().join("service_test.go"), GO_ALLOCATOR_TEST).expect("write Go allocator test");
    let output = std::process::Command::new(go)
        .args(["test", "-vet=off", "./..."])
        .env("GO111MODULE", "off")
        .current_dir(directory.path())
        .output()
        .expect("run Go allocator control");
    assert!(
        output.status.success(),
        "generated Go allocator control failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

const GO_ALLOCATOR_HEADER: &str = r#"#include <stdint.h>
#include <stdlib.h>
#include <string.h>
typedef char *(*handler_callback_t)(void *, const char *);
typedef void (*handler_response_free_t)(char *);
static inline int allocator_control(
    handler_callback_t callback,
    handler_response_free_t response_free,
    void *context
) {
    char *response = callback(context, "{}");
    int valid = response != NULL && strcmp(response, "{}") == 0;
    response_free(response);
    return valid ? 17 : -1;
}
"#;

const GO_ALLOCATOR_CONTROL: &str = r#"
var _ = fmt.Sprintf
var _ net.IP
var _ = time.Second

func allocatorControl() int {
    id := registerHandler(func(_ []byte) ([]byte, error) { return []byte("{}"), nil })
    defer unregisterHandler(id)
    return int(C.allocator_control(
        C.get_service_handler_callback(),
        C.get_service_handler_response_free(),
        unsafe.Pointer(id),
    ))
}
"#;

const GO_ALLOCATOR_TEST: &str = r#"package binding
import "testing"
func TestAllocatorControl(t *testing.T) {
    if status := allocatorControl(); status != 17 { t.Fatalf("status = %d", status) }
}
"#;

/// `service_c_imports_comment.jinja`'s per-registration and per-entrypoint C param
/// loops close with `{% for param in ... %}...{{ param.name }}{% endfor +%}` — the
/// `+` on `endfor` is load bearing: minijinja's `trim_blocks(true)` (set in
/// `template_env::make_env`) strips the newline following *any* block tag,
/// including a `{% endfor %}` that closes a same-line loop, so without `+%}` the
/// closing `// );` line merges onto the last parameter's comment line instead of
/// starting its own. This fixture's `add_handler` registration has two metadata
/// params and its `run` entrypoint has one, so both loops execute and both need
/// their trailing `// );` on a line of its own.
#[test]
fn service_c_imports_comment_puts_closing_paren_on_its_own_line() {
    let api = make_fixture_surface();
    let config = ResolvedCrateConfig {
        name: "test_crate".to_owned(),
        ..ResolvedCrateConfig::default()
    };

    let go = gen_service_go(&api, &config, "binding", "TEST_CRATE");

    assert!(
        go.contains(concat!(
            "// extern int test_crate_test_service_register_add_handler(\n",
            "//     uint64_t owner,\n",
            "//     char* (*callback)(void*, const char*),\n",
            "//     void (*response_free)(char*),\n",
            "//     void* context,\n",
            "//     const char* method,\n",
            "//     const char* path\n",
            "// );\n",
        )),
        "registration comment block did not render with `// );` on its own line:\n{go}"
    );
    assert!(
        go.contains(concat!(
            "// extern void test_crate_test_service_ep_run(\n",
            "//     uint64_t owner,\n",
            "//     const char* addr\n",
            "// );\n",
        )),
        "entrypoint comment block did not render with `// );` on its own line:\n{go}"
    );
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
    assert!(go.contains("owner C.uint64_t"));
    assert!(go.contains("test_crate_test_service_free(s.owner)"));
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
    assert!(go.contains("C.get_service_handler_response_free(),"));
    assert!(!go.contains("(*C.TEST_CRATETestServiceOpaque)"));
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
    assert!(!go.contains("(*C.TEST_CRATETestServiceOpaque)"));
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
    assert!(go.contains("//export service_handler_response_free"));
    assert!(go.contains("C.free(unsafe.Pointer(response))"));
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

#[test]
fn marshalled_dto_handle_uses_scalar_zero_sentinel() {
    let api = ApiSurface {
        types: vec![crate::core::ir::TypeDef {
            name: "Config".into(),
            has_serde: true,
            ..crate::core::ir::TypeDef::default()
        }],
        ..ApiSurface::default()
    };
    let (setup, argument) = service_c_arg_expr_with_marshal("config", &TypeRef::Named("Config".into()), &api, "sample");
    assert_eq!(argument, "c_config");
    assert!(setup.contains("if c_config == 0"), "{setup}");
    assert!(!setup.contains("if c_config == nil"), "{setup}");

    let Some(go) = required_go() else {
        return;
    };
    let directory = tempfile::tempdir().expect("temporary Go scalar-handle directory");
    let source = format!(
        r#"package sample
/*
#include <stdint.h>
#include <stdlib.h>
typedef uint64_t SAMPLEAlefHandle;
static SAMPLEAlefHandle sample_config_from_json(char *value) {{ (void)value; return 0; }}
static void sample_config_free(SAMPLEAlefHandle value) {{ (void)value; }}
*/
import "C"
import (
    "encoding/json"
    "errors"
)
type Config struct{{}}
func use(config Config) error {{
{setup}
    _ = {argument}
    return nil
}}
"#
    );
    std::fs::write(directory.path().join("scalar_handle.go"), source).expect("write neutral Go source");
    let output = std::process::Command::new(go)
        .args(["test", "./..."])
        .env("GO111MODULE", "off")
        .current_dir(directory.path())
        .output()
        .expect("run Go compiler");
    assert!(
        output.status.success(),
        "generated Go scalar-handle setup failed to compile:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Keeps required mode wired to a job that actually has `go` on `PATH`.
///
/// The env var alone is not enough, and assuming it was is exactly how this went wrong: the
/// previous version of this test asserted only `ALEF_REQUIRE_GO`, on the stated grounds that
/// "GitHub-hosted runner images preinstall a Go LTS toolchain, so unlike
/// `ALEF_REQUIRE_JAVAC`/`ALEF_REQUIRE_DOTNET`/`ALEF_REQUIRE_KOTLINC` this needs no dedicated
/// `uses:` setup step". That is false for the arm64 macOS images, which carry no Go at all, so
/// setting the variable there only converted a silent skip into a permanent, ignored red on
/// `Test (macos-latest)` from 2026-08-31 onward. Requiring the explicit setup step means the
/// claim is now enforced rather than assumed. ~keep
#[test]
fn ci_requires_go_for_runtime_regressions() {
    let workflow = include_str!("../../../../../.github/workflows/ci.yml");

    assert!(
        workflow.contains("ALEF_REQUIRE_GO: \"1\""),
        "the test job must make a missing Go toolchain a hard failure"
    );
    assert!(
        workflow.contains("uses: actions/setup-go@"),
        "`ALEF_REQUIRE_GO` needs an explicit Go install on every leg of the matrix: the arm64 \
         macOS runner image does not preinstall one, so without this step the variable only \
         turns a skip into a permanently red leg"
    );
}
