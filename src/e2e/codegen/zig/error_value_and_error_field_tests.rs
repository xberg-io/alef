use super::*;

fn assertion(assertion_type: &str, field: Option<&str>, value: Option<&str>) -> crate::e2e::fixture::Assertion {
    crate::e2e::fixture::Assertion {
        assertion_type: assertion_type.into(),
        field: field.map(str::to_string),
        value: value.map(|v| serde_json::Value::String(v.to_string())),
        ..Default::default()
    }
}

fn render(assertions: Vec<crate::e2e::fixture::Assertion>) -> String {
    render_with_errors(assertions, &[])
}

fn render_with_errors(assertions: Vec<crate::e2e::fixture::Assertion>, errors: &[crate::core::ir::ErrorDef]) -> String {
    let fixture = Fixture {
        id: "invalid_input".into(),
        description: "Rejects invalid input".into(),
        assertions,
        ..Fixture::default()
    };
    let mut e2e = E2eConfig::default();
    e2e.call.function = "parse".into();
    let _ = crate::e2e::codegen::take_skip_records();
    render_test_file(
        "error",
        &[&fixture],
        &e2e,
        "parse",
        "result",
        &[],
        "sample",
        "sample",
        &ResolvedCrateConfig::default(),
        &[],
        errors,
        crate::e2e::codegen::call_ir::CallIr::default(),
        &[],
    )
}

fn error_def_with(variant_name: &str, error_code: Option<u32>) -> Vec<crate::core::ir::ErrorDef> {
    vec![crate::core::ir::ErrorDef {
        exception_base: None,
        name: "ApiError".to_string(),
        rust_path: "lib::ApiError".to_string(),
        original_rust_path: String::new(),
        variants: vec![crate::core::ir::ErrorVariant {
            name: variant_name.to_string(),
            error_code,
            is_unit: true,
            ..crate::core::ir::ErrorVariant::default()
        }],
        doc: String::new(),
        methods: vec![],
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }]
}

/// The defect: a declared `error` value was discarded, leaving `} else |_| {}` — a check that
/// passes for ANY failure. The message and the error-set member name must both be compared.
#[test]
fn a_declared_error_value_is_compared_against_message_and_error_name() {
    let rendered = render(vec![assertion("error", None, Some("BadRequest"))]);

    assert!(
        rendered.contains("if (sample.parse()) |_| {"),
        "the error block itself must still render: {rendered}"
    );
    assert!(
        rendered.contains("const _err_message: []const u8 = sample._last_error() orelse \"\";"),
        "the FFI message must be bound: {rendered}"
    );
    assert!(
        rendered.contains("std.mem.indexOf(u8, _err_message, \"BadRequest\") != null"),
        "the declared value must be compared to the message: {rendered}"
    );
    assert!(
        rendered.contains("std.mem.indexOf(u8, _err_name, \"BadRequest\") != null"),
        "the declared value must also be compared to @errorName: {rendered}"
    );
    assert!(
        !rendered.contains("} else |_| {}"),
        "the value-discarding arm must be gone: {rendered}"
    );
}

/// Negative control for the arm above: with no declared value the output is byte-identical to
/// the pre-existing shape, so the change cannot have rewritten every error fixture.
#[test]
fn an_error_assertion_without_a_value_keeps_the_bare_arm() {
    let rendered = render(vec![assertion("error", None, None)]);

    assert!(rendered.contains("if (sample.parse()) |_| {"), "{rendered}");
    assert!(rendered.contains("} else |_| {}"), "{rendered}");
    assert!(!rendered.contains("_last_error()"), "{rendered}");
}

/// A CODED known variant still asserts, exactly as `a_declared_error_value_is_compared_
/// against_message_and_error_name` proves for the no-IR case above.
#[test]
fn a_coded_known_variant_still_asserts() {
    let rendered = render_with_errors(
        vec![assertion("error", None, Some("Authentication"))],
        &error_def_with("Authentication", Some(100)),
    );

    assert!(
        rendered.contains("std.mem.indexOf(u8, _err_name, \"Authentication\") != null"),
        "{rendered}"
    );
}

