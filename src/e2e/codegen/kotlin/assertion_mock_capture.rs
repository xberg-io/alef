//! `mock.*` request-count virtual field assertion rendering for Kotlin e2e tests (alef issue #443).
//!
//! Mirrors `java::assertion_mock_capture`: `mock.*` fields (see `mock_assertions::model`) resolve
//! against the generated mock server's own request log through the once-per-suite helper
//! `mock_assertions::render_helper` emits into the file (see
//! `kotlin::test_file::render_test_file_inner`), not a struct field, so interception happens
//! before every other branch in `assertions::render_assertion_with_streaming_context` --
//! including every gate `assertion_field_gates::try_render_field_shape_gates` runs, starting
//! with `try_skip_field_not_available_on_result_type`, which would otherwise misclassify a
//! `mock.*` field as `FieldSkip::NotAvailableOnResultType` (blaming the fixture) rather than
//! alef's own generator gap. The helper is a private member function of the generated test
//! class, so every call site here reads a bare method call (same class, no qualifier needed).
//!
//! Shared verbatim between `kotlin` and `kotlin_android`: both suites run on the host JVM, never
//! a real Android device (see `kotlin_android.rs`'s module doc), so the emitted call and the
//! helper it references are identical for either target -- see
//! `mock_assertions::snippets::MOCK_CAPTURE_TABLE`'s `"kotlin"`/`"kotlin_android"` rows, which
//! point at the same template and helper name for exactly that reason.

use std::fmt::Write as FmtWrite;

use crate::e2e::codegen::mock_assertions::{build_path_and_query, mock_capture, parse_mock_field};
use crate::e2e::escape::escape_kotlin;
use crate::e2e::fixture::Assertion;

/// Kotlin expression for the mock server's base URL -- matches every other per-suite (not
/// per-fixture) mock-server URL read in this backend; `test_method.rs`'s per-fixture mock URLs
/// also fall back to this exact env var.
const MOCK_SERVER_URL_EXPR: &str = "System.getenv(\"MOCK_SERVER_URL\")";

/// Renders an assertion against a `mock.*` virtual field into `out` and returns `true` when
/// `assertion.field` names one (and the assertion was handled), signaling the caller to return
/// immediately rather than fall through to ordinary field resolution. `kotlin_android_style`
/// selects which capture-table row's `helper_name` to call -- see this module's doc for why the
/// two rows currently emit the identical name.
pub(super) fn try_render_mock_capture_assertion(
    out: &mut String,
    assertion: &Assertion,
    kotlin_android_style: bool,
) -> bool {
    let Some(field) = assertion.field.as_deref() else {
        return false;
    };
    let Ok(query) = parse_mock_field(field) else {
        return false;
    };
    let lang = if kotlin_android_style {
        "kotlin_android"
    } else {
        "kotlin"
    };
    let capture = mock_capture(lang).unwrap_or_else(|_| {
        panic!(
            "Kotlin e2e generator: mock.* field '{field}' has no {lang} capture-table entry (mock_assertions::snippets)"
        )
    });
    let path_and_query = build_path_and_query(&query);
    let expr = format!("{}({MOCK_SERVER_URL_EXPR}, \"{path_and_query}\")", capture.helper_name);
    render_mock_capture_comparison(out, assertion, field, &expr);
    true
}

/// Feed the mock-request-count `expr` (an `Int`) into the same `kotlin.test` assertion family
/// every other numeric comparison in this backend already uses (`assertEquals`/`assertTrue` are
/// unconditionally imported by `test_file.rs`).
fn render_mock_capture_comparison(out: &mut String, assertion: &Assertion, field: &str, expr: &str) {
    let assertion_type = assertion.assertion_type.as_str();
    let Some(val) = &assertion.value else {
        panic!(
            "Kotlin e2e generator: mock.* field '{field}' assertion '{assertion_type}' requires a numeric value in the fixture"
        );
    };
    let Some(n) = val.as_u64() else {
        panic!(
            "Kotlin e2e generator: mock.* field '{field}' assertion '{assertion_type}' requires a numeric value in the fixture, got {val:?}"
        );
    };
    // `field` is embedded in the emitted assertion message below, inside a Kotlin string literal
    // -- `mock.requests["POST /v1/chat"]` carries literal `"` characters that must be escaped
    // there.
    let escaped_field = escape_kotlin(field);
    match assertion_type {
        "equals" | "count_equals" => {
            let _ = writeln!(
                out,
                "        assertEquals({n}, {expr}, \"expected {escaped_field} == {n}\")"
            );
        }
        "not_equals" => {
            // Fully qualified: `kotlin.test.assertNotEquals` is not part of this backend's
            // unconditional `kotlin.test.*` import list (see `test_file.rs`), and gating a new
            // import on this one rarely-used arm is not worth it. ~keep
            let _ = writeln!(
                out,
                "        kotlin.test.assertNotEquals({n}, {expr}, \"expected {escaped_field} != {n}\")"
            );
        }
        "greater_than" => {
            let _ = writeln!(
                out,
                "        assertTrue({expr} > {n}, \"expected {escaped_field} > {n}\")"
            );
        }
        "greater_than_or_equal" | "count_min" => {
            let _ = writeln!(
                out,
                "        assertTrue({expr} >= {n}, \"expected {escaped_field} >= {n}\")"
            );
        }
        "less_than" => {
            let _ = writeln!(
                out,
                "        assertTrue({expr} < {n}, \"expected {escaped_field} < {n}\")"
            );
        }
        "less_than_or_equal" => {
            let _ = writeln!(
                out,
                "        assertTrue({expr} <= {n}, \"expected {escaped_field} <= {n}\")"
            );
        }
        other => {
            panic!("Kotlin e2e generator: unsupported assertion type '{other}' on mock.* field '{field}'");
        }
    }
}

#[cfg(test)]
#[path = "assertion_mock_capture/tests.rs"]
mod tests;
