//! Tests for user-typed `[N]` array index syntax in fixture field paths.
//!
//! Verifies that `parse_path` recognises bracket-index notation directly:
//! `choices[0].message.content` → `ArrayField{name:"choices",index:0}`
//! → `Field("message")` → `Field("content")`.
//!
//! Explicit indices (`[2]`, `[1]`, …) are preserved through all per-language
//! renderers so that each target language emits the correct accessor
//! expression without falling back to the old "always [0]" behaviour.

use alef::e2e::field_access::FieldResolver;
use std::collections::{HashMap, HashSet};

fn empty_resolver() -> FieldResolver {
    FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    )
}

// ── choices[0].message.content ────────────────────────────────────────────────

#[test]
fn choices_0_message_content_rust() {
    let r = empty_resolver();
    assert_eq!(
        r.accessor("choices[0].message.content", "rust", "result"),
        "result.choices[0].message.content"
    );
}

#[test]
fn choices_0_message_content_python() {
    let r = empty_resolver();
    assert_eq!(
        r.accessor("choices[0].message.content", "python", "result"),
        "result.choices[0].message.content"
    );
}

#[test]
fn choices_0_message_content_typescript() {
    let r = empty_resolver();
    assert_eq!(
        r.accessor("choices[0].message.content", "typescript", "result"),
        "result.choices[0].message.content"
    );
}

#[test]
fn choices_0_message_content_java() {
    let r = empty_resolver();
    assert_eq!(
        r.accessor("choices[0].message.content", "java", "result"),
        "result.choices().get(0).message().content()"
    );
}

#[test]
fn choices_0_message_content_kotlin() {
    let r = empty_resolver();
    assert_eq!(
        r.accessor("choices[0].message.content", "kotlin", "result"),
        "result.choices().first().message().content()"
    );
}

#[test]
fn choices_0_message_content_csharp() {
    let r = empty_resolver();
    assert_eq!(
        r.accessor("choices[0].message.content", "csharp", "result"),
        "result.Choices[0].Message.Content"
    );
}

#[test]
fn choices_0_message_content_swift() {
    let r = empty_resolver();
    // With no Swift first-class map configured, unknown roots default to
    // swift-bridge method access.
    assert_eq!(
        r.accessor("choices[0].message.content", "swift", "result"),
        "result.choices()[0].message().content()"
    );
}

#[test]
fn choices_0_message_content_go() {
    let r = empty_resolver();
    assert_eq!(
        r.accessor("choices[0].message.content", "go", "result"),
        "result.Choices[0].Message.Content"
    );
}

#[test]
fn choices_0_message_content_zig() {
    let r = empty_resolver();
    assert_eq!(
        r.accessor("choices[0].message.content", "zig", "result"),
        "result.choices[0].message.content"
    );
}

#[test]
fn choices_0_message_content_elixir() {
    let r = empty_resolver();
    assert_eq!(
        r.accessor("choices[0].message.content", "elixir", "result"),
        "Enum.at(result.choices, 0).message.content"
    );
}

#[test]
fn choices_0_message_content_ruby() {
    let r = empty_resolver();
    assert_eq!(
        r.accessor("choices[0].message.content", "ruby", "result"),
        "result.choices[0].message.content"
    );
}

#[test]
fn choices_0_message_content_php() {
    let r = empty_resolver();
    assert_eq!(
        r.accessor("choices[0].message.content", "php", "$result"),
        "$result->choices[0]->message->content"
    );
}

#[test]
fn choices_0_message_content_dart() {
    let r = empty_resolver();
    assert_eq!(
        r.accessor("choices[0].message.content", "dart", "result"),
        "result.choices[0].message.content"
    );
}

// ── data[2].text ──────────────────────────────────────────────────────────────

#[test]
fn data_2_text_rust() {
    let r = empty_resolver();
    assert_eq!(r.accessor("data[2].text", "rust", "result"), "result.data[2].text");
}

#[test]
fn data_2_text_python() {
    let r = empty_resolver();
    assert_eq!(r.accessor("data[2].text", "python", "result"), "result.data[2].text");
}

#[test]
fn data_2_text_typescript() {
    let r = empty_resolver();
    assert_eq!(
        r.accessor("data[2].text", "typescript", "result"),
        "result.data[2].text"
    );
}

#[test]
fn data_2_text_java() {
    let r = empty_resolver();
    assert_eq!(
        r.accessor("data[2].text", "java", "result"),
        "result.data().get(2).text()"
    );
}

#[test]
fn data_2_text_kotlin() {
    let r = empty_resolver();
    // index 2 ≠ 0, so use .get(2) instead of .first()
    assert_eq!(
        r.accessor("data[2].text", "kotlin", "result"),
        "result.data().get(2).text()"
    );
}

#[test]
fn data_2_text_csharp() {
    let r = empty_resolver();
    assert_eq!(r.accessor("data[2].text", "csharp", "result"), "result.Data[2].Text");
}

#[test]
fn data_2_text_swift() {
    let r = empty_resolver();
    assert_eq!(r.accessor("data[2].text", "swift", "result"), "result.data()[2].text()");
}

#[test]
fn data_2_text_go() {
    let r = empty_resolver();
    assert_eq!(r.accessor("data[2].text", "go", "result"), "result.Data[2].Text");
}

#[test]
fn data_2_text_zig() {
    let r = empty_resolver();
    assert_eq!(r.accessor("data[2].text", "zig", "result"), "result.data[2].text");
}

#[test]
fn data_2_text_elixir() {
    let r = empty_resolver();
    assert_eq!(
        r.accessor("data[2].text", "elixir", "result"),
        "Enum.at(result.data, 2).text"
    );
}