/// The defect this fix closes: a declared value naming a real `ErrorVariant` with no
/// `error_code` must render the registered skip inside the error arm, not an
/// `@errorName`/message comparison that can never pass.
#[test]
fn an_uncoded_known_variant_renders_the_skip() {
    let rendered = render_with_errors(
        vec![assertion("error", None, Some("Authentication"))],
        &error_def_with("Authentication", None),
    );

    assert!(
        rendered.contains(
            "// skipped: declared error variant 'Authentication' not substantiated by this backend's \
             generated error type"
        ),
        "{rendered}"
    );
    assert!(
        !rendered.contains("\"Authentication\""),
        "must not compare anything against the unsubstantiable variant name, got:\n{rendered}"
    );
}

/// The second half of the same arm: refusing the variant comparison is not a licence to discard
/// the captured error. `_ = _err;` left a block that ran on ANY failure and asserted nothing, so
/// a binding that lost the FFI message and collapsed the code to the catch-all member still went
/// green. The arm must bind both channels and require at least one of them to carry a signal.
#[test]
fn an_uncoded_known_variant_still_asserts_on_the_captured_error_and_its_message() {
    let rendered = render_with_errors(
        vec![assertion("error", None, Some("Authentication"))],
        &error_def_with("Authentication", None),
    );

    assert!(
        rendered.contains("} else |_err| {"),
        "the error must still be captured: {rendered}"
    );
    assert!(
        !rendered.contains("_ = _err;"),
        "the captured error must not be discarded: {rendered}"
    );
    assert!(
        rendered.contains("const _err_name = @errorName(_err);"),
        "the error-set member name must be bound: {rendered}"
    );
    assert!(
        rendered.contains("const _err_message: []const u8 = sample._last_error() orelse \"\";"),
        "the FFI message must be bound: {rendered}"
    );
    assert!(
        rendered
            .contains("try testing.expect(_err_message.len > 0 or !std.mem.eql(u8, _err_name, \"UnknownFfiError\"));"),
        "the arm must assert the failure carried a diagnosable identity: {rendered}"
    );
}

#[test]
fn an_equals_on_an_error_field_is_named_instead_of_dropped() {
    let rendered = render(vec![
        assertion("error", None, Some("BadRequest")),
        assertion("equals", Some("error.status_code"), None),
    ]);

    // Positive first: the fixture really did produce an error block.
    assert!(
        rendered.contains("return error.TestUnexpectedResult;"),
        "the error block must render before we assert anything about the second assertion: {rendered}"
    );
    assert!(
        rendered.contains(
            "// skipped: assertion type 'equals' has no accessor for error field error.status_code in this backend"
        ),
        "{rendered}"
    );

    let records = crate::e2e::codegen::take_skip_records();
    assert_eq!(records.len(), 1, "got: {records:?}");
    assert_eq!(records[0].language, "zig");
    assert_eq!(records[0].field, "equals");
}

/// The status assertion of an error fixture reads the status the wrapper captured with the
/// error, instead of being named as skipped.
#[test]
fn an_error_status_code_assertion_reads_the_captured_status() {
    use crate::e2e::codegen::error_field_checks::test_fixtures::{status_code_errors, status_code_fixture};

    let fixture = status_code_fixture(401);
    let mut e2e = E2eConfig::default();
    e2e.call.function = "parse".into();
    let _ = crate::e2e::codegen::take_skip_records();
    let rendered = render_test_file(
        "error",
        &[&fixture],
        &e2e,
        "parse",
        "result",
        &[],
        "sample",
        "sample",
        &ResolvedCrateConfig::default(),
        &[],
        &status_code_errors(),
        crate::e2e::codegen::call_ir::CallIr::default(),
        &[],
    );

    assert!(
        rendered.contains("try testing.expect(sample._last_error_details().status_code == 401);"),
        "the status must be compared with the fixture value: {rendered}"
    );
    assert!(
        !rendered.contains("no accessor for error field error.status_code"),
        "a rendered assertion must not also be named as skipped: {rendered}"
    );
    assert!(crate::e2e::codegen::take_skip_records().is_empty());
}
