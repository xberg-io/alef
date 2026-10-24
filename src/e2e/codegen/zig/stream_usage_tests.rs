//! A streaming Zig fixture asserts `usage.total_tokens` through the JSON-struct path, not through
//! the shared streaming accessor: Zig keeps every chunk as a JSON string, parses the LAST one and
//! reads the counter off the parsed object. This pins that path, so the streaming usage assertion
//! stays a real comparison.

use super::assertions::render_json_assertion;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;
use std::collections::{HashMap, HashSet};

fn resolver() -> FieldResolver {
    FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    )
}

fn render_streaming(assertion_type: &str, field: &str, value: serde_json::Value) -> String {
    let assertion = Assertion {
        assertion_type: assertion_type.to_string(),
        field: Some(field.to_string()),
        value: Some(value),
        ..Assertion::default()
    };
    let mut out = String::new();
    render_json_assertion(&mut out, &assertion, "result", &resolver(), true);
    out
}

#[test]
fn a_streaming_usage_counter_is_compared_against_the_parsed_last_chunk() {
    let out = render_streaming("equals", "usage.total_tokens", serde_json::json!(18));
    assert!(
        !out.contains("skipped"),
        "the usage assertion must not skip. Emitted:\n{out}"
    );
    assert!(
        out.contains("testing.expectEqual(") && out.contains("18"),
        "the counter must be compared with the fixture value. Emitted:\n{out}"
    );
    assert!(
        out.contains("\"usage\"") && out.contains("\"total_tokens\""),
        "the counter must be read from `usage.total_tokens` of the parsed chunk. Emitted:\n{out}"
    );
    assert!(
        !out.contains("chunks.items"),
        "the counter is not read from the collected list. Emitted:\n{out}"
    );
}
