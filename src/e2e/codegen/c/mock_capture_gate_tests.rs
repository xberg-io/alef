//! End-to-end coverage for the `mock.*` request-count gate (alef issue #443).
//!
//! The call site (`assertion_mock_capture::try_render_mock_capture_assertion`) and the
//! helper-definition gate (`assertion_mock_capture::append_helper_if_referenced`) are tested
//! SEPARATELY everywhere else in this codebase -- these tests drive the real `CCodegen::generate`
//! entry point so a fixture's call, the file's helper definition, AND the extra `#include`s the
//! helper needs all land in ONE rendered artifact. Mirrors `java::mock_capture_gate_tests` /
//! `csharp::mock_capture_gate_tests`. ~keep

use super::*;
use crate::core::ir::{FunctionDef, MethodDef, TypeDef, TypeRef};
use crate::e2e::fixture::{Assertion, Fixture, FixtureGroup};

fn mock_assertion(field: &str, assertion_type: &str, value: serde_json::Value) -> Assertion {
    Assertion {
        assertion_type: assertion_type.to_string(),
        field: Some(field.to_string()),
        value: Some(value),
        ..Default::default()
    }
}

fn render(assertions: Vec<Assertion>) -> String {
    let group = FixtureGroup {
        category: "mock_capture".into(),
        fixtures: vec![Fixture {
            id: "mock_capture_fixture".into(),
            description: "asserts against the mock server request log".into(),
            assertions,
            ..Fixture::default()
        }],
    };
    let mut e2e = E2eConfig::default();
    e2e.call.function = "get_thing".into();
    let functions = vec![FunctionDef {
        name: "get_thing".into(),
        return_type: TypeRef::Named("Thing".into()),
        error_type: Some("String".into()),
        ..FunctionDef::default()
    }];
    let config = ResolvedCrateConfig {
        name: "sample".into(),
        ..ResolvedCrateConfig::default()
    };

    let generated = CCodegen
        .generate(&[group], &e2e, &config, &[], &[], &functions, &[])
        .expect("C harness renders");
    generated
        .into_iter()
        .find(|file| file.path.ends_with("test_mock_capture.c"))
        .expect("category test is emitted")
        .content
}

/// Covers both `mock.requests.total` and the bracketed `mock.requests["POST /v1/chat"]` key form
/// on the same fixture -- cheap here since both share one rendered file. Also proves the extra
/// socket `#include`s land BEFORE the helper's call site (a real compiler would reject them
/// after), which is the part unique to this backend among every wired language so far. ~keep
#[test]
fn a_mock_assertion_gets_its_call_the_includes_and_the_helper_definition_in_one_file() {
    let out = render(vec![
        mock_assertion("mock.requests.total", "equals", serde_json::json!(1)),
        mock_assertion(
            r#"mock.requests["POST /v1/chat"]"#,
            "greater_than_or_equal",
            serde_json::json!(1),
        ),
    ]);

    assert!(
        out.contains(
            "assert(alef_mock_request_count(getenv(\"MOCK_SERVER_URL\"), \"/__alef/requests/total?prefix=\") == 1 && \"equals assertion failed\");"
        ),
        "expected the total-count call, got:\n{out}"
    );
    assert!(
        out.contains(
            "assert(alef_mock_request_count(getenv(\"MOCK_SERVER_URL\"), \"/__alef/requests/one?key=POST%20%2Fv1%2Fchat\") >= 1 && \"expected greater than or equal\");"
        ),
        "expected the bracketed-key call, got:\n{out}"
    );
    assert!(
        out.contains("static int alef_mock_request_count(const char *base_url, const char *path_and_query)"),
        "expected the helper definition in the same file as the call, got:\n{out}"
    );
    assert!(
        out.contains("#include <sys/socket.h>")
            && out.contains("#include <netdb.h>")
            && out.contains("#include <unistd.h>"),
        "expected the socket includes the helper needs, got:\n{out}"
    );

    let includes_pos = out.find("#include <sys/socket.h>").unwrap();
    let call_pos = out.find("alef_mock_request_count(getenv").unwrap();
    let helper_def_pos = out.find("static int alef_mock_request_count").unwrap();
    assert!(
        includes_pos < call_pos,
        "the socket #include must precede the first call site, got:\n{out}"
    );
    assert!(
        includes_pos < helper_def_pos,
        "the socket #include must precede the helper definition, got:\n{out}"
    );
}

