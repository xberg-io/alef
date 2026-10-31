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
        crate::alef_context! { pkg_name => "binding", ffi_header => "test_crate.h" },
    );
    let registry = crate::backends::go::template_env::render("service_handler_registry.jinja", crate::alef_context! {});
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
            options: Vec::new(),
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

fn go_ci_step_runs(step: &serde_json::Value) -> bool {
    (step["continue-on-error"].is_null() || step["continue-on-error"] == false)
        && (step["if"].is_null() || step["if"] == true || matches!(step["if"].as_str(), Some("true" | "${{ true }}")))
}

fn go_ci_census_command_matches(command: &str, expected: &[&str]) -> bool {
    let mut words = command.split_whitespace();
    if words.next() != Some("scripts/toolchain-census.sh") {
        return false;
    }
    let arguments: Vec<_> = words.collect();
    if arguments.len() != expected.len() * 2 {
        return false;
    }
    let mut tools = Vec::new();
    for pair in arguments.as_chunks::<2>().0 {
        if pair[0] != "--require" || tools.contains(&pair[1]) {
            return false;
        }
        tools.push(pair[1]);
    }
    expected.iter().all(|tool| tools.contains(tool))
}

fn go_ci_census_script_runs(script: &str) -> bool {
    let lines: Vec<_> = script.lines().map(str::trim).filter(|line| !line.is_empty()).collect();
    lines.len() == 5
        && lines[0] == "if [ \"$RUNNER_OS\" = \"macOS\" ]; then"
        && lines[2] == "else"
        && lines[4] == "fi"
        && go_ci_census_command_matches(lines[1], &["go", "ruby", "swift", "wasm-bindgen"])
        && go_ci_census_command_matches(lines[3], &["go", "ruby", "wasm-bindgen"])
}

fn go_ci_census_runs(steps: &[serde_json::Value]) -> bool {
    steps.iter().any(|step| {
        let enabled = go_ci_step_runs(step) || step["if"] == "${{ !cancelled() }}";
        enabled
            && step["name"] == "Toolchain fixture census"
            && step["shell"] == "bash"
            && (step["continue-on-error"].is_null() || step["continue-on-error"] == false)
            && step["run"].as_str().is_some_and(go_ci_census_script_runs)
    })
}

fn go_ci_requirement_is_enforced(workflow: &serde_json::Value) -> bool {
    let job = &workflow["jobs"]["test"];
    let Some(steps) = job["steps"].as_array() else {
        return false;
    };
    let Some(run) = steps.iter().position(|step| step["name"] == "Run tests") else {
        return false;
    };
    let required = &steps[run];
    go_ci_step_runs(job)
        && go_ci_census_runs(&steps[run + 1..])
        && go_ci_step_runs(required)
        && required["env"]["ALEF_REQUIRE_GO"] == "1"
        && required["run"].as_str().is_some_and(|run| {
            run.trim().lines().count() == 1
                && run
                    .split_whitespace()
                    .eq(["mise", "run", "test:run", "--", "--no-fail-fast"])
        })
        && steps[..run].iter().any(|step| {
            let tools = step["with"]["tools"]
                .as_str()
                .filter(|tools| !tools.trim().is_empty())
                .or_else(|| step["with"]["install-args"].as_str())
                .unwrap_or("");
            go_ci_step_runs(step)
                && step["uses"]
                    .as_str()
                    .is_some_and(|action| action.starts_with("xberg-io/actions/setup-mise@"))
                && step["with"]["install"] != false
                && step["with"]["install"] != "false"
                && tools
                    .split_whitespace()
                    .any(|tool| tool.split(['=', '@']).next() == Some("go"))
        })
}

#[test]
fn ci_requires_go_for_runtime_regressions() {
    let workflow = serde_saphyr::from_str(crate::test_support::ci_workflow::CI_WORKFLOW).expect("CI workflow YAML");
    assert!(
        go_ci_requirement_is_enforced(&workflow),
        "Test must install Go before its required-mode suite"
    );
}

fn go_ci_mutant(workflow: &serde_json::Value, setup: usize, mode: &str) -> serde_json::Value {
    let mut mutant = workflow.clone();
    let steps = mutant["jobs"]["test"]["steps"].as_array_mut().unwrap();
    match mode {
        "remove" => {
            steps.remove(setup);
        }
        "other-job" => {
            let moved = steps.remove(setup);
            mutant["jobs"]["unrelated"]["steps"] = serde_json::json!([moved]);
        }
        "after-tests" => {
            let moved = steps.remove(setup);
            steps.push(moved);
        }
        "disabled" => steps[setup]["if"] = serde_json::json!(false),
        "no-install" => steps[setup]["with"]["install"] = serde_json::json!(false),
        _ => {
            let run = steps.iter_mut().find(|step| step["name"] == "Run tests").unwrap();
            run["env"]["ALEF_REQUIRE_GO"] = serde_json::json!("0");
        }
    }
    mutant
}