// ── errors[1].messages[0].detail ─────────────────────────────────────────────

#[test]
fn nested_array_indices_rust() {
    let r = empty_resolver();
    assert_eq!(
        r.accessor("errors[1].messages[0].detail", "rust", "result"),
        "result.errors[1].messages[0].detail"
    );
}

#[test]
fn nested_array_indices_python() {
    let r = empty_resolver();
    assert_eq!(
        r.accessor("errors[1].messages[0].detail", "python", "result"),
        "result.errors[1].messages[0].detail"
    );
}

#[test]
fn nested_array_indices_typescript() {
    let r = empty_resolver();
    assert_eq!(
        r.accessor("errors[1].messages[0].detail", "typescript", "result"),
        "result.errors[1].messages[0].detail"
    );
}

#[test]
fn nested_array_indices_java() {
    let r = empty_resolver();
    assert_eq!(
        r.accessor("errors[1].messages[0].detail", "java", "result"),
        "result.errors().get(1).messages().get(0).detail()"
    );
}

#[test]
fn nested_array_indices_kotlin() {
    let r = empty_resolver();
    // errors[1] → .get(1), messages[0] → .first()
    assert_eq!(
        r.accessor("errors[1].messages[0].detail", "kotlin", "result"),
        "result.errors().get(1).messages().first().detail()"
    );
}

#[test]
fn nested_array_indices_csharp() {
    let r = empty_resolver();
    assert_eq!(
        r.accessor("errors[1].messages[0].detail", "csharp", "result"),
        "result.Errors[1].Messages[0].Detail"
    );
}

#[test]
fn nested_array_indices_go() {
    let r = empty_resolver();
    assert_eq!(
        r.accessor("errors[1].messages[0].detail", "go", "result"),
        "result.Errors[1].Messages[0].Detail"
    );
}

#[test]
fn nested_array_indices_elixir() {
    let r = empty_resolver();
    assert_eq!(
        r.accessor("errors[1].messages[0].detail", "elixir", "result"),
        "Enum.at(Enum.at(result.errors, 1).messages, 0).detail"
    );
}

// ── explicit index takes precedence over config default ───────────────────────

#[test]
fn explicit_index_overrides_config_default() {
    // When the user writes `choices[2]` and `choices` is also in array_fields,
    // the explicit index 2 must take precedence over the default index 0.
    let mut arrays = HashSet::new();
    arrays.insert("choices".to_string());
    let r = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &arrays,
        &HashSet::new(),
    );
    assert_eq!(
        r.accessor("choices[2].message.content", "rust", "result"),
        "result.choices[2].message.content"
    );
    assert_eq!(
        r.accessor("choices[2].message.content", "python", "result"),
        "result.choices[2].message.content"
    );
}

// ── Swift optional-chain subscript on Optional<Vec<T>> getter ─────────────────
//
// When an array field is listed in `fields_optional` (meaning the getter
// returns `Optional<RustVec<T>>` in Swift), the subscript must use `()?[N]`
// so Swift can unwrap the Optional before indexing.  Subsequent non-leaf
// segments must also use `?.` chaining.
//
// Mirrors the real-world fixture:
//   field = "choices[0].message.tool_calls[0].function.name"
//   fields_optional = ["choices[0].message.tool_calls"]

fn resolver_with_optional(optional_path: &str) -> FieldResolver {
    let mut optional = HashSet::new();
    optional.insert(optional_path.to_string());
    FieldResolver::new(
        &HashMap::new(),
        &optional,
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    )
}

#[test]
fn swift_optional_array_field_subscript_uses_optional_chain() {
    // tool_calls[0] is an explicit-index ArrayField where the getter returns
    // Optional<RustVec<T>>.  Swift accessor must emit `()?[0]` not `()[0]`.
    let r = resolver_with_optional("choices[0].message.tool_calls");
    assert_eq!(
        r.accessor("choices[0].message.tool_calls[0].function.name", "swift", "result"),
        "result.choices()[0].message().toolCalls()?[0].function().name()"
    );
}

#[test]
fn swift_optional_array_field_leaf_no_trailing_question() {
    // When tool_calls[0] is the leaf (last segment), no trailing `?` should be
    // appended — the Optional subscript is correct on its own.
    let r = resolver_with_optional("choices[0].message.tool_calls");
    assert_eq!(
        r.accessor("choices[0].message.tool_calls[0]", "swift", "result"),
        "result.choices()[0].message().toolCalls()?[0]"
    );
}

#[test]
fn swift_non_optional_array_field_unchanged() {
    // Array fields NOT in fields_optional emit plain `[N]` without `?`.
    let r = resolver_with_optional("choices[0].message.tool_calls");
    assert_eq!(
        r.accessor("choices[0].message.content", "swift", "result"),
        "result.choices()[0].message().content()"
    );
}

#[test]
fn swift_path_so_far_includes_index_for_subsequent_checks() {
    // After processing `choices[0]` (optional), path_so_far must be "choices[0]"
    // so that a subsequent Field segment can build "choices[0].message" for its
    // optional check.  This test uses a resolver where "choices[0].message" is
    // optional to verify the index suffix is correctly threaded.
    let r = resolver_with_optional("choices[0].message");
    assert_eq!(
        r.accessor("choices[0].message.content", "swift", "result"),
        "result.choices()[0].message()?.content()"
    );
}

// ── string-keyed map access is unaffected ─────────────────────────────────────

#[test]
fn string_bracket_key_stays_map_access() {
    // `meta[key]` must still produce MapAccess, not ArrayField.
    let r = empty_resolver();
    assert_eq!(
        r.accessor("meta[key].value", "rust", "result"),
        "result.meta.get(\"key\").map(|s| s.as_str()).value"
    );
}
