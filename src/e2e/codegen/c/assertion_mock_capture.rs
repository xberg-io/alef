//! `mock.*` request-count virtual field assertion rendering for C e2e tests (alef issue #443).
//!
//! Mirrors `java::assertion_mock_capture` / `csharp::assertion_mock_capture`: `mock.*` fields
//! (see `mock_assertions::model`) resolve against the generated mock server's own request log
//! through the once-per-suite helper `mock_assertions::render_helper` emits into the file (see
//! [`append_helper_if_referenced`], called from `c::render_test_file`), not a struct field.
//!
//! ~keep This backend has FAR more call sites than the one- or three-site backends wired so far,
//! because C's `test_function.rs`/`call_patterns.rs` do not funnel every fixture through a single
//! `render_assertion` the way most backends do. The full inventory, found by grepping every
//! `fixture.assertions` reference under `src/e2e/codegen/c/` (not just the streaming one the
//! issue's own intel table names):
//!
//! 1. `assertions::render_assertion` -- the ordinary field-access oracle every plain (opaque
//!    struct result) fixture goes through. Must intercept first, exactly like every other backend.
//! 2. `streaming::emit_chat_stream_assertion` -- chat-stream fixtures never reach (1); named in
//!    the issue's own split-dispatch intel as the one site that made `c` a required lane.
//! 3. `test_function::render_test_function_impl`'s "raw C result type" path (functions returning
//!    `char*`/`int32_t`/`uintptr_t` directly) has its OWN bespoke `match assertion.assertion_type`
//!    keyed off the bare `result_var`, never `assertion.field` -- unintercepted, a `mock.*`
//!    assertion here would compile and silently assert the wrong thing (the raw result compared
//!    against the mock count) rather than skip or fail loudly.
//! 4. `call_patterns::render_bytes_test_function` -- byte-buffer methods have their own bespoke
//!    assertion loop that only understands `not_error`/`not_empty`/`not_null`; every other type,
//!    `mock.*` included, previously fell into a "not meaningful on raw byte buffer" comment.
//! 5 & 6. `test_function.rs`'s TWO per-fixture field-EXTRACTION loops (client-factory path and the
//!    second/legacy path) have no `field_resolver.is_valid_for_result` guard at all, unlike (7)
//!    below -- left alone, they walk `mock.requests.total` as if it were a real nested field path
//!    and either emit a bogus `{prefix}_{type}_mock(...)` accessor call or `bail!` generation
//!    outright via `ensure_leaf_field_exists`/`emit_nested_accessor`'s own diagnostics. These two
//!    loops must skip `mock.*` fields entirely (see `is_mock_virtual_field` calls in
//!    `test_function.rs`), leaving the field's actual rendering to (1).
//! 7. `call_patterns::render_engine_factory_test_function`'s field-extraction loop already guards
//!    on `field_resolver.is_valid_for_result`, so `mock.*` is naturally skipped there with no
//!    change needed -- but its separate `has_active_assertions` pre-check used the same
//!    "valid for result type" test to decide whether to render assertions AT ALL. A fixture whose
//!    ONLY assertion is `mock.*` would have measured zero active assertions and taken the
//!    soft-null-guard branch that renders no assertions whatsoever -- a silent full drop, not
//!    even the `FieldSkip::NotAvailableOnResultType` misattribution `ensure_capture_declared`'s
//!    own doc warns about. Fixed by counting `is_mock_virtual_field` fields as active too.
//!
//! `c::visitor` (the visitor-pattern file generator) is NOT intercepted: visitor fixtures require
//! a matching typed call (`c_visitor_fixture_has_typed_call`) and are a fundamentally different,
//! callback-driven code shape with no per-fixture HTTP-round-trip assertion story today -- the
//! same class of "architecture, not a gap" the module doc already carves out for HTTP-fixture
//! early-returns. Left unaudited beyond this note.

use std::fmt::Write as FmtWrite;

use crate::e2e::codegen::mock_assertions::{build_path_and_query, mock_capture, parse_mock_field};
use crate::e2e::fixture::Assertion;

/// C expression for the mock server's base URL -- the same bare `getenv("MOCK_SERVER_URL")` every
/// other mock-server call site in this backend already reads (`call_patterns.rs`,
/// `test_function.rs`; see those files' `mock_base`/`base_url` locals). The introspection
/// endpoints (`/__alef/requests/*`) are served by the mock binary's SHARED listener, never a
/// per-fixture one, so the bare env var -- not a fixture-derived `base_url` -- is always the
/// right root to query, exactly like every other wired backend already does. ~keep
const MOCK_SERVER_URL_EXPR: &str = "getenv(\"MOCK_SERVER_URL\")";

