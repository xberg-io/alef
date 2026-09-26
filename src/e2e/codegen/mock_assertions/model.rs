//! Grammar for the `mock.*` request-count virtual-field namespace (alef issue #433).
//!
//! ~keep Follows the pattern `docs/design/assertion-kinds.md` and `assertion_kinds_design.rs`
//! establish: a virtual-field namespace whose accessor reads state the generator arranges around
//! the call, with `streaming_assertions` as the working instance. `mock.*` is the request-COUNT
//! capture -- the generated mock HTTP server exposes a plain-text integer introspection endpoint,
//! and a fixture asserts against it the same way it asserts against `stream.items.length`: through
//! a name that is not a field of any result type.
//!
//! # Grammar
//!
//! ```text
//! mock.requests.total          -- every request the mock server has recorded
//! mock.requests["<key>"]       -- requests matching <key> exactly, where <key> is either
//!                                 "<METHOD> <path>" or "<path>" (any method)
//! ```
//!
//! `<key>` is passed through verbatim to the mock server's introspection endpoint; this module
//! does not interpret its shape any further than "one double-quoted, non-empty string". See
//! `src/e2e/schema/fixture.schema.json` for the documented endpoints and response-body tokens.

/// The root every `mock.*` virtual field starts with.
pub(crate) const MOCK_REQUESTS_ROOT: &str = "mock.requests";

/// The one statically-nameable `mock.*` field. The bracket form (`mock.requests["<key>"]`) is
/// open-ended and cannot be enumerated, so it is recognised structurally by [`parse_mock_field`]
/// rather than listed here -- mirrors `streaming_assertions::model::STREAMING_VIRTUAL_ROOTS`
/// standing in for `tool_calls[0]...` deep paths that `STREAMING_VIRTUAL_FIELDS` cannot list.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "no emitter consumes it until the reference-backend wave -- #433"
    )
)]
pub(crate) const MOCK_VIRTUAL_FIELDS: &[&str] = &["mock.requests.total"];

/// `assertion_recipes` opt-in name gating every `mock.*` field. `mock.*` captures an HTTP round
/// trip into the mock server for every test that asserts it, so enabling it must be a fixture
/// author's deliberate choice -- the same rationale `assertion_kinds_design.rs` gives for
/// `TIMING_RECIPE`, not `outcome.*`'s deliberately-ungated default.
pub(crate) const MOCK_RECIPE: &str = "mock_requests";

/// A parsed `mock.*` field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MockQuery {
    /// `mock.requests.total`.
    Total,
    /// `mock.requests["<key>"]` -- `<key>` is the bracket's quoted content, verbatim.
    Count(String),
}

/// Why a candidate `mock.*` field did not parse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MockFieldSyntax {
    /// The field does not start with [`MOCK_REQUESTS_ROOT`] at all.
    NotMockField,
    /// Exactly `mock.requests`, with no `.total` or bracket continuation.
    BareRoot,
    /// A continuation that is neither `.total` nor a `[...]` bracket.
    UnrecognizedTail,
    /// A `[` with no matching `]`.
    UnbalancedBracket,
    /// A bracket whose content is not wrapped in a matching pair of double quotes.
    MissingQuotedKey,
    /// A bracket whose quoted content is the empty string.
    EmptyKey,
}

/// Parse `field` against the `mock.*` grammar.
///
/// ~keep Reuses the bracket-with-key idiom `streaming_assertions::renderers::parse_tail` already
/// established for `tool_calls[0].function.name`, rather than inventing a second parser: split on
/// the literal `[`/`]` delimiters and treat everything between them as the segment's payload. The
/// only new element here is the quoted-string payload the bracket carries, since a mock request
/// key is text (`"POST /v1/chat"`), not a numeric index.
pub(crate) fn parse_mock_field(field: &str) -> Result<MockQuery, MockFieldSyntax> {
    let Some(rest) = field.strip_prefix(MOCK_REQUESTS_ROOT) else {
        return Err(MockFieldSyntax::NotMockField);
    };
    if rest.is_empty() {
        return Err(MockFieldSyntax::BareRoot);
    }
    if rest == ".total" {
        return Ok(MockQuery::Total);
    }
    let Some(bracketed) = rest.strip_prefix('[') else {
        return Err(MockFieldSyntax::UnrecognizedTail);
    };
    let Some(inner) = bracketed.strip_suffix(']') else {
        return Err(MockFieldSyntax::UnbalancedBracket);
    };
    let Some(key) = inner.strip_prefix('"').and_then(|rest| rest.strip_suffix('"')) else {
        return Err(MockFieldSyntax::MissingQuotedKey);
    };
    if key.is_empty() {
        return Err(MockFieldSyntax::EmptyKey);
    }
    Ok(MockQuery::Count(key.to_string()))
}