#[test]
fn go_ci_guard_rejects_missing_displaced_and_disabled_provisioning() {
    let workflow: serde_json::Value =
        serde_saphyr::from_str(include_str!("../../../../../.github/workflows/ci.yml")).unwrap();
    let steps = workflow["jobs"]["test"]["steps"].as_array().unwrap();
    let setup = steps.iter().position(|step| step["with"]["tools"] == "go").unwrap();
    for mode in [
        "remove",
        "other-job",
        "after-tests",
        "disabled",
        "no-install",
        "no-required-mode",
    ] {
        let mutant = go_ci_mutant(&workflow, setup, mode);
        assert!(!go_ci_requirement_is_enforced(&mutant), "accepted {mode}");
    }
    let mut equivalent = workflow;
    equivalent["jobs"]["test"]["steps"][setup]["with"]["tools"] = serde_json::json!("node go uv");
    assert!(go_ci_requirement_is_enforced(&equivalent));
}

#[test]
fn go_ci_guard_rejects_narrowed_suites() {
    let workflow: serde_json::Value =
        serde_saphyr::from_str(include_str!("../../../../../.github/workflows/ci.yml")).unwrap();
    for command in [
        "mise run test:run -- --lib no_such_test",
        "mise\nrun test:run -- --no-fail-fast",
        "mise run test:run -- --help",
        "mise run test:run -- --no-fail-fast --lib no_such_test",
        "echo mise run test:run -- --no-fail-fast",
        "mise run test:run -- --no-fail-fast; true",
    ] {
        let mut mutant = workflow.clone();
        let run = mutant["jobs"]["test"]["steps"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|step| step["name"] == "Run tests")
            .unwrap();
        run["run"] = serde_json::json!(command);
        assert_ne!(mutant, workflow);
        assert!(
            !go_ci_requirement_is_enforced(&mutant),
            "accepted narrowed suite: {command}"
        );
    }
}

#[test]
fn go_ci_guard_rejects_unreachable_census_text() {
    let workflow: serde_json::Value =
        serde_saphyr::from_str(include_str!("../../../../../.github/workflows/ci.yml")).unwrap();
    let census = workflow["jobs"]["test"]["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|step| step["name"] == "Toolchain fixture census")
        .unwrap()["run"]
        .as_str()
        .unwrap();
    for script in [
        format!("cat <<'CENSUS'\n{census}\nCENSUS"),
        format!("if false; then\n{census}\nfi"),
        census.replace("if [ \"$RUNNER_OS\" = \"macOS\" ]; then", "if false; then"),
        census.replacen("--require go", "", 1),
        census.replace("--require go", ""),
    ] {
        let mut mutant = workflow.clone();
        let step = mutant["jobs"]["test"]["steps"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|step| step["name"] == "Toolchain fixture census")
            .unwrap();
        step["run"] = serde_json::json!(script);
        assert_ne!(mutant, workflow);
        assert!(
            !go_ci_requirement_is_enforced(&mutant),
            "accepted unreachable or incomplete census: {script}"
        );
    }
}

#[test]
fn go_ci_guard_rejects_ignored_failures_and_conditional_execution() {
    let workflow: serde_json::Value =
        serde_saphyr::from_str(include_str!("../../../../../.github/workflows/ci.yml")).unwrap();
    for name in ["job", "Run tests", "Set up Go", "Toolchain fixture census"] {
        for (field, value) in [
            ("continue-on-error", serde_json::json!(true)),
            ("if", serde_json::json!(false)),
            ("if", serde_json::json!("${{ runner.os == 'Linux' }}")),
        ] {
            let mut mutant = workflow.clone();
            let job = &mut mutant["jobs"]["test"];
            let target = if name == "job" {
                job
            } else {
                job["steps"]
                    .as_array_mut()
                    .unwrap()
                    .iter_mut()
                    .find(|step| step["name"] == name)
                    .unwrap()
            };
            target[field] = value;
            assert_ne!(mutant, workflow);
            assert!(!go_ci_requirement_is_enforced(&mutant), "accepted {name}: {field}");
        }
    }
}
