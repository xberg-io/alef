//! Regression coverage for a void `not_error` fixture whose call config also declares
//! `returns_result: true`: the unused `{:ok, result}` binding must be underscore-prefixed
//! rather than asserted `refute is_nil`, since rustler encodes a Rust `()` success payload as
//! `nil`. This covers only that one shape — `returns_result: true` — where a real `{:ok, _}`/
//! `{:error, _}` match happens. A `returns_void` fixture with `returns_result: false` (a
//! bare-atom fallible NIF, no tuple to match on) is a DIFFERENT shape with no such match to
//! rely on; see `bare_atom_not_error_tests` for that sibling.
//!
//! Split out of `test_case.rs`, which is over the 1000-line cap and may not grow.

use super::render_test_case;
use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::fixture::{Assertion, Fixture};
use std::collections::{HashMap, HashSet};

/// Regression test for the void `not_error` defect: before this fix, a `returns_void`,
/// `returns_result: true` fixture whose only assertion was `not_error` still bound
/// `{:ok, result} = call(...)` and then asserted `refute is_nil(result)` — but rustler encodes
/// a Rust `()` success payload as `nil`, so that assertion FAILED every successful call, not
/// just an unsuccessful one. The `{:ok, result} = call(...)` match itself is already the real
/// check for this shape (an `{:error, _}` return raises `MatchError`), so the fix
/// underscore-prefixes the unused binding and emits no `refute is_nil` line.
#[test]
fn void_not_error_fixture_binds_underscored_and_emits_no_failing_assertion() {
    let fixture = Fixture {
        docs: None,
        requirements: Vec::new(),
        id: "prefetch_languages".to_string(),
        category: None,
        description: "test".to_string(),
        tags: vec![],
        skip: None,
        env: None,
        setup: Vec::new(),
        call: None,
        input: serde_json::Value::Null,
        mock_response: None,
        source: String::new(),
        http: None,
        asyncapi: None,
        websocket: None,
        preserve_input_urls: false,
        assertions: vec![Assertion {
            assertion_type: "not_error".to_string(),
            ..Default::default()
        }],
        visitor: None,
        args: vec![],
        assertion_recipes: vec![],
    };
    let call = CallConfig {
        function: "prefetch_languages".to_string(),
        module: "MyLib".to_string(),
        result_var: "result".to_string(),
        returns_result: true,
        returns_void: true,
        ..Default::default()
    };
    let e2e_config = E2eConfig {
        call,
        ..Default::default()
    };
    let config = crate::core::config::ResolvedCrateConfig::default();
    let type_defs: Vec<crate::core::ir::TypeDef> = Vec::new();

    let mut out = String::new();
    render_test_case(
        &mut out,
        &fixture,
        &e2e_config,
        "",
        "",
        "",
        &[],
        None,
        None,
        &HashMap::new(),
        None,
        &HashSet::new(),
        &[],
        &[],
        &config,
        &type_defs,
        &[],
        &[],
    );

    assert!(
        !out.contains("refute is_nil"),
        "a void call's result is always nil; asserting non-nil would fail every successful \
         call, got:\n{out}"
    );
    assert!(
        out.contains("{:ok, _result} ="),
        "the unused binding must be underscore-prefixed to avoid an unused-variable warning, \
         got:\n{out}"
    );
}
