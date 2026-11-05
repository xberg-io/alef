//! Assertions on a stream-scoped path (`usage.total_tokens`) for a streaming Dart fixture.
//!
//! A stream result is the `List` of chunks collected from the stream, so `usage` is not a member
//! of it. The shared streaming resolver reads the path off the last chunk. `Usage` is optional on
//! a chunk, so a stream without a usage payload yields `null` and fails the matcher.

use crate::e2e::codegen::streaming_assertions::{StreamingFieldResolver, is_stream_scoped_field};
use crate::e2e::fixture::Assertion;

use super::dart_format_value;

/// Render `assertion` when it names a stream-scoped path and has a value the matcher can take.
///
/// Returns `false` for any other assertion, so the caller's registered skip line still applies to
/// an assertion type this renderer does not implement.
pub(super) fn try_render(out: &mut String, assertion: &Assertion, result_var: &str) -> bool {
    let Some(field) = assertion.field.as_deref().filter(|f| is_stream_scoped_field(f)) else {
        return false;
    };
    let Some(value) = assertion.value.as_ref() else {
        return false;
    };
    let matcher = match (assertion.assertion_type.as_str(), value.is_number()) {
        ("equals", _) => "equals",
        ("greater_than", true) => "greaterThan",
        ("greater_than_or_equal", true) => "greaterThanOrEqualTo",
        ("less_than", true) => "lessThan",
        ("less_than_or_equal", true) => "lessThanOrEqualTo",
        _ => return false,
    };
    let Some(expr) = StreamingFieldResolver::accessor(field, "dart", result_var) else {
        return false;
    };
    render(out, &expr, matcher, &dart_format_value(value));
    true
}

/// Render `assertion` against `expression`, whose binding value has type `declared`: a number
/// compared against a `BigInt`-typed value is rendered as a `BigInt` literal.
pub(super) fn try_render_with_expression(
    out: &mut String,
    assertion: &Assertion,
    expression: &str,
    declared: &crate::core::ir::TypeRef,
) -> bool {
    let Some(value) = assertion.value.as_ref() else {
        return false;
    };
    let matcher = match (assertion.assertion_type.as_str(), value.is_number()) {
        ("equals", _) => "equals",
        ("greater_than", true) => "greaterThan",
        ("greater_than_or_equal", true) => "greaterThanOrEqualTo",
        ("less_than", true) => "lessThan",
        ("less_than_or_equal", true) => "lessThanOrEqualTo",
        _ => return false,
    };
    render(
        out,
        expression,
        matcher,
        &crate::e2e::codegen::dart::values::render_expected_value(value, Some(declared)),
    );
    true
}

fn render(out: &mut String, expression: &str, matcher: &str, value: &str) {
    out.push_str(&crate::e2e::template_env::render(
        "dart/stream_counter.jinja",
        crate::alef_context! {
            expression => expression, matcher => matcher, value => value,
        },
    ));
}

