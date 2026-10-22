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
//! mock.requests["<key>"].body.<path>
//!                              -- a field of the JSON body of those requests. Fixture authors
//!                                 write this form; `normalize_body_assertion` rewrites it, at
//!                                 fixture load, into the predicate form below
//! mock.requests["<key>"].body.<path> == <json>
//! mock.requests["<key>"].body.<path> != <json>
//!                              -- the predicate form: the mock server answers `1` when the
//!                                 comparison holds for the requests matching <key>, else `0`
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
    /// `mock.requests["<key>"].body.<path> == <json>` (or `!=`): whether the JSON body of the
    /// requests matching `<key>` holds `<json>` at `<path>`.
    BodyPredicate {
        key: String,
        path: String,
        op: BodyOp,
        /// The expected value as compact JSON text.
        value_json: String,
    },
}

/// The comparison a [`MockQuery::BodyPredicate`] makes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BodyOp {
    Equals,
    NotEquals,
}

impl BodyOp {
    /// The operator as the fixture field and the mock server's `op` parameter spell it.
    pub(crate) fn symbol(self) -> &'static str {
        match self {
            Self::Equals => "==",
            Self::NotEquals => "!=",
        }
    }

    /// The mock server's `op` query value.
    pub(crate) fn wire_name(self) -> &'static str {
        match self {
            Self::Equals => "equals",
            Self::NotEquals => "not_equals",
        }
    }
}

/// Separator between a bracketed key and the body path of a `body` field.
const BODY_SEGMENT: &str = "].body.";

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
    if let Some((head, body_tail)) = bracketed.split_once(BODY_SEGMENT) {
        let key = parse_quoted_key(head)?;
        return parse_body_predicate(key, body_tail);
    }
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

/// The quoted key inside a bracket whose closing `]` is already removed.
fn parse_quoted_key(inner: &str) -> Result<String, MockFieldSyntax> {
    let Some(key) = inner.strip_prefix('"').and_then(|rest| rest.strip_suffix('"')) else {
        return Err(MockFieldSyntax::MissingQuotedKey);
    };
    if key.is_empty() {
        return Err(MockFieldSyntax::EmptyKey);
    }
    Ok(key.to_string())
}

/// Parse `<path> == <json>` or `<path> != <json>`, the part of a predicate field after `.body.`.
///
/// ~keep A body path never holds whitespace, so the first space ends the path and the operator
/// that follows it is unambiguous even when the JSON text holds ` == ` itself.
fn parse_body_predicate(key: String, tail: &str) -> Result<MockQuery, MockFieldSyntax> {
    let Some((path, comparison)) = tail.split_once(' ') else {
        return Err(MockFieldSyntax::UnrecognizedTail);
    };
    if path.is_empty() {
        return Err(MockFieldSyntax::UnrecognizedTail);
    }
    let (op, value_json) = if let Some(value) = comparison.strip_prefix("== ") {
        (BodyOp::Equals, value)
    } else if let Some(value) = comparison.strip_prefix("!= ") {
        (BodyOp::NotEquals, value)
    } else {
        return Err(MockFieldSyntax::UnrecognizedTail);
    };
    if serde_json::from_str::<serde_json::Value>(value_json).is_err() {
        return Err(MockFieldSyntax::UnrecognizedTail);
    }
    Ok(MockQuery::BodyPredicate {
        key,
        path: path.to_string(),
        op,
        value_json: value_json.to_string(),
    })
}

/// The key and body path of a fixture-authored body field, `mock.requests["<key>"].body.<path>`,
/// or `None` for any other field (including the predicate form, which holds whitespace).
fn parse_body_reference(field: &str) -> Option<(String, String)> {
    let bracketed = field.strip_prefix(MOCK_REQUESTS_ROOT)?.strip_prefix('[')?;
    let (head, path) = bracketed.split_once(BODY_SEGMENT)?;
    if path.is_empty() || path.contains(char::is_whitespace) {
        return None;
    }
    Some((parse_quoted_key(head).ok()?, path.to_string()))
}

