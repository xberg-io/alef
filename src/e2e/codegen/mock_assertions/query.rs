//! Builds the mock server's `path_and_query` string from a parsed `MockQuery` (alef issue #443).
//!
//! Pure data preparation, not generated code: the result is a plain URL string handed to a
//! backend's own assertion renderer as a Minijinja/format argument, never emitted target-language
//! structure itself -- see the `codegen-structure` rule ("Rust prepares data ... Jinja emits
//! structure").

use super::helper::{MOCK_BODY_PATH, MOCK_ONE_PATH, MOCK_TOTAL_PATH};
use super::model::MockQuery;
use crate::e2e::codegen::percent_encode_query;

/// Build the mock server's `path_and_query` string for `query`.
///
/// `MockQuery::Total` (`mock.requests.total`) means "every request the mock server has recorded"
/// (see `src/e2e/schema/fixture.schema.json`), so it queries with an empty `prefix` -- the mock
/// server's own `total_requests_for_prefix` treats an empty prefix as matching every key.
/// `MockQuery::Count` percent-encodes its key (the `"<METHOD> <path>"` or bare `"<path>"` string)
/// as a URI query-component value, since the key can contain a literal space (`"POST
/// /v1/chat"`) that would otherwise corrupt the request line a helper builds from this string.
pub(crate) fn build_path_and_query(query: &MockQuery) -> String {
    match query {
        MockQuery::Total => format!("{MOCK_TOTAL_PATH}?prefix="),
        MockQuery::Count(key) => format!("{MOCK_ONE_PATH}?key={}", percent_encode_query(key)),
        MockQuery::BodyPredicate {
            key,
            path,
            op,
            value_json,
        } => format!(
            "{MOCK_BODY_PATH}?key={}&field={}&op={}&value={}",
            percent_encode_query(key),
            percent_encode_query(path),
            op.wire_name(),
            percent_encode_query(value_json)
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::e2e::codegen::mock_assertions::model::BodyOp;

    #[test]
    fn body_predicate_percent_encodes_every_parameter() {
        let query = MockQuery::BodyPredicate {
            key: "POST /api/chat".to_string(),
            path: "options.num_ctx".to_string(),
            op: BodyOp::Equals,
            value_json: "8192".to_string(),
        };
        assert_eq!(
            build_path_and_query(&query),
            "/__alef/requests/body?key=POST%20%2Fapi%2Fchat&field=options.num_ctx&op=equals&value=8192"
        );
    }

    #[test]
    fn body_predicate_keeps_a_json_string_value_inert_in_the_url() {
        let query = MockQuery::BodyPredicate {
            key: "/api/chat".to_string(),
            path: "model".to_string(),
            op: BodyOp::NotEquals,
            value_json: "\"a&b\"".to_string(),
        };
        assert_eq!(
            build_path_and_query(&query),
            "/__alef/requests/body?key=%2Fapi%2Fchat&field=model&op=not_equals&value=%22a%26b%22"
        );
    }

    #[test]
    fn total_queries_the_total_endpoint_with_an_empty_prefix() {
        assert_eq!(
            build_path_and_query(&MockQuery::Total),
            "/__alef/requests/total?prefix="
        );
    }

    #[test]
    fn count_percent_encodes_a_method_and_path_key() {
        assert_eq!(
            build_path_and_query(&MockQuery::Count("POST /v1/chat".to_string())),
            "/__alef/requests/one?key=POST%20%2Fv1%2Fchat"
        );
    }

    #[test]
    fn count_percent_encodes_a_bare_path_key() {
        assert_eq!(
            build_path_and_query(&MockQuery::Count("/v1/chat".to_string())),
            "/__alef/requests/one?key=%2Fv1%2Fchat"
        );
    }
}
