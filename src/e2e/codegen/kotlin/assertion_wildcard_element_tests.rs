//! Regression coverage for the element accessor a wildcard (`container[].field`) fixture path
//! expands to in the Kotlin e2e generator.
//!
//! ~keep Kotlin was reported CLEAN by a survey of one consumer corpus's generated output. It is
//! not structurally clean: its wildcard branch routed the element half through the same
//! result-anchored `FieldResolver::accessor` the six confirmed-defective backends used, against a
//! resolver its `call_field_resolver` anchors at the call's declared result type exactly as they
//! do. That corpus simply produced no `result_fields` envelope rescue for this backend. The
//! fixture below supplies one, and the assertions fail against the pre-fix generator — an
//! accident, not an immunity.
//!
//! Kotlin's renderer lowers index 0 to `.first()` rather than a visible `[0]`, so the pre-fix
//! output was `e.records().first().kind()`: the container path applied a second time against a
//! binding that is already an element, and harder to spot in review than the `[0]` spelling.

use std::collections::{HashMap, HashSet};

use super::assertions::{RenderAssertionContext, render_assertion};
use crate::e2e::codegen::wildcard_element_fixture::{
    WILDCARD_FIELD, assert_container_accessor_appears_once, assert_element_relative, contains_assertion,
    envelope_resolver, report_resolver,
};
use crate::e2e::field_access::FieldResolver;

const CONTAINER_ACCESSOR: &str = ".records()";

fn render(resolver: &FieldResolver) -> String {
    let mut out = String::new();
    render_assertion(
        &mut out,
        &contains_assertion(WILDCARD_FIELD),
        RenderAssertionContext {
            result_var: "result",
            class_name: "SampleTest",
            field_resolver: resolver,
            result_is_simple: false,
            result_is_option: false,
            enum_fields: &HashSet::new(),
            json_scalar_fields: &HashSet::new(),
            fields_c_types: &HashMap::new(),
            is_streaming: false,
            kotlin_android_style: false,
            not_error_may_assert_presence: false,
        },
    );
    out
}

#[test]
fn wildcard_lambda_body_is_relative_to_the_element_binding() {
    let rendered = render(&report_resolver("kotlin"));

    assert_element_relative(&rendered, "e.kind().toString()", "e.records().first()");
    assert_container_accessor_appears_once(&rendered, CONTAINER_ACCESSOR);
}

/// The container half must keep resolving against the result variable — the fix must not turn
/// into "never anchor", which would still satisfy the assertion above.
#[test]
fn wildcard_container_stays_anchored_to_the_result_variable() {
    let rendered = render(&report_resolver("kotlin"));

    assert!(
        rendered.contains("result.records().any { e ->"),
        "container half must quantify over the result variable's own field, got: {rendered}"
    );
}

/// The stronger container control: on an envelope root the container is reachable only THROUGH
/// the `result_fields` projection, so dropping the anchoring from `accessor` — rather than only
/// from the element half — renders `result.records()`, a member the envelope does not declare.
#[test]
fn wildcard_container_keeps_its_envelope_projection() {
    let rendered = render(&envelope_resolver("kotlin"));

    assert!(
        rendered.contains("result.results().first().records().any { e ->"),
        "container half must keep the result-anchored envelope projection, got: {rendered}"
    );
    assert_container_accessor_appears_once(&rendered, CONTAINER_ACCESSOR);
}