/// Whether `field` names a `mock.*` virtual field, static or bracketed.
///
/// Mirrors `streaming_assertions::is_streaming_virtual_field` -- called at the same point in a
/// backend's `render_assertion` (before the `is_valid_for_result` guard, after the
/// traversal/wildcard guards) once a later wave wires per-language interception.
pub(crate) fn is_mock_virtual_field(field: &str) -> bool {
    parse_mock_field(field).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn total_parses() {
        assert_eq!(parse_mock_field("mock.requests.total"), Ok(MockQuery::Total));
    }

    #[test]
    fn method_and_path_key_parses() {
        assert_eq!(
            parse_mock_field(r#"mock.requests["POST /v1/chat"]"#),
            Ok(MockQuery::Count("POST /v1/chat".to_string()))
        );
    }

    #[test]
    fn bare_path_key_parses() {
        assert_eq!(
            parse_mock_field(r#"mock.requests["/v1/chat"]"#),
            Ok(MockQuery::Count("/v1/chat".to_string()))
        );
    }

    #[test]
    fn round_trips_the_documented_grammar() {
        for field in [
            "mock.requests.total",
            r#"mock.requests["POST /v1/chat"]"#,
            r#"mock.requests["/v1/chat"]"#,
        ] {
            assert!(is_mock_virtual_field(field), "expected {field} to parse");
        }
    }

    /// The grammar's own negative examples, named directly in the task contract. ~keep
    #[test]
    fn rejects_bare_root() {
        assert_eq!(parse_mock_field("mock.requests"), Err(MockFieldSyntax::BareRoot));
        assert!(!is_mock_virtual_field("mock.requests"));
    }

    #[test]
    fn rejects_unrelated_root() {
        assert_eq!(parse_mock_field("mock.foo"), Err(MockFieldSyntax::NotMockField));
        assert!(!is_mock_virtual_field("mock.foo"));
    }

    #[test]
    fn rejects_unbalanced_brackets() {
        assert_eq!(
            parse_mock_field("mock.requests[\"POST /v1/chat\""),
            Err(MockFieldSyntax::UnbalancedBracket)
        );
        assert_eq!(
            parse_mock_field("mock.requests["),
            Err(MockFieldSyntax::UnbalancedBracket)
        );
        assert!(!is_mock_virtual_field("mock.requests["));
    }

    #[test]
    fn rejects_an_unquoted_key() {
        assert_eq!(
            parse_mock_field("mock.requests[chat]"),
            Err(MockFieldSyntax::MissingQuotedKey)
        );
    }

    #[test]
    fn rejects_an_empty_key() {
        assert_eq!(parse_mock_field(r#"mock.requests[""]"#), Err(MockFieldSyntax::EmptyKey));
    }

    #[test]
    fn rejects_an_unrecognized_tail() {
        assert_eq!(
            parse_mock_field("mock.requests.count"),
            Err(MockFieldSyntax::UnrecognizedTail)
        );
    }

    /// Every statically-nameable field must itself parse -- a consumer enumerating
    /// [`MOCK_VIRTUAL_FIELDS`] (docs generation, fixture validation) must never be handed a name
    /// [`parse_mock_field`] then rejects. ~keep
    #[test]
    fn every_static_field_parses() {
        for field in MOCK_VIRTUAL_FIELDS {
            assert!(is_mock_virtual_field(field), "{field} is listed but does not parse");
        }
    }
}