/// Covers the OTHER field-extraction loop in `test_function.rs` (the client-factory path, used
/// when `[e2e.call.overrides.c].client_factory` is configured) -- a structurally near-identical
/// duplicate of the free-function path `render` above exercises, with its own independent
/// `is_mock_virtual_field` guard added in the same fix. Without that guard this fixture would
/// have `bail!`ed out of `CCodegen::generate` entirely via `ensure_leaf_field_exists`/
/// `emit_nested_accessor`'s own diagnostics, well before ever reaching a rendered assertion. ~keep
#[test]
fn a_mock_assertion_survives_the_client_factory_extraction_loop_too() {
    let group = FixtureGroup {
        category: "mock_capture_client".into(),
        fixtures: vec![Fixture {
            id: "mock_capture_client_fixture".into(),
            description: "asserts against the mock server request log via a client method".into(),
            assertions: vec![mock_assertion("mock.requests.total", "equals", serde_json::json!(1))],
            ..Fixture::default()
        }],
    };
    let mut e2e = E2eConfig::default();
    e2e.call.function = "get_thing".into();
    e2e.call.overrides.insert(
        "c".into(),
        crate::e2e::config::CallOverride {
            client_factory: Some("create_client".into()),
            ..Default::default()
        },
    );
    // A method on an opaque type, not a free `FunctionDef`: `resolve_c_client_owner_type`
    // resolves the client-factory pattern's owner type either from `[[crates.adapters]]` or by
    // finding a type whose methods declare `function_name` -- a free function satisfies neither,
    // and generation would otherwise skip with "could not resolve the client owner type" before
    // ever reaching the assertion-rendering code this test exists to exercise.
    let type_defs = vec![TypeDef {
        name: "Client".into(),
        is_opaque: true,
        methods: vec![MethodDef {
            name: "get_thing".into(),
            return_type: TypeRef::Named("Thing".into()),
            error_type: Some("String".into()),
            ..MethodDef::default()
        }],
        ..TypeDef::default()
    }];
    let config = ResolvedCrateConfig {
        name: "sample".into(),
        ..ResolvedCrateConfig::default()
    };

    let generated = CCodegen
        .generate(&[group], &e2e, &config, &type_defs, &[], &[], &[])
        .expect("C harness renders");
    let out = generated
        .into_iter()
        .find(|file| file.path.ends_with("test_mock_capture_client.c"))
        .expect("category test is emitted")
        .content;

    assert!(
        out.contains(
            "assert(alef_mock_request_count(getenv(\"MOCK_SERVER_URL\"), \"/__alef/requests/total?prefix=\") == 1 && \"equals assertion failed\");"
        ),
        "expected the mock-count call via the client-factory path, got:\n{out}"
    );
    assert!(
        !out.contains("_mock("),
        "must not have emitted a bogus accessor call for the 'mock' segment, got:\n{out}"
    );
}

/// Negative direction: an always-emit bug in the helper-definition gate would pass the positive
/// test above, so a fixture with NO `mock.*` assertion must not reference, define, or pull in the
/// extra includes for the helper.
#[test]
fn a_file_with_no_mock_assertion_defines_no_helper_and_adds_no_socket_includes() {
    let out = render(vec![Assertion {
        assertion_type: "not_error".to_string(),
        ..Default::default()
    }]);
    assert!(
        !out.contains("alef_mock_request_count"),
        "a file with no mock.* assertion must not reference or define the helper, got:\n{out}"
    );
    assert!(
        !out.contains("sys/socket.h") && !out.contains("<netdb.h>") && !out.contains("<unistd.h>"),
        "a file with no mock.* assertion must not gain the helper's socket includes, got:\n{out}"
    );
}