pub(in crate::e2e::codegen::dart) fn render_streaming_assertion_dart_with_usage_ir(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    context: Option<&crate::e2e::codegen::streaming_assertions::UsageAccessContext<'_>>,
) {
    if let Some(context) = context.filter(|context| context.has_declared_item())
        && let Some(field) = assertion.field.as_deref().filter(|field| is_stream_scoped_field(field))
    {
        let Some((expression, declared)) = context.typed_accessor(field, result_var) else {
            crate::e2e::codegen::streaming_assertions::render_usage_unavailable(out, field, "    ");
            return;
        };
        if try_render_with_expression(out, assertion, &expression, &declared) {
            return;
        }
    }
    super::render_streaming_assertion_dart(out, assertion, result_var);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(assertion_type: &str, field: &str, value: serde_json::Value) -> Option<String> {
        let assertion = Assertion {
            assertion_type: assertion_type.to_string(),
            field: Some(field.to_string()),
            value: Some(value),
            ..Assertion::default()
        };
        let mut out = String::new();
        try_render(&mut out, &assertion, "result").then_some(out)
    }

    #[test]
    fn a_counter_is_read_off_the_last_chunk_and_compared_as_a_number() {
        assert_eq!(
            render("equals", "usage.total_tokens", serde_json::json!(18)).as_deref(),
            Some("    expect(result.lastOrNull?.usage?.totalTokens, equals(18));\n")
        );
        assert_eq!(
            render("greater_than_or_equal", "usage.total_tokens", serde_json::json!(1)).as_deref(),
            Some("    expect(result.lastOrNull?.usage?.totalTokens, greaterThanOrEqualTo(1));\n")
        );
    }

    #[test]
    fn a_string_counter_is_compared_as_a_string() {
        assert_eq!(
            render("equals", "usage.model", serde_json::json!("gpt-4")).as_deref(),
            Some("    expect(result.lastOrNull?.usage?.model, equals('gpt-4'));\n")
        );
    }

    /// The streaming entry point routes a counter assertion here, so the emitted line is a real
    /// `expect`, not the registered skip comment the arm fell to before.
    #[test]
    fn the_streaming_entry_point_asserts_a_numeric_counter() {
        let assertion = Assertion {
            assertion_type: "equals".to_string(),
            field: Some("usage.total_tokens".to_string()),
            value: Some(serde_json::json!(18)),
            ..Assertion::default()
        };
        let mut out = String::new();
        super::super::render_streaming_assertion_dart(&mut out, &assertion, "result");
        assert_eq!(out, "    expect(result.lastOrNull?.usage?.totalTokens, equals(18));\n");
    }

    /// A call that does not stream has no collected list: `usage` is a member of the response.
    #[test]
    fn a_non_streaming_usage_assertion_never_reads_the_chunks() {
        use crate::e2e::field_access::FieldResolver;
        use std::collections::{HashMap, HashSet};
        let resolver = FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        );
        let assertion = Assertion {
            assertion_type: "equals".to_string(),
            field: Some("usage.total_tokens".to_string()),
            value: Some(serde_json::json!(18)),
            ..Assertion::default()
        };
        let mut out = String::new();
        super::super::render_assertion_dart(&mut out, &assertion, "result", false, &resolver);
        assert!(out.contains("result.usage"), "must read the response. Emitted:\n{out}");
        assert!(!out.contains("lastOrNull"), "must not read the chunks. Emitted:\n{out}");
    }

    /// flutter_rust_bridge exposes a `u64` chunk field as `BigInt`, and `BigInt == int` is never
    /// true, so the typed streaming path must type the expected value the way the non-streaming
    /// path does: `expect(result.lastOrNull?.usage?.totalTokens, equals(18))` failed with
    /// "Expected: <18> Actual: BigInt:<18>".
    #[test]
    fn a_u64_chunk_counter_is_compared_as_a_bigint() {
        use crate::core::config::{Language, ResolvedCrateConfig};
        use crate::core::ir::{FieldDef, PrimitiveType, TypeDef, TypeRef};
        let field = |name: &str, ty: TypeRef| FieldDef {
            name: name.into(),
            optional: matches!(ty, TypeRef::Optional(_)),
            ty,
            ..FieldDef::default()
        };
        let types = vec![
            TypeDef {
                name: "Chunk".into(),
                has_serde: true,
                fields: vec![field(
                    "usage",
                    TypeRef::Optional(Box::new(TypeRef::Named("Usage".into()))),
                )],
                ..TypeDef::default()
            },
            TypeDef {
                name: "Usage".into(),
                has_serde: true,
                fields: vec![
                    field("total_tokens", TypeRef::Primitive(PrimitiveType::U64)),
                    field("prompt_tokens", TypeRef::Primitive(PrimitiveType::U32)),
                ],
                ..TypeDef::default()
            },
        ];
        let config = ResolvedCrateConfig::default();
        let context = crate::e2e::codegen::streaming_assertions::UsageAccessContext {
            language: Language::Dart,
            item_type: "Chunk",
            types: &types,
            config: &config,
            swift_map: None,
        };
        let emit = |assertion_type: &str, field: &str, value: serde_json::Value| {
            let assertion = Assertion {
                assertion_type: assertion_type.to_string(),
                field: Some(field.to_string()),
                value: Some(value),
                ..Assertion::default()
            };
            let mut out = String::new();
            render_streaming_assertion_dart_with_usage_ir(&mut out, &assertion, "result", Some(&context));
            out
        };
        assert_eq!(
            emit("equals", "usage.total_tokens", serde_json::json!(18)),
            "    expect(result.lastOrNull?.usage?.totalTokens, equals(BigInt.parse('18')));\n"
        );
        assert_eq!(
            emit("greater_than_or_equal", "usage.total_tokens", serde_json::json!(1)),
            "    expect(result.lastOrNull?.usage?.totalTokens, greaterThanOrEqualTo(BigInt.parse('1')));\n"
        );
        assert_eq!(
            emit("equals", "usage.prompt_tokens", serde_json::json!(10)),
            "    expect(result.lastOrNull?.usage?.promptTokens, equals(10));\n"
        );
    }

    #[test]
    fn other_fields_and_other_assertion_types_are_left_to_the_caller() {
        assert!(render("equals", "stream_content", serde_json::json!("hi")).is_none());
        assert!(render("contains", "usage.total_tokens", serde_json::json!(1)).is_none());
        assert!(render("greater_than", "usage.total_tokens", serde_json::json!("x")).is_none());
    }
}
