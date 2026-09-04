//! Regression coverage for the bare-atom `not_error` defect: a `returns_void` fixture whose
//! call config also declares `returns_result: false` — a fallible NIF with no `Result` wrapper
//! for rustler to auto-tuple, so it encodes success/failure as the bare atoms `:ok`/`:error`
//! directly (rustler convention: `Ok(_) => atom("ok")`, `Err(_) => atom("error")`) — has no
//! `{:ok, _}`/`{:error, _}` tuple to match on. Before this fix, the generator treated this
//! exactly like the `returns_result: true` shape covered in `void_not_error_binding_tests`: it
//! underscore-prefixed the binding and emitted no assertion at all, on the (false, for this
//! shape) premise that a match above already raised on failure. The emitted test could never
//! fail: a real NIF returning the atom `:error` on every call would still pass, because nothing
//! ever inspected the bound value.
//!
//! Also covers the sibling non-void case (mirrors the generator's `list_*`-style plugin
//! registry calls): a bare, non-void, `returns_result: false` call whose only assertion is
//! `not_error` used to emit `refute is_nil(result)`, which is equally vacuous against a bare
//! `:error` atom (`:error` is not `nil`).
//!
//! Split out of `test_case.rs`, which is over the 1000-line cap and may not grow.

use super::render_test_case;
use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::fixture::{Assertion, Fixture};
use std::collections::{HashMap, HashSet};

fn not_error_fixture(id: &str) -> Fixture {
    Fixture {
        docs: None,
        requirements: Vec::new(),
        id: id.to_string(),
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
    }
}

fn render(fixture: &Fixture, call: CallConfig) -> String {
    let e2e_config = E2eConfig {
        call,
        ..Default::default()
    };
    let config = crate::core::config::ResolvedCrateConfig::default();
    let type_defs: Vec<crate::core::ir::TypeDef> = Vec::new();

    let mut out = String::new();
    render_test_case(
        &mut out,
        fixture,
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
    out
}

/// The bare-atom void defect itself: `returns_void: true` with `returns_result: false` (the
/// default) must bind the result WITHOUT underscoring it, and assert `== :ok` directly, since
/// there is no `{:ok, _}` match to raise a `MatchError` on an `:error` return.
#[test]
fn bare_atom_void_not_error_fixture_binds_and_asserts_ok() {
    let fixture = not_error_fixture("clear_validators");
    let call = CallConfig {
        function: "clear_validators".to_string(),
        module: "MyLib".to_string(),
        result_var: "result".to_string(),
        returns_void: true,
        ..Default::default()
    };
    let out = render(&fixture, call);

    assert!(
        out.contains("result = MyLib.clear_validators()"),
        "a bare-atom fallible call has no tuple to match, so the binding must not be \
         underscore-prefixed (it is referenced by the assertion below), got:\n{out}"
    );
    assert!(
        out.contains("assert result == :ok"),
        "a bare-atom void call must assert directly on the returned atom since no \
         `{{:ok, _}}` match exists to fail on `:error`, got:\n{out}"
    );
    assert!(
        !out.contains("{:ok,"),
        "a `returns_result: false` call must not be rendered as a tuple match, got:\n{out}"
    );
}

/// The `list_*`-style sibling: a bare, non-void, `returns_result: false` call whose only
/// assertion is `not_error` must refute both `nil` and the bare `:error` sentinel, not just
/// `nil` — `refute is_nil(result)` alone passes trivially against an `:error` atom return.
#[test]
fn bare_atom_non_void_not_error_fixture_refutes_nil_and_error() {
    let fixture = not_error_fixture("list_validators");
    let call = CallConfig {
        function: "list_validators".to_string(),
        module: "MyLib".to_string(),
        result_var: "result".to_string(),
        ..Default::default()
    };
    let out = render(&fixture, call);

    assert!(
        out.contains("refute result in [nil, :error]"),
        "a bare, returns_result: false call must refute the `:error` sentinel alongside nil, \
         got:\n{out}"
    );
    assert!(
        !out.contains("refute is_nil(result)"),
        "the plain nil-only check passes vacuously against a bare `:error` atom return, \
         got:\n{out}"
    );
}
