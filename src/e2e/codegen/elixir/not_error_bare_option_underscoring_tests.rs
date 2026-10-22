//! ~keep Regression coverage for `render_test_case`'s `actual_result_var` underscore-prefixing
//! interaction with the `not_error_presence::may_assert_presence` unification. Lives in its own
//! file rather than growing `test_case.rs`: that file is already over the repo's 1,000-line cap
//! (see `file-modularization` in CLAUDE.md).
//!
//! Before the unification, Elixir only suppressed `not_error`'s `refute is_nil(...)` when a
//! *sibling* assertion existed. A fixture whose *sole* assertion was `not_error` on an
//! `Option<T>`-returning call (`result_is_option: true`) still got an unconditional
//! `refute is_nil(result)`, which fails every time the call's success path legitimately returns
//! `None` (rustler's `nil`). Closing that gap makes `render_assertion`'s `not_error` arm render
//! nothing for this shape (mirroring the pre-existing `returns_void` case), which in turn means
//! `actual_result_var` must underscore-prefix the `{:ok, result} = call(...)` binding — otherwise
//! `mix compile --warnings-as-errors` fails on an unused variable downstream. This test exercises
//! the real `render_test_case` top-level generator, not a hand-written mirror of it.

use super::test_case::render_test_case;
use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::fixture::{Assertion, Fixture};
use std::collections::{HashMap, HashSet};

#[test]
fn not_error_on_a_bare_option_result_binds_underscored_and_emits_no_assertion() {
    let fixture = Fixture {
        docs: None,
        requirements: Vec::new(),
        id: "detect_language".to_string(),
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
        client_options: None,
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
        function: "detect_language".to_string(),
        module: "MyLib".to_string(),
        result_var: "result".to_string(),
        returns_result: true,
        returns_void: false,
        result_is_option: true,
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
        "a bare Option<T> result may legitimately be nil on success; not_error must not \
         assert non-nil, got:\n{out}"
    );
    assert!(
        out.contains("{:ok, _result} ="),
        "the unused binding must be underscore-prefixed to avoid an unused-variable warning, \
         got:\n{out}"
    );
}