/// Rewrite a fixture-authored body assertion into the predicate form the generators render.
///
/// `mock.requests["POST /api/chat"].body.options.num_ctx` with `equals` and `8192` becomes the
/// field `mock.requests["POST /api/chat"].body.options.num_ctx == 8192` asserted `equals 1`: the
/// mock server evaluates the comparison, so every language reads one integer through the helper
/// it already has for request counts and no language parses JSON. Any other field is left as is.
///
/// Fails when the assertion type is neither `equals` nor `not_equals`, or has no `value`.
pub(crate) fn normalize_body_assertion(assertion: &mut crate::e2e::fixture::Assertion) -> Result<(), String> {
    let Some(field) = assertion.field.as_deref() else {
        return Ok(());
    };
    let Some((key, path)) = parse_body_reference(field) else {
        return Ok(());
    };
    let op = match assertion.assertion_type.as_str() {
        "equals" => BodyOp::Equals,
        "not_equals" => BodyOp::NotEquals,
        other => {
            return Err(format!(
                "assertion type '{other}' on '{field}' is not supported; a request body field takes 'equals' or 'not_equals'"
            ));
        }
    };
    let Some(value) = assertion.value.as_ref() else {
        return Err(format!("assertion on '{field}' needs a 'value'"));
    };
    let compact = value.to_string();
    if value.is_array() || value.is_object() || compact.contains(['\\', '$', '%', '{', '}']) {
        return Err(format!(
            "value {compact} on '{field}' is not supported; a request body field compares to a number, boolean, null or plain string"
        ));
    }
    assertion.field = Some(format!(
        "{MOCK_REQUESTS_ROOT}[\"{key}\"].body.{path} {} {compact}",
        op.symbol()
    ));
    assertion.assertion_type = "equals".to_string();
    assertion.value = Some(serde_json::Value::from(1_u64));
    Ok(())
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

    fn body_assertion(assertion_type: &str, field: &str, value: serde_json::Value) -> crate::e2e::fixture::Assertion {
        crate::e2e::fixture::Assertion {
            assertion_type: assertion_type.to_string(),
            field: Some(field.to_string()),
            value: Some(value),
            ..Default::default()
        }
    }

    #[test]
    fn body_predicate_parses_the_key_path_operator_and_value() {
        assert_eq!(
            parse_mock_field(r#"mock.requests["POST /api/chat"].body.options.num_ctx == 8192"#),
            Ok(MockQuery::BodyPredicate {
                key: "POST /api/chat".to_string(),
                path: "options.num_ctx".to_string(),
                op: BodyOp::Equals,
                value_json: "8192".to_string(),
            })
        );
        assert_eq!(
            parse_mock_field(r#"mock.requests["/api/chat"].body.model != "a == b""#),
            Ok(MockQuery::BodyPredicate {
                key: "/api/chat".to_string(),
                path: "model".to_string(),
                op: BodyOp::NotEquals,
                value_json: r#""a == b""#.to_string(),
            }),
            "the first space ends the path, so ` == ` inside the JSON value stays in the value"
        );
    }

    #[test]
    fn body_predicate_rejects_a_missing_operator_or_invalid_json() {
        for field in [
            r#"mock.requests["POST /a"].body.x"#,
            r#"mock.requests["POST /a"].body.x 8192"#,
            r#"mock.requests["POST /a"].body.x == nope"#,
            r#"mock.requests["POST /a"].body. == 1"#,
        ] {
            assert!(!is_mock_virtual_field(field), "{field} must not parse");
        }
    }

    #[test]
    fn normalize_rewrites_an_equals_body_assertion_into_the_predicate() {
        let mut assertion = body_assertion(
            "equals",
            r#"mock.requests["POST /api/chat"].body.options.num_ctx"#,
            serde_json::json!(8192),
        );
        normalize_body_assertion(&mut assertion).expect("equals normalizes");
        assert_eq!(
            assertion.field.as_deref(),
            Some(r#"mock.requests["POST /api/chat"].body.options.num_ctx == 8192"#)
        );
        assert_eq!(assertion.assertion_type, "equals");
        assert_eq!(assertion.value, Some(serde_json::json!(1)));
        assert!(is_mock_virtual_field(assertion.field.as_deref().unwrap()));
    }

    #[test]
    fn normalize_rewrites_not_equals_with_a_string_value() {
        let mut assertion = body_assertion(
            "not_equals",
            r#"mock.requests["/api/chat"].body.model"#,
            serde_json::json!("llama"),
        );
        normalize_body_assertion(&mut assertion).expect("not_equals normalizes");
        assert_eq!(
            assertion.field.as_deref(),
            Some(r#"mock.requests["/api/chat"].body.model != "llama""#)
        );
    }

    #[test]
    fn normalize_leaves_every_other_field_alone() {
        for field in ["mock.requests.total", r#"mock.requests["POST /a"]"#, "choices.0.text"] {
            let mut assertion = body_assertion("equals", field, serde_json::json!(1));
            normalize_body_assertion(&mut assertion).expect("not a body field");
            assert_eq!(assertion.field.as_deref(), Some(field));
            assert_eq!(assertion.value, Some(serde_json::json!(1)));
        }
    }

    #[test]
    fn normalize_refuses_an_assertion_type_a_body_field_cannot_take() {
        let mut assertion = body_assertion("contains", r#"mock.requests["POST /a"].body.x"#, serde_json::json!("v"));
        let error = normalize_body_assertion(&mut assertion).expect_err("contains is refused");
        assert!(error.contains("'contains'"), "{error}");
    }

    #[test]
    fn normalize_refuses_a_value_the_generators_cannot_embed_safely() {
        for value in [
            serde_json::json!({"a": 1}),
            serde_json::json!([1]),
            serde_json::json!("a\"b"),
            serde_json::json!("100%"),
            serde_json::json!("${x}"),
        ] {
            let mut assertion = body_assertion("equals", r#"mock.requests["POST /a"].body.x"#, value.clone());
            assert!(
                normalize_body_assertion(&mut assertion).is_err(),
                "{value} must be refused"
            );
        }
    }
}
