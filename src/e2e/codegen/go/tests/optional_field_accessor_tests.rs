//! Coverage for optional-field accessor rendering: a plain `Option<String>` field must
//! deref with `string(*field_expr)`, while a `display_as_text` field (e.g.
//! `Option<AssistantContent>`) must instead call its `.Text()` accessor.
//!
//! Split out of `tests.rs`, which is over the 1000-line cap and may not grow.

use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::fixture::{Assertion, Fixture};

use super::test_function::{GoTestFunctionContext, render_test_function};

/// A plain `Option<String>` optional field should still emit `string(*field_expr)`.
/// This guards against regressions where the display_as_text path is taken for
/// normal optional string fields.
#[test]
fn test_go_plain_optional_string_uses_string_deref_not_text_accessor() {
    let mut optional = std::collections::HashSet::new();
    optional.insert("content".to_string());
    let e2e_config = E2eConfig {
        call: CallConfig {
            function: "chat".to_string(),
            module: "github.com/example/mylib".to_string(),
            result_var: "result".to_string(),
            returns_result: true,
            ..CallConfig::default()
        },
        fields_optional: optional,
        // fields_display_as_text is intentionally empty — plain string field.
        ..E2eConfig::default()
    };
    let fixture = Fixture {
        docs: None,
        requirements: Vec::new(),
        id: "plain_optional_string".to_string(),
        category: None,
        description: "plain optional string field".to_string(),
        tags: vec![],
        skip: None,
        env: None,
        setup: Vec::new(),
        call: None,
        input: serde_json::Value::Null,
        mock_response: Some(crate::e2e::fixture::MockResponse {
            status: 200,
            body: Some(serde_json::Value::Null),
            stream_chunks: None,
            headers: std::collections::BTreeMap::new(),
            delay_ms: None,
            stream_chunk_delay_ms: None,
        }),
        source: String::new(),
        http: None,
        asyncapi: None,
        websocket: None,
        client_options: None,
        preserve_input_urls: false,
        visitor: None,
        args: vec![],
        assertion_recipes: vec![],
        assertions: vec![Assertion {
            assertion_type: "equals".to_string(),
            field: Some("content".to_string()),
            value: Some(serde_json::Value::String("hello".to_string())),
            ..Default::default()
        }],
    };

    let mut out = String::new();
    let config = crate::core::config::ResolvedCrateConfig::default();
    let type_defs: Vec<crate::core::ir::TypeDef> = Vec::new();
    let enums: Vec<crate::core::ir::EnumDef> = Vec::new();
    render_test_function(
        &mut out,
        &fixture,
        GoTestFunctionContext {
            import_alias: "pkg",
            e2e_config: &e2e_config,
            adapters: &[],
            data_enum_names: &std::collections::HashSet::new(),
            config: &config,
            type_defs: &type_defs,
            enums: &enums,
            errors: &[],
            functions: &[],
        },
    );
    // Plain optional string: must use `string(*field_expr)`, NOT `.Text()`.
    assert!(
        out.contains("string(*"),
        "plain optional string must use string(*); got:\n{out}"
    );
    assert!(
        !out.contains(".Text()"),
        "plain optional string must NOT use .Text(); got:\n{out}"
    );
}

/// A `display_as_text` field (e.g. `Option<AssistantContent>`) should emit
/// `field_expr.Text()` instead of `string(*field_expr)` for Go optional locals.
#[test]
fn test_go_display_as_text_optional_uses_text_accessor_not_string_deref() {
    let mut optional = std::collections::HashSet::new();
    optional.insert("content".to_string());
    let mut dat = std::collections::HashSet::new();
    dat.insert("content".to_string());
    let e2e_config = E2eConfig {
        call: CallConfig {
            function: "chat".to_string(),
            module: "github.com/example/mylib".to_string(),
            result_var: "result".to_string(),
            returns_result: true,
            ..CallConfig::default()
        },
        fields_optional: optional,
        fields_display_as_text: dat,
        ..E2eConfig::default()
    };
    let fixture = Fixture {
        docs: None,
        requirements: Vec::new(),
        id: "display_as_text_content".to_string(),
        category: None,
        description: "display_as_text optional field".to_string(),
        tags: vec![],
        skip: None,
        env: None,
        setup: Vec::new(),
        call: None,
        input: serde_json::Value::Null,
        mock_response: Some(crate::e2e::fixture::MockResponse {
            status: 200,
            body: Some(serde_json::Value::Null),
            stream_chunks: None,
            headers: std::collections::BTreeMap::new(),
            delay_ms: None,
            stream_chunk_delay_ms: None,
        }),
        source: String::new(),
        http: None,
        asyncapi: None,
        websocket: None,
        client_options: None,
        preserve_input_urls: false,
        visitor: None,
        args: vec![],
        assertion_recipes: vec![],
        assertions: vec![Assertion {
            assertion_type: "equals".to_string(),
            field: Some("content".to_string()),
            value: Some(serde_json::Value::String("Hello, world!".to_string())),
            ..Default::default()
        }],
    };

    let mut out = String::new();
    let config = crate::core::config::ResolvedCrateConfig::default();
    let type_defs: Vec<crate::core::ir::TypeDef> = Vec::new();
    let enums: Vec<crate::core::ir::EnumDef> = Vec::new();
    render_test_function(
        &mut out,
        &fixture,
        GoTestFunctionContext {
            import_alias: "pkg",
            e2e_config: &e2e_config,
            adapters: &[],
            data_enum_names: &std::collections::HashSet::new(),
            config: &config,
            type_defs: &type_defs,
            enums: &enums,
            errors: &[],
            functions: &[],
        },
    );
    // display_as_text field: must use `.Text()`, NOT `string(*field_expr)`.
    assert!(
        out.contains(".Text()"),
        "display_as_text field must use .Text(); got:\n{out}"
    );
    assert!(
        !out.contains("string(*"),
        "display_as_text field must NOT use string(*); got:\n{out}"
    );
}
