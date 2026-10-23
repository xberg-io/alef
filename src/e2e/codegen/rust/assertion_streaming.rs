//! Streaming virtual field assertion rendering for Rust e2e tests.
//!
//! Split out of `assertions.rs`: streaming virtual fields (e.g. `chunks`, `imports`,
//! `structure`) resolve against the collected `chunks` local rather than a struct field, which
//! makes their assertion rendering a self-contained concern distinct from the synthetic derived
//! fields in `assertion_synthetic.rs` (e.g. `chunks_have_content`).

use std::fmt::Write as FmtWrite;

use crate::e2e::codegen::streaming_assertions::StreamingFieldResolver;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;

/// Renders an assertion against a streaming virtual field into `out` and returns `true` when
/// `assertion.field` names one (and the assertion was handled), signaling the caller to return
/// immediately rather than fall through to struct-field assertion handling.
///
/// Streaming virtual fields: intercept before is_valid_for_result so they are never skipped.
/// These fields resolve against the `chunks` collected-list variable.
///
/// For streaming fixtures, `chunks` is bound by the collect snippet emitted in
/// `render_test_function`. For non-streaming fixtures whose result struct has a literal field
/// whose name collides with a streaming-virtual name (e.g. `chunks`, `imports`, `structure`),
/// `render_test_function` emits `let {f} = &result.{f};` before assertions, so the hardcoded
/// `chunks` identifier used below still resolves.
pub(super) fn try_render_streaming_virtual_field_assertion(
    out: &mut String,
    assertion: &Assertion,
    dep_name: &str,
    field_resolver: &FieldResolver,
    streaming_item_type: Option<&str>,
    is_streaming: bool,
) -> bool {
    let Some(f) = &assertion.field else { return false };
    // A stream-scoped field (`usage.total_tokens`) is an ordinary response field on a call that
    // does not stream, so only a streaming call may claim it.
    let claimed = crate::e2e::codegen::streaming_assertions::is_streaming_virtual_field(f)
        || (is_streaming && crate::e2e::codegen::streaming_assertions::is_stream_scoped_field(f));
    if f.is_empty() || !claimed {
        return false;
    }

    if let Some(expr) = StreamingFieldResolver::accessor_with_streaming_context(
        f,
        "rust",
        "chunks",
        Some(dep_name),
        streaming_item_type,
    ) {
        // `field_resolver.is_optional` answers for the *declared* IR field, but the expression
        // actually emitted is whatever the streaming resolver built — and most of its accessors
        // flatten the declared `Option` away (`.iter().flatten()`, `.unwrap_or_default()`) while
        // pinning a concrete type. Ask the accessor owner which shape it produced instead of
        // assuming the declared wrapper survived: appending `.as_ref()` to a `Vec`- or
        // `String`-typed chain is an unannotatable `AsRef<T>` call (E0282), not a no-op. ~keep
        let option_wrapper_survives = field_resolver.is_optional(f)
            && StreamingFieldResolver::accessor_is_collected_var_passthrough(f, "rust", "chunks");
        match assertion.assertion_type.as_str() {
            "count_min" => {
                if let Some(val) = &assertion.value
                    && let Some(n) = val.as_u64()
                {
                    let expr_for_len = if option_wrapper_survives {
                        format!("{expr}.as_ref().map_or(0, |v| v.len())")
                    } else {
                        format!("{expr}.len()")
                    };
                    let _ = writeln!(
                        out,
                        "    assert!({expr_for_len} >= {n} as usize, \"expected >= {n} chunks\");"
                    );
                } else {
                    panic!(
                        "Rust e2e generator: streaming field '{f}' assertion 'count_min' requires a numeric value in the fixture, got {:?}",
                        assertion.value
                    );
                }
            }
            "count_equals" => {
                if let Some(val) = &assertion.value
                    && let Some(n) = val.as_u64()
                {
                    let expr_for_len = if option_wrapper_survives {
                        format!("{expr}.as_ref().map_or(0, |v| v.len())")
                    } else {
                        format!("{expr}.len()")
                    };
                    let _ = writeln!(
                        out,
                        "    assert_eq!({expr_for_len}, {n} as usize, \"expected exactly {n} chunks\");"
                    );
                } else {
                    panic!(
                        "Rust e2e generator: streaming field '{f}' assertion 'count_equals' requires a numeric value in the fixture, got {:?}",
                        assertion.value
                    );
                }
            }
            "equals" => {
                if let Some(serde_json::Value::String(s)) = &assertion.value {
                    let escaped = crate::e2e::escape::escape_rust(s);
                    let _ = writeln!(out, "    assert_eq!({expr}, \"{escaped}\");");
                } else if let Some(val) = &assertion.value {
                    let lit = super::assertion_synthetic::numeric_literal(val);
                    let _ = writeln!(out, "    assert_eq!({expr}, {lit});");
                } else {
                    panic!(
                        "Rust e2e generator: streaming field '{f}' assertion 'equals' requires a string or numeric value in the fixture, got {:?}",
                        assertion.value
                    );
                }
            }
            "not_empty" => {
                let check_expr = if option_wrapper_survives {
                    format!("{expr}.as_ref().is_some_and(|v| !v.is_empty())")
                } else {
                    format!("!{expr}.is_empty()")
                };
                let _ = writeln!(out, "    assert!({check_expr}, \"expected non-empty\");");
            }
            "is_empty" => {
                let check_expr = if option_wrapper_survives {
                    format!("{expr}.as_ref().is_none_or(|v| v.is_empty())")
                } else {
                    format!("{expr}.is_empty()")
                };
                let _ = writeln!(out, "    assert!({check_expr}, \"expected empty\");");
            }
            "is_true" => {
                let _ = writeln!(out, "    assert!({expr}, \"expected true\");");
            }
            "is_false" => {
                let _ = writeln!(out, "    assert!(!{expr}, \"expected false\");");
            }
            "greater_than" => {
                if let Some(val) = &assertion.value {
                    let lit = super::assertion_synthetic::numeric_literal(val);
                    let _ = writeln!(out, "    assert!({expr} > {lit}, \"expected > {lit}\");");
                } else {
                    panic!(
                        "Rust e2e generator: streaming field '{f}' assertion 'greater_than' requires a numeric value in the fixture, got {:?}",
                        assertion.value
                    );
                }
            }
            "greater_than_or_equal" => {
                if let Some(val) = &assertion.value {
                    let lit = super::assertion_synthetic::numeric_literal(val);
                    let _ = writeln!(out, "    assert!({expr} >= {lit}, \"expected >= {lit}\");");
                } else {
                    panic!(
                        "Rust e2e generator: streaming field '{f}' assertion 'greater_than_or_equal' requires a numeric value in the fixture, got {:?}",
                        assertion.value
                    );
                }
            }
            "contains" => {
                if let Some(serde_json::Value::String(s)) = &assertion.value {
                    let escaped = crate::e2e::escape::escape_rust(s);
                    let _ = writeln!(
                        out,
                        "    assert!({expr}.contains(\"{escaped}\"), \"expected to contain: {escaped}\");"
                    );
                } else {
                    panic!(
                        "Rust e2e generator: streaming field '{f}' assertion 'contains' requires a string value in the fixture, got {:?}",
                        assertion.value
                    );
                }
            }
            other => {
                panic!("Rust e2e generator: unsupported assertion type '{other}' on streaming field '{f}'");
            }
        }
    } else {
        panic!(
            "Rust e2e generator: streaming field '{f}' has no accessor for context (streaming_item_type={streaming_item_type:?}); check the streaming adapter configuration"
        );
    }
    true
}
