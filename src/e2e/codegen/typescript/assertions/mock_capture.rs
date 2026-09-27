//! `mock.*` request-count virtual field assertion rendering for TypeScript/Node e2e tests (alef
//! issue #443).
//!
//! Mirrors `rust::assertion_mock_capture` / `python::assertion_mock_capture`: `mock.*` fields
//! (see `mock_assertions::model`) resolve against the generated mock server's own request log
//! through the once-per-suite helper `mock_assertions::render_helper` emits into the file, not a
//! struct field. Every generated `it(...)` callback is `async () => { ... }` unconditionally (see
//! `test_case.rs`/`http_test.jinja`), so -- unlike the Python backend -- there is no sync/async
//! split to thread through here: `await` is always valid.
//!
//! `"node"` and `"wasm"` both have a `mock_assertions::snippets::mock_capture` entry -- the WASM
//! e2e generator (`e2e::codegen::wasm`) renders its test files through this same
//! `typescript::render_test_file` / `render_assertion` machinery (with `lang = "wasm"`), running
//! under the identical vitest-on-Node.js harness `"node"` uses, so one template and one call site
//! serve both. [`render`] returns `false` only for a language with no capture-table entry at all,
//! in which case the caller falls through to whatever that language's own (pre-existing)
//! field-refusal/skip handling does.

use crate::e2e::codegen::mock_assertions::{build_path_and_query, mock_capture, parse_mock_field};
use crate::e2e::fixture::Assertion;

/// TypeScript expression for the mock server's base URL -- the single mock server
/// `globalSetup.ts.jinja` spawns once per test run, not a per-test instance (see
/// `test_file/args.rs`'s identical `process.env.MOCK_SERVER_URL` usage for per-fixture base
/// URLs), so every fixture reaches it through this same environment variable. Wrapped in a
/// template literal, matching that existing call-site idiom, so a `string | undefined` value is
/// always coerced to `string` without a `!` non-null assertion.
const MOCK_SERVER_URL_EXPR: &str = "`${process.env.MOCK_SERVER_URL}`";

/// Renders an assertion against a `mock.*` virtual field into `out`, returning `true` when
/// `field` names one AND `lang` has a capture-table entry (i.e. `lang == "node"`).
pub(super) fn render(out: &mut String, assertion: &Assertion, field: &str, lang: &str) -> bool {
    let Ok(query) = parse_mock_field(field) else {
        return false;
    };
    let Ok(capture) = mock_capture(lang) else {
        return false;
    };
    let path_and_query = build_path_and_query(&query);
    let expr = format!(
        "await {}({MOCK_SERVER_URL_EXPR}, \"{path_and_query}\")",
        capture.helper_name
    );
    render_mock_capture_comparison(out, assertion, field, &expr);
    true
}

/// Feed the mock-request-count `expr` (a `number`) into the same numeric comparison operators
/// every other integer-valued field already supports.
fn render_mock_capture_comparison(out: &mut String, assertion: &Assertion, field: &str, expr: &str) {
    let assertion_type = assertion.assertion_type.as_str();
    let Some(val) = &assertion.value else {
        panic!(
            "TypeScript e2e generator: mock.* field '{field}' assertion '{assertion_type}' requires a numeric value in the fixture"
        );
    };
    let Some(n) = val.as_u64() else {
        panic!(
            "TypeScript e2e generator: mock.* field '{field}' assertion '{assertion_type}' requires a numeric value in the fixture, got {val:?}"
        );
    };
    // Unlike the Rust/Python renderers, `field` is never spliced into the emitted expression
    // here -- vitest's own `expect(...).toBe(...)` failure message already reports actual vs.
    // expected, so no custom message string (and therefore no escaping of `field`) is needed.
    match assertion_type {
        "equals" | "count_equals" => {
            out.push_str(&format!("    expect({expr}).toBe({n});\n"));
        }
        "greater_than" => {
            out.push_str(&format!("    expect({expr}).toBeGreaterThan({n});\n"));
        }
        "greater_than_or_equal" | "count_min" => {
            out.push_str(&format!("    expect({expr}).toBeGreaterThanOrEqual({n});\n"));
        }
        "less_than" => {
            out.push_str(&format!("    expect({expr}).toBeLessThan({n});\n"));
        }
        "less_than_or_equal" => {
            out.push_str(&format!("    expect({expr}).toBeLessThanOrEqual({n});\n"));
        }
        "not_equals" => {
            out.push_str(&format!("    expect({expr}).not.toBe({n});\n"));
        }
        other => {
            panic!("TypeScript e2e generator: unsupported assertion type '{other}' on mock.* field '{field}'");
        }
    }
}

#[cfg(test)]
#[path = "mock_capture/tests.rs"]
mod tests;
