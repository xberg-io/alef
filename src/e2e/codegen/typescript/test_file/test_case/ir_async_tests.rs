use super::*;
use crate::e2e::config::CallConfig;
use crate::e2e::fixture::Assertion;

/// Regression coverage: `alef.toml`'s `[call] async` is a plain `bool` defaulting to
/// `false`, so a fixture whose config never set it is indistinguishable from one that
/// explicitly opted out. A Rust function that became `async fn` after the fixture was
/// authored left the config stale, and the generated test called it without `await` —
/// a `Promise` object compared against the expected value instead of its resolved
/// value. The core IR's own `is_async` (populated regardless of what `alef.toml` says)
/// must be consulted when the config does not override the answer.
#[test]
fn ir_async_function_gets_await_even_when_config_omits_async() {
    let fixture = Fixture {
        docs: None,
        requirements: Vec::new(),
        id: "detect_widget_smoke".to_string(),
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
            ..Assertion::default()
        }],
        visitor: None,
        args: vec![],
        assertion_recipes: vec![],
    };
    let call = CallConfig {
        function: "detect_widget".to_string(),
        module: "myLib".to_string(),
        result_var: "result".to_string(),
        returns_result: true,
        // Deliberately NOT setting `r#async: true` -- the IR alone must supply it.
        ..Default::default()
    };
    let e2e_config = E2eConfig {
        call,
        ..Default::default()
    };
    let config = crate::core::config::ResolvedCrateConfig::default();
    let functions = vec![crate::core::ir::FunctionDef {
        name: "detect_widget".to_string(),
        is_async: true,
        ..Default::default()
    }];
    let type_defs: Vec<TypeDef> = Vec::new();
    let enums: Vec<EnumDef> = Vec::new();
    let errors: Vec<crate::core::ir::ErrorDef> = Vec::new();
    let mut referenced_enums = std::collections::BTreeSet::new();

    let mut out = String::new();
    render_test_case(
        &mut out,
        &fixture,
        None,
        None,
        &e2e_config,
        "node",
        &std::collections::HashMap::new(),
        &std::collections::HashMap::new(),
        &std::collections::HashMap::new(),
        &type_defs,
        &enums,
        &functions,
        "",
        &config,
        &mut referenced_enums,
        &errors,
    );

    assert!(
        out.contains("await detectWidget("),
        "the IR declares detect_widget async; the call must be awaited, got:\n{out}"
    );
}

/// A per-language `async` override is trusted verbatim, even against a disagreeing IR:
/// this is the escape hatch for a call the IR seam does not (or should not) resolve.
#[test]
fn explicit_async_override_wins_over_a_sync_ir_signature() {
    let fixture = Fixture {
        docs: None,
        requirements: Vec::new(),
        id: "detect_widget_smoke".to_string(),
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
            ..Assertion::default()
        }],
        visitor: None,
        args: vec![],
        assertion_recipes: vec![],
    };
    let mut overrides = std::collections::HashMap::new();
    overrides.insert(
        "node".to_string(),
        crate::core::config::e2e::CallOverride {
            r#async: Some(true),
            ..Default::default()
        },
    );
    let call = CallConfig {
        function: "detect_widget".to_string(),
        module: "myLib".to_string(),
        result_var: "result".to_string(),
        returns_result: true,
        overrides,
        ..Default::default()
    };
    let e2e_config = E2eConfig {
        call,
        ..Default::default()
    };
    let config = crate::core::config::ResolvedCrateConfig::default();
    // IR says sync; the override must still win.
    let functions = vec![crate::core::ir::FunctionDef {
        name: "detect_widget".to_string(),
        is_async: false,
        ..Default::default()
    }];
    let type_defs: Vec<TypeDef> = Vec::new();
    let enums: Vec<EnumDef> = Vec::new();
    let errors: Vec<crate::core::ir::ErrorDef> = Vec::new();
    let mut referenced_enums = std::collections::BTreeSet::new();

    let mut out = String::new();
    render_test_case(
        &mut out,
        &fixture,
        None,
        None,
        &e2e_config,
        "node",
        &std::collections::HashMap::new(),
        &std::collections::HashMap::new(),
        &std::collections::HashMap::new(),
        &type_defs,
        &enums,
        &functions,
        "",
        &config,
        &mut referenced_enums,
        &errors,
    );

    assert!(
        out.contains("await detectWidget("),
        "an explicit per-language async override must win over a sync IR signature, got:\n{out}"
    );
}