/// Renders an assertion against a `mock.*` virtual field into `out` and returns `true` when
/// `assertion.field` names one (and the assertion was handled), signaling the caller to skip
/// every other branch rather than fall through to ordinary field resolution.
pub(super) fn try_render_mock_capture_assertion(out: &mut String, assertion: &Assertion) -> bool {
    let Some(field) = assertion.field.as_deref() else {
        return false;
    };
    let Ok(query) = parse_mock_field(field) else {
        return false;
    };
    let capture = mock_capture("c").unwrap_or_else(|_| {
        panic!("C e2e generator: mock.* field '{field}' has no c capture-table entry (mock_assertions::snippets)")
    });
    let path_and_query = build_path_and_query(&query);
    let expr = format!("{}({MOCK_SERVER_URL_EXPR}, \"{path_and_query}\")", capture.helper_name);
    render_mock_capture_comparison(out, assertion, field, &expr);
    true
}

/// Feed the mock-request-count `expr` (an `int`) into the same `assert(<cond> && "<message>")`
/// idiom every other numeric comparison in this backend already uses (see
/// `test_function.rs`'s "Other assertions" loop for the identical wording on `equals` /
/// `greater_than_or_equal`). Unlike Java's counterpart, the field name is not embedded in the
/// message: no other assertion message in this backend's generated C does that either, and
/// skipping it means the message never needs `escape_c` for a field carrying literal `"`
/// characters (`mock.requests["POST /v1/chat"]`).
fn render_mock_capture_comparison(out: &mut String, assertion: &Assertion, field: &str, expr: &str) {
    let assertion_type = assertion.assertion_type.as_str();
    let Some(val) = &assertion.value else {
        panic!(
            "C e2e generator: mock.* field '{field}' assertion '{assertion_type}' requires a numeric value in the fixture"
        );
    };
    let Some(n) = val.as_u64() else {
        panic!(
            "C e2e generator: mock.* field '{field}' assertion '{assertion_type}' requires a numeric value in the fixture, got {val:?}"
        );
    };
    match assertion_type {
        "equals" | "count_equals" => {
            let _ = writeln!(out, "    assert({expr} == {n} && \"equals assertion failed\");");
        }
        "not_equals" => {
            let _ = writeln!(out, "    assert({expr} != {n} && \"expected not equal\");");
        }
        "greater_than" => {
            let _ = writeln!(out, "    assert({expr} > {n} && \"expected greater than\");");
        }
        "greater_than_or_equal" | "count_min" => {
            let _ = writeln!(out, "    assert({expr} >= {n} && \"expected greater than or equal\");");
        }
        "less_than" => {
            let _ = writeln!(out, "    assert({expr} < {n} && \"expected less than\");");
        }
        "less_than_or_equal" => {
            let _ = writeln!(out, "    assert({expr} <= {n} && \"expected less than or equal\");");
        }
        other => {
            panic!("C e2e generator: unsupported assertion type '{other}' on mock.* field '{field}'");
        }
    }
}

/// POSIX headers the emitted helper needs beyond this file's unconditional fixed includes
/// (`assert.h`/`stdint.h`/`string.h`/`stdio.h`/`stdlib.h`, written by `c::render_test_file`
/// before any fixture body) -- inserted only when the helper is actually referenced, so a
/// generated file with no `mock.*` assertion gets no new header dependency. Every other wired
/// backend needs no equivalent because each already has a standard-library HTTP client
/// (`java.net.http`, `HttpClient`, `net/http`, ...); C alone speaks the raw POSIX socket API,
/// which lives behind headers this file does not otherwise include. ~keep
const MOCK_HELPER_INCLUDES: &str = "#include <sys/socket.h>\n#include <netdb.h>\n#include <unistd.h>\n";

/// Appends the once-per-suite mock-request-count helper -- and the socket headers it needs -- to
/// `out` iff a fixture already rendered into it actually calls the helper. `includes_offset` is
/// the byte offset right after this file's fixed `#include` block (captured by
/// `c::render_test_file` before any fixture body is appended): the extra includes must land
/// there, not at the end, because C requires every `#include` to precede its first use. Mirrors
/// `csharp::assertion_mock_capture::append_helper_if_referenced`'s helper-only version of this
/// gate, extended with the include injection this backend alone needs (see
/// [`MOCK_HELPER_INCLUDES`]).
pub(super) fn append_helper_if_referenced(out: &mut String, includes_offset: usize) {
    let Ok(capture) = mock_capture("c") else {
        return;
    };
    if !out.contains(capture.helper_name) {
        return;
    }
    out.insert_str(includes_offset, MOCK_HELPER_INCLUDES);
    if let Some(rendered) = crate::e2e::codegen::mock_assertions::render_helper("c") {
        out.push('\n');
        out.push_str(&rendered);
    }
}

#[cfg(test)]
#[path = "assertion_mock_capture/tests.rs"]
mod tests;
