//! Verifies that Zig e2e codegen properly wraps C string pointers ([*c]const u8)
//! with std.mem.span() before passing them to Zig functions that expect
//! slices ([]const u8), such as std.json.parseFromSlice and std.fmt.allocPrint.
//!
//! This fixes the error: "Unable to stringify type '[*c]const u8'"
//! which occurs when trying to use raw C pointers with Zig's format/stringify functions.

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::zig::ZigE2eCodegen;
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup};

fn make_fixture_with_assertions(id: &str, assertions: Vec<Assertion>) -> Fixture {
    Fixture {
        id: id.to_string(),
        category: Some("smoke".to_string()),
        description: "test fixture".to_string(),
        tags: Vec::new(),
        skip: None,
        env: None,
        setup: Vec::new(),
        call: None,
        input: serde_json::json!({ "request": { "model": "gpt-4o", "messages": [] } }),
        mock_response: Some(alef::e2e::fixture::MockResponse {
            status: 200,
            body: Some(serde_json::Value::Null),
            stream_chunks: None,
            headers: std::collections::BTreeMap::new(),
        }),
        visitor: None,
        args: Vec::new(),
        assertion_recipes: Vec::new(),
        assertions,
        source: "smoke.json".to_string(),
        http: None,
    }
}

fn make_group(_id: &str, fixture: Fixture) -> FixtureGroup {
    FixtureGroup {
        category: "smoke".to_string(),
        fixtures: vec![fixture],
    }
}

fn render_zig_test(toml: &str, fixture_id: &str, fixture: Fixture) -> String {
    let cfg: NewAlefConfig = toml::from_str(toml).expect("config parses");
    let resolved = cfg.clone().resolve().expect("config resolves").remove(0);
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    let groups = vec![make_group(fixture_id, fixture)];
    let files = ZigE2eCodegen
        .generate(&groups, &e2e, &resolved, &[], &[])
        .expect("generation succeeds");
    files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("smoke_test.zig"))
        .expect("smoke_test.zig is emitted")
        .content
        .clone()
}

const BASE_TOML: &str = r#"
[workspace]
languages = ["ffi", "zig"]

[[crates]]
name = "demo-client"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "samplellm"

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "chat"
module = "demo_client"
result_var = "result"

[[crates.e2e.call.args]]
name = "request"
field = "input.request"
type = "json_object"
"#;

/// Verify that C string pointers are wrapped with std.mem.span() in parseFromSlice calls.
#[test]
fn c_string_pointer_wrapped_in_parse_from_slice() {
    let toml = format!(
        r#"{}
[crates.e2e.call.overrides.zig]
result_is_json_struct = true
"#,
        BASE_TOML
    );

    let fixture = make_fixture_with_assertions(
        "parse_span_test",
        vec![Assertion {
            assertion_type: "contains".to_string(),
            field: Some("id".to_string()),
            value: Some(serde_json::Value::String("test".to_string())),
            values: None,
            method: None,
            check: None,
            args: None,
            return_type: None,
        }],
    );

    let rendered = render_zig_test(&toml, "parse_span_test", fixture);

    // The generated code should convert _result_json (a [*c]const u8) using std.mem.span
    // before passing to parseFromSlice (which expects []const u8).
    assert!(
        rendered.contains("std.mem.span(_result_json)"),
        "must wrap C string pointer with std.mem.span in parseFromSlice. Rendered:\n{rendered}"
    );
}

/// Verify that C string pointers are wrapped with std.mem.span() in std.fmt.allocPrint calls.
#[test]
fn c_string_pointer_wrapped_in_format_string() {
    let toml = format!(
        r#"{}
[crates.e2e.call.overrides.zig]
function = "interact"
result_is_json_struct = true
"#,
        BASE_TOML
    );

    let fixture = make_fixture_with_assertions(
        "interact_format_span_test",
        vec![Assertion {
            assertion_type: "contains".to_string(),
            field: Some("interaction.action_results".to_string()),
            value: Some(serde_json::Value::String("success".to_string())),
            values: None,
            method: None,
            check: None,
            args: None,
            return_type: None,
        }],
    );

    let rendered = render_zig_test(&toml, "interact_format_span_test", fixture);

    // For interact(), the result gets wrapped in JSON format string with {s} specifier.
    // The {s} format requires a []const u8 slice, so _result_json must be wrapped.
    assert!(
        rendered.contains("std.mem.span(_result_json)"),
        "must wrap C string pointer with std.mem.span in format string. Rendered:\n{rendered}"
    );
    assert!(
        rendered.contains("allocPrint"),
        "interact path should use allocPrint to build wrapped JSON. Rendered:\n{rendered}"
    );
}

/// Verify that C string pointers in normal (non-JSON) results are wrapped with std.mem.span()
/// when the result is a JSON struct but the wrap_field is None.
#[test]
fn c_string_pointer_wrapped_in_parse_without_wrap_field() {
    let toml = format!(
        r#"{}
[crates.e2e.call.overrides.zig]
function = "extract"
result_is_json_struct = true
"#,
        BASE_TOML
    );

    let fixture = make_fixture_with_assertions(
        "extract_span_test",
        vec![Assertion {
            assertion_type: "contains".to_string(),
            field: Some("text".to_string()),
            value: Some(serde_json::Value::String("hello".to_string())),
            values: None,
            method: None,
            check: None,
            args: None,
            return_type: None,
        }],
    );

    let rendered = render_zig_test(&toml, "extract_span_test", fixture);

    // When result is JSON struct but no wrap_field, still need to convert _result_json pointer.
    // This tests the else clause in the parse_json_var if statement (line 587 in test_file.rs)
    assert!(
        rendered.contains("std.mem.span(_result_json)"),
        "must wrap C string pointer with std.mem.span in JSON parse when no wrap_field. Rendered:\n{rendered}"
    );
}
