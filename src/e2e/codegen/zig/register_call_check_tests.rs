use super::*;
use crate::core::config::e2e::{ArgMapping, CallOverride};
use crate::core::config::{ResolvedCrateConfig, TraitBridgeConfig};
use crate::core::ir::{MethodDef, ReceiverKind, TypeDef, TypeRef};
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::Fixture;

/// A `TestTrait` with one infallible `u64`-returning method, wired into a
/// `[[crates.trait_bridges]]` entry the same way `register_embedding_backend` /
/// `register_ocr_backend` / etc. are wired in `alef.toml`. Reused by every test
/// below so the trait-bridge plumbing is identical and only the call config
/// (the thing actually under test) differs between cases.
fn trait_bridge_method() -> MethodDef {
    MethodDef {
        name: "probe".to_string(),
        params: vec![],
        return_type: TypeRef::Primitive(crate::core::ir::PrimitiveType::U64),
        is_async: false,
        is_static: false,
        error_type: None,
        doc: String::new(),
        receiver: Some(ReceiverKind::Ref),
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
    }
}

fn register_fixture() -> Fixture {
    Fixture {
        id: "register_test_backend_trait_bridge".into(),
        description: "register_test_backend: trait bridge".into(),
        args: vec![ArgMapping {
            name: "backend".to_string(),
            field: "backend".to_string(),
            arg_type: "test_backend".to_string(),
            optional: false,
            owned: false,
            element_type: None,
            go_type: None,
            vec_inner_is_ref: false,
            trait_name: Some("TestTrait".to_string()),
        }],
        assertions: vec![crate::e2e::fixture::Assertion {
            assertion_type: "not_error".into(),
            ..Default::default()
        }],
        ..Fixture::default()
    }
}

/// Base plumbing shared by both scenarios: an `e2e_config` whose default call
/// resolves to `function_name`, a `TraitBridgeConfig` matching `TestTrait`, and a
/// `type_defs` slice exposing `TestTrait`'s one method so `emit_test_backend` can
/// stub it.
fn base_e2e_config(function_name: &str, zig_override: CallOverride) -> (E2eConfig, ResolvedCrateConfig, Vec<TypeDef>) {
    let mut e2e = E2eConfig::default();
    e2e.call.function = function_name.to_string();
    e2e.call.overrides.insert("zig".into(), zig_override);

    let mut config = ResolvedCrateConfig::default();
    config.trait_bridges.push(TraitBridgeConfig {
        trait_name: "TestTrait".to_string(),
        super_trait: None,
        register_fn: Some(function_name.to_string()),
        ..Default::default()
    });

    let type_defs = vec![TypeDef {
        name: "TestTrait".to_string(),
        methods: vec![trait_bridge_method()],
        is_trait: true,
        ..Default::default()
    }];

    (e2e, config, type_defs)
}

/// The register-fn shape: the zig binding returns a raw `i32` status code and
/// writes an `out_error` pointer on failure (`returns_result = false`), matching
/// `pub fn register_embedding_backend(...) i32` in the real generated bindings.
/// The emitted call must check the return code instead of discarding it, and
/// must free the `out_error` allocation on the failure path.
#[test]
fn register_style_call_checks_return_code_and_frees_error_message() {
    let fixture = register_fixture();
    let (e2e, config, type_defs) = base_e2e_config(
        "register_test_backend",
        CallOverride {
            function: Some("register_test_backend".to_string()),
            returns_result: Some(false),
            ..Default::default()
        },
    );

    let rendered = render_test_file(
        "plugin_api",
        &[&fixture],
        &e2e,
        "register_test_backend",
        "result",
        &[],
        "sample",
        "sample",
        &config,
        &type_defs,
        &[],
        crate::e2e::codegen::call_ir::CallIr::default(),
        &[],
    );

    // The call's return value must be bound and checked, never bare-discarded.
    assert!(
        !rendered.contains("_ = sample.register_test_backend("),
        "return code must not be discarded via `_ = `, got:\n{rendered}"
    );
    assert!(
        rendered.contains("const _rc = sample.register_test_backend(") && rendered.contains("if (_rc != 0) {"),
        "expected the i32 return code to be captured and checked, got:\n{rendered}"
    );
    // A failure must be surfaced (naming the call) and fail the test.
    assert!(
        rendered.contains("register_test_backend failed:"),
        "failure message must name the failing call, got:\n{rendered}"
    );
    assert!(
        rendered.contains("return error.TestUnexpectedResult;"),
        "a non-zero return code must fail the test, got:\n{rendered}"
    );
    // The out_error allocation must be freed on the failure path, not leaked.
    assert!(
        rendered.contains("sample._free_string(_m)"),
        "out_error message must be freed on failure, got:\n{rendered}"
    );
    // Must not use `try` — the wrapper returns a raw i32, not a Zig error union.
    assert!(
        !rendered.contains("try sample.register_test_backend("),
        "register-fn shape returns i32, not an error union; `try` would not compile:\n{rendered}"
    );
}

/// The unregister/clear shape: the zig binding returns a genuine Zig error union
/// (`returns_result` unset — defaults to `true`), matching
/// `pub fn unregister_ocr_backend(...) SomeDomainError!void` in a real generated
/// binding. This must keep using `try` and must NOT be routed through the new
/// register-fn out_error check, even though this fixture also carries a
/// `test_backend` arg.
#[test]
fn error_union_style_call_keeps_using_try() {
    let fixture = register_fixture();
    let (e2e, config, type_defs) = base_e2e_config(
        "clear_test_backend",
        CallOverride {
            function: Some("clear_test_backend".to_string()),
            ..Default::default()
        },
    );

    let rendered = render_test_file(
        "plugin_api",
        &[&fixture],
        &e2e,
        "clear_test_backend",
        "result",
        &[],
        "sample",
        "sample",
        &config,
        &type_defs,
        &[],
        crate::e2e::codegen::call_ir::CallIr::default(),
        &[],
    );

    assert!(
        rendered.contains("_ = try sample.clear_test_backend("),
        "an error-union-returning call must still use `try`, got:\n{rendered}"
    );
    assert!(
        !rendered.contains("const _rc ="),
        "the out_error return-code check must not apply to an error-union call, got:\n{rendered}"
    );
}
